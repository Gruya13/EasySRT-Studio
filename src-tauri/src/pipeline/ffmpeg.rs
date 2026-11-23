//! Audio conversion via FFmpeg to 16 kHz mono 16-bit PCM WAV for Whisper ASR,
//! plus deterministic duration calculation from the resulting PCM WAV byte size.

use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const WAV_HEADER_BYTES: u64 = 44;
const BYTES_PER_SECOND_16KHZ_MONO_S16LE: u64 = 16_000 * 2;

#[derive(Debug, Clone)]
pub struct ConvertedAudio {
    pub wav_path: PathBuf,
    pub duration_ms: u64,
}

/// Computes exact duration in milliseconds from a 16 kHz mono 16-bit PCM WAV file byte length.
pub fn compute_pcm16_mono_16khz_duration_ms(file_size: u64) -> u64 {
    (file_size.saturating_sub(WAV_HEADER_BYTES) * 1000) / BYTES_PER_SECOND_16KHZ_MONO_S16LE
}

/// Builds the effective `(current_dir, PATH, LD_LIBRARY_PATH)` strings for a binary,
/// prioritizing its parent directory, grandparent directory, and bundled MSVC runtime DLLs on Windows.
pub fn build_binary_env_strings(
    binary_path: &Path,
) -> (Option<PathBuf>, Option<String>, Option<String>) {
    let Some(parent) = binary_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    else {
        return (None, None, None);
    };

    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut dirs: Vec<String> = vec![parent.to_string_lossy().into_owned()];

    if let Some(grandparent) = parent
        .parent()
        .filter(|p| !p.as_os_str().is_empty() && *p != parent)
    {
        dirs.push(grandparent.to_string_lossy().into_owned());
    }

    #[cfg(windows)]
    if let Some(vcruntime_dir) = crate::setup_manager::find_bundled_vcruntime_dir(None) {
        let vc_str = vcruntime_dir.to_string_lossy().into_owned();
        if !dirs.iter().any(|d| d == &vc_str) {
            dirs.push(vc_str);
        }
    }

    let prefix = dirs.join(sep);
    let path_val = match std::env::var("PATH") {
        Ok(existing) if !existing.is_empty() => format!("{prefix}{sep}{existing}"),
        _ => prefix.clone(),
    };

    #[cfg(target_os = "linux")]
    let ld_val = Some(match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) if !existing.is_empty() => format!("{prefix}:{existing}"),
        _ => prefix,
    });
    #[cfg(not(target_os = "linux"))]
    let ld_val = None;

    (Some(parent.to_path_buf()), Some(path_val), ld_val)
}

/// Configures environment variables (`PATH` and `LD_LIBRARY_PATH`) and Windows `CREATE_NO_WINDOW`
/// on a `tokio::process::Command` so bundled DLLs/shared libraries next to the binary load cleanly.
pub fn configure_command_env(cmd: &mut Command, binary_path: &Path) {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let (dir, path_val, ld_val) = build_binary_env_strings(binary_path);
    if let Some(parent) = dir {
        cmd.current_dir(parent);
    }
    if let Some(path) = path_val {
        cmd.env("PATH", path);
    }
    if let Some(ld) = ld_val {
        cmd.env("LD_LIBRARY_PATH", ld);
    }
}

/// Converts any supported input audio/video file (`MP3`, `WAV`, `M4A`, `FLAC`, `OGG`, etc.)
/// into a temporary 16 kHz mono 16-bit PCM WAV file (`pcm_s16le`) and computes `duration_ms`.
pub async fn convert_to_16khz_mono_wav(
    ffmpeg_bin: &Path,
    input_audio: &Path,
    output_wav: &Path,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<ConvertedAudio> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    if !input_audio.exists() {
        return Err(anyhow!(
            "Input audio file does not exist: {}",
            input_audio.to_string_lossy()
        ));
    }

    if let Some(parent) = output_wav.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create temporary WAV directory: {}",
                    parent.to_string_lossy()
                )
            })?;
        }
    }

    let mut cmd = Command::new(ffmpeg_bin);
    configure_command_env(&mut cmd, ffmpeg_bin);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input_audio)
        .arg("-vn")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg("-y")
        .arg(output_wav)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn ffmpeg binary at {}",
            ffmpeg_bin.to_string_lossy()
        )
    })?;

    let stderr_pipe = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let status = loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stderr_task.abort();
            let _ = fs::remove_file(output_wav);
            return Err(anyhow!("Job cancelled"));
        }

        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => tokio::time::sleep(Duration::from_millis(40)).await,
            Err(err) => {
                stderr_task.abort();
                return Err(err).context("Failed polling ffmpeg process");
            }
        }
    };

    let stderr_buf = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let _ = fs::remove_file(output_wav);
        return Err(anyhow!(
            "FFmpeg audio conversion failed (status {}): {}",
            status,
            stderr_buf.trim()
        ));
    }

    let meta = fs::metadata(output_wav).with_context(|| {
        format!(
            "FFmpeg finished but output WAV is missing: {}",
            output_wav.to_string_lossy()
        )
    })?;

    let duration_ms = compute_pcm16_mono_16khz_duration_ms(meta.len());

    Ok(ConvertedAudio {
        wav_path: output_wav.to_path_buf(),
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_exact_pcm16_duration_from_size() {
        // 1 second of 16 kHz mono 16-bit PCM = 32,000 bytes + 44-byte WAV header
        assert_eq!(compute_pcm16_mono_16khz_duration_ms(32_044), 1000);
        // 2.5 seconds = 80,000 bytes + 44-byte header
        assert_eq!(compute_pcm16_mono_16khz_duration_ms(80_044), 2500);
        assert_eq!(compute_pcm16_mono_16khz_duration_ms(44), 0);
    }

    #[tokio::test]
    async fn converts_synthetic_audio_with_system_ffmpeg() {
        let ffmpeg_path = Path::new("/usr/bin/ffmpeg");
        if !ffmpeg_path.exists() {
            return;
        }

        let temp_dir =
            std::env::temp_dir().join(format!("easysrt_ffmpeg_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let src_audio = temp_dir.join("synth_input.wav");
        let out_wav = temp_dir.join("converted_16k.wav");

        // Generate a 1.5s 44.1kHz stereo sine wave via /usr/bin/ffmpeg
        let gen_status = Command::new(ffmpeg_path)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100:duration=1.5",
                "-ac",
                "2",
                "-y",
            ])
            .arg(&src_audio)
            .status()
            .await
            .unwrap();
        assert!(gen_status.success());

        let cancel = Arc::new(AtomicBool::new(false));
        let converted = convert_to_16khz_mono_wav(ffmpeg_path, &src_audio, &out_wav, &cancel)
            .await
            .unwrap();

        assert!(converted.wav_path.exists());
        // 1.5s = 1500ms (allow tiny container header variance of +-20ms)
        assert!(
            (1480..=1520).contains(&converted.duration_ms),
            "expected ~1500ms, got {}ms",
            converted.duration_ms
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
