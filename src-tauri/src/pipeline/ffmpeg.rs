//! Audio conversion via FFmpeg to 16 kHz mono 16-bit PCM WAV for Whisper ASR,
//! fast metadata duration probing, sparse window extraction (WS5), streaming
//! bounded-RAM waveform peak extraction (WS6), and deterministic duration math.

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const WAV_HEADER_BYTES: u64 = 44;
const SAMPLE_RATE_16KHZ: u64 = 16_000;
const BYTES_PER_SECOND_16KHZ_MONO_S16LE: u64 = SAMPLE_RATE_16KHZ * 2;
const FFMPEG_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WAVEFORM_BUCKETS: usize = 16_384;

/// Approved audio/video file extensions accepted by `prepare_audio_waveform` and `probe_media_file`.
pub const APPROVED_MEDIA_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "m4a", "mp4", "mkv", "mov", "webm", "ogg", "flac", "aac", "wma", "opus", "avi",
];

/// Verifies `raw_path` resolves to an existing regular file whose canonical target has an
/// approved audio/video extension (blocking `.json`, `.ssh/id_rsa`, directories, and symlinks
/// targeting non-media files).
pub fn validate_approved_media_file(raw_path: &str) -> Result<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(anyhow!("Invalid media file path"));
    }

    let path = Path::new(trimmed);
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("Media file not found or inaccessible: {trimmed}"))?;
    let meta = fs::metadata(&canonical).with_context(|| {
        format!(
            "Failed to inspect media file metadata: {}",
            canonical.display()
        )
    })?;
    if !meta.is_file() {
        return Err(anyhow!(
            "Media path is not a regular file: {}",
            canonical.display()
        ));
    }

    let ext = canonical
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if !APPROVED_MEDIA_EXTENSIONS.contains(&ext.as_str()) {
        return Err(anyhow!(
            "Disallowed media file extension '.{ext}' for {}",
            canonical.display()
        ));
    }

    Ok(strip_windows_verbatim_prefix(canonical))
}

#[derive(Debug, Clone)]
pub struct ConvertedAudio {
    pub wav_path: PathBuf,
    pub duration_ms: u64,
}

/// Computes exact duration in milliseconds from a 16 kHz mono 16-bit PCM WAV file byte length.
pub fn compute_pcm16_mono_16khz_duration_ms(file_size: u64) -> u64 {
    (file_size.saturating_sub(WAV_HEADER_BYTES) * 1000) / BYTES_PER_SECOND_16KHZ_MONO_S16LE
}

fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(s) = path.to_str() {
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                if !stripped.starts_with("UNC\\") {
                    return PathBuf::from(stripped);
                }
            }
        }
    }
    path
}

/// Verifies `input_path` is a regular file and returns its canonicalized absolute path so FFmpeg
/// can never interpret a crafted filename as a CLI flag (`-...`) or protocol prefix (`concat:`, `http:`).
fn validate_and_canonicalize_media_input(input_path: &Path) -> Result<PathBuf> {
    if !input_path.is_file() {
        return Err(anyhow!(
            "Input media file does not exist or is not a regular file: {}",
            input_path.to_string_lossy()
        ));
    }
    let canonical = fs::canonicalize(input_path).with_context(|| {
        format!(
            "Failed to canonicalize input media path: {}",
            input_path.to_string_lossy()
        )
    })?;
    Ok(strip_windows_verbatim_prefix(canonical))
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

/// Probes `ffmpeg -nostdin -version` with a strict `5s` timeout, returning the first version line.
pub async fn probe_ffmpeg_version(ffmpeg_bin: &Path) -> Result<String> {
    let mut cmd = Command::new(ffmpeg_bin);
    configure_command_env(&mut cmd, ffmpeg_bin);
    cmd.kill_on_drop(true)
        .arg("-nostdin")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = tokio::time::timeout(FFMPEG_PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out probing ffmpeg -version at {} after {}s",
                ffmpeg_bin.display(),
                FFMPEG_PROBE_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| {
            format!(
                "Failed to execute ffmpeg -version at {}",
                ffmpeg_bin.display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "ffmpeg -version exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .next()
        .unwrap_or("ffmpeg")
        .trim()
        .to_string())
}

/// Pure parser for FFmpeg `Duration: HH:MM:SS.cc` metadata lines from stderr.
pub fn parse_ffmpeg_duration_ms(stderr: &str) -> Option<u64> {
    static DURATION_RE: OnceLock<Regex> = OnceLock::new();
    let re = DURATION_RE.get_or_init(|| {
        Regex::new(r"Duration:\s*(\d+):(\d{2}):(\d{2})(?:\.(\d+))?")
            .expect("valid ffmpeg duration regex")
    });

    let caps = re.captures(stderr)?;
    let hours: u64 = caps.get(1)?.as_str().parse().ok()?;
    let mins: u64 = caps.get(2)?.as_str().parse().ok()?;
    let secs: u64 = caps.get(3)?.as_str().parse().ok()?;
    let frac_ms: u64 = match caps.get(4) {
        Some(m) => {
            let frac_str = format!("0.{}", m.as_str());
            let frac = frac_str.parse::<f64>().unwrap_or(0.0);
            (frac * 1000.0).round() as u64
        }
        None => 0,
    };

    Some(
        hours
            .saturating_mul(3_600_000)
            .saturating_add(mins.saturating_mul(60_000))
            .saturating_add(secs.saturating_mul(1_000))
            .saturating_add(frac_ms),
    )
}

/// Fast header/metadata duration probe via `ffmpeg -nostdin -hide_banner -i <canon>` stderr
/// `Duration:` parsing with a strict `5s` timeout (zero full decode).
pub async fn probe_media_duration_ms(ffmpeg_bin: &Path, input_path: &Path) -> Result<u64> {
    let canonical_input = validate_and_canonicalize_media_input(input_path)?;

    let mut cmd = Command::new(ffmpeg_bin);
    configure_command_env(&mut cmd, ffmpeg_bin);
    cmd.kill_on_drop(true)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-i")
        .arg(&canonical_input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let output = tokio::time::timeout(FFMPEG_PROBE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out probing media duration for {} after {}s",
                canonical_input.display(),
                FFMPEG_PROBE_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| {
            format!(
                "Failed to spawn ffmpeg binary at {}",
                ffmpeg_bin.to_string_lossy()
            )
        })?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_ffmpeg_duration_ms(&stderr).ok_or_else(|| {
        anyhow!(
            "Could not determine media duration from FFmpeg metadata for {}",
            canonical_input.display()
        )
    })
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

    let canonical_input = validate_and_canonicalize_media_input(input_audio)?;

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
    cmd.kill_on_drop(true)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(&canonical_input)
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

/// Fast `-ss` sparse 16 kHz mono PCM16 WAV window extraction (for Workstream 5 intake probe).
/// Returns the extracted window duration in milliseconds.
pub async fn extract_pcm16_window_wav(
    ffmpeg_bin: &Path,
    input_path: &Path,
    output_wav: &Path,
    start_secs: f64,
    duration_secs: f64,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<u64> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }
    if !start_secs.is_finite() || start_secs < 0.0 {
        return Err(anyhow!("Invalid window start_secs: {start_secs}"));
    }
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        return Err(anyhow!("Invalid window duration_secs: {duration_secs}"));
    }

    let canonical_input = validate_and_canonicalize_media_input(input_path)?;

    if let Some(parent) = output_wav.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create temporary WAV window directory: {}",
                    parent.to_string_lossy()
                )
            })?;
        }
    }

    let mut cmd = Command::new(ffmpeg_bin);
    configure_command_env(&mut cmd, ffmpeg_bin);
    cmd.kill_on_drop(true)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(format!("{start_secs:.3}"))
        .arg("-t")
        .arg(format!("{duration_secs:.3}"))
        .arg("-i")
        .arg(&canonical_input)
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
            Ok(None) => tokio::time::sleep(Duration::from_millis(25)).await,
            Err(err) => {
                stderr_task.abort();
                let _ = fs::remove_file(output_wav);
                return Err(err).context("Failed polling ffmpeg window extraction process");
            }
        }
    };

    let stderr_buf = stderr_task.await.unwrap_or_default();

    if !status.success() {
        let _ = fs::remove_file(output_wav);
        return Err(anyhow!(
            "FFmpeg window extraction failed (status {}): {}",
            status,
            stderr_buf.trim()
        ));
    }

    let meta = fs::metadata(output_wav).with_context(|| {
        format!(
            "FFmpeg finished window extraction but output WAV is missing: {}",
            output_wav.to_string_lossy()
        )
    })?;
    Ok(compute_pcm16_mono_16khz_duration_ms(meta.len()))
}

#[derive(Clone, Copy, Default)]
struct PeakBin {
    peak: f32,
    sum_sq: f64,
    count: u64,
}

impl PeakBin {
    fn push_sample(&mut self, abs_norm: f32) {
        if abs_norm > self.peak {
            self.peak = abs_norm;
        }
        let v = f64::from(abs_norm);
        self.sum_sq += v * v;
        self.count += 1;
    }

    fn merge(self, other: Self) -> Self {
        Self {
            peak: self.peak.max(other.peak),
            sum_sq: self.sum_sq + other.sum_sq,
            count: self.count + other.count,
        }
    }

    fn blended_amplitude(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let rms = (self.sum_sq / (self.count as f64)).max(0.0).sqrt() as f32;
        0.55 * self.peak + 0.45 * rms
    }
}

/// Bounded-memory adaptive peak accumulator: keeps at most `4 * target_buckets` fine bins in RAM
/// regardless of audio length while preserving exact `peak` and `RMS` statistics across folds.
struct StreamingPeakAccumulator {
    target_buckets: usize,
    max_fine_bins: usize,
    samples_per_bin: u64,
    bins: Vec<PeakBin>,
    current: PeakBin,
    total_samples: u64,
}

impl StreamingPeakAccumulator {
    fn new(target_buckets: usize) -> Self {
        let clamped = target_buckets.clamp(1, MAX_WAVEFORM_BUCKETS);
        let max_fine_bins = clamped.saturating_mul(4).max(64);
        Self {
            target_buckets: clamped,
            max_fine_bins,
            samples_per_bin: 16, // start at 1 ms resolution (16 samples @ 16 kHz)
            bins: Vec::with_capacity(max_fine_bins),
            current: PeakBin::default(),
            total_samples: 0,
        }
    }

    fn push_i16(&mut self, sample: i16) {
        let abs_norm = (f32::from(sample).abs() / 32_768.0).clamp(0.0, 1.0);
        self.current.push_sample(abs_norm);
        self.total_samples = self.total_samples.saturating_add(1);

        if self.current.count >= self.samples_per_bin {
            self.bins.push(self.current);
            self.current = PeakBin::default();
            if self.bins.len() >= self.max_fine_bins {
                self.fold_pairs();
            }
        }
    }

    fn fold_pairs(&mut self) {
        let mut write_idx = 0usize;
        let len = self.bins.len();
        let mut read_idx = 0usize;
        while read_idx + 1 < len {
            self.bins[write_idx] = self.bins[read_idx].merge(self.bins[read_idx + 1]);
            write_idx += 1;
            read_idx += 2;
        }
        if read_idx < len {
            // Carry leftover odd bin into `current` so future samples merge seamlessly
            self.current = self.bins[read_idx].merge(self.current);
        }
        self.bins.truncate(write_idx);
        self.samples_per_bin = self.samples_per_bin.saturating_mul(2);
    }

    fn finish(mut self) -> (Vec<f32>, u64) {
        if self.current.count > 0 {
            self.bins.push(self.current);
        }
        let duration_ms = (self.total_samples.saturating_mul(1_000)) / SAMPLE_RATE_16KHZ;
        if self.bins.is_empty() {
            return (Vec::new(), duration_ms);
        }

        let out_count = self.target_buckets.min(self.bins.len());
        let mut raw = Vec::with_capacity(out_count);
        let fine_len = self.bins.len();

        for b in 0..out_count {
            let start = (b * fine_len) / out_count;
            let end = (((b + 1) * fine_len) / out_count).max(start + 1).min(fine_len);
            let mut merged = PeakBin::default();
            for bin in &self.bins[start..end] {
                merged = merged.merge(*bin);
            }
            raw.push(merged.blended_amplitude());
        }

        // 99th-percentile normalization matching WaveformTimeline visual contrast
        let mut sorted = raw.clone();
        sorted.sort_unstable_by(f32::total_cmp);
        let p99_idx = ((sorted.len() as f64) * 0.99).floor() as usize;
        let p99 = sorted[p99_idx.min(sorted.len() - 1)];
        let scale = if p99 > 0.005 { p99 } else { 1.0 };

        let peaks: Vec<f32> = raw
            .into_iter()
            .map(|val| {
                if val <= 0.0005 {
                    0.0
                } else {
                    (val / scale).clamp(0.0, 1.0)
                }
            })
            .collect();

        (peaks, duration_ms)
    }
}

/// Streams 16 kHz mono `s16le` PCM directly from `ffmpeg -nostdin -i <canon> -ac 1 -ar 16000 -f s16le pipe:1`
/// to compute normalized `0.0..=1.0` waveform peak buckets and exact `duration_ms` in bounded O(target_buckets) RAM.
pub async fn extract_waveform_peaks(
    ffmpeg_bin: &Path,
    input_path: &Path,
    target_buckets: usize,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(Vec<f32>, u64)> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    let canonical_input = validate_and_canonicalize_media_input(input_path)?;

    let mut cmd = Command::new(ffmpeg_bin);
    configure_command_env(&mut cmd, ffmpeg_bin);
    cmd.kill_on_drop(true)
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(&canonical_input)
        .arg("-vn")
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-f")
        .arg("s16le")
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn ffmpeg binary at {}",
            ffmpeg_bin.to_string_lossy()
        )
    })?;

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture ffmpeg stdout pipe for waveform peaks"))?;
    let stderr_pipe = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let mut acc = StreamingPeakAccumulator::new(target_buckets);
    let mut read_buf = vec![0u8; 32 * 1024];
    let mut carry_byte: Option<u8> = None;

    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stderr_task.abort();
            return Err(anyhow!("Job cancelled"));
        }

        let n = match tokio::time::timeout(Duration::from_millis(100), stdout_pipe.read(&mut read_buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            Ok(Err(err)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                stderr_task.abort();
                return Err(err).context("Failed reading PCM stream from ffmpeg");
            }
            Err(_) => continue, // Re-check cancel_flag while waiting on ffmpeg output
        };

        let chunk = &read_buf[..n];
        let mut idx = 0usize;
        if let Some(lo) = carry_byte.take() {
            let sample = i16::from_le_bytes([lo, chunk[0]]);
            acc.push_i16(sample);
            idx = 1;
        }
        while idx + 1 < chunk.len() {
            let sample = i16::from_le_bytes([chunk[idx], chunk[idx + 1]]);
            acc.push_i16(sample);
            idx += 2;
        }
        if idx < chunk.len() {
            carry_byte = Some(chunk[idx]);
        }
    }

    drop(stdout_pipe);
    let status = child
        .wait()
        .await
        .context("Failed waiting for ffmpeg waveform process")?;
    let stderr_buf = stderr_task.await.unwrap_or_default();

    if !status.success() {
        return Err(anyhow!(
            "FFmpeg waveform peak extraction failed (status {}): {}",
            status,
            stderr_buf.trim()
        ));
    }

    Ok(acc.finish())
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

    #[test]
    fn parses_ffmpeg_duration_lines_accurately() {
        let sample = "Input #0, mp3, from 'clip.mp3':\n  Duration: 00:01:20.64, start: 0.023021, bitrate: 193 kb/s\n";
        assert_eq!(parse_ffmpeg_duration_ms(sample), Some(80_640));
        assert_eq!(
            parse_ffmpeg_duration_ms("  Duration: 01:02:03.5, bitrate: 128 kb/s"),
            Some(3_723_500)
        );
        assert_eq!(
            parse_ffmpeg_duration_ms("  Duration: 00:00:02.125, start: 0.0"),
            Some(2_125)
        );
        assert_eq!(parse_ffmpeg_duration_ms("  Duration: N/A, start: 0.0"), None);
    }

    #[test]
    fn rejects_directory_or_missing_input_before_spawning() {
        let dir = std::env::temp_dir();
        assert!(validate_and_canonicalize_media_input(&dir).is_err());
        assert!(validate_and_canonicalize_media_input(Path::new("-flag-looking-file")).is_err());
    }

    fn write_synthetic_wav(path: &Path, samples: &[i16]) {
        let data_len = (samples.len() * 2) as u32;
        let riff_len = data_len + 36;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_len.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }

    #[tokio::test]
    async fn converts_probes_windows_and_extracts_waveform_peaks_with_system_ffmpeg() {
        let ffmpeg_path = Path::new("/usr/bin/ffmpeg");
        if !ffmpeg_path.exists() {
            return;
        }

        let version = probe_ffmpeg_version(ffmpeg_path).await.unwrap();
        assert!(
            version.to_ascii_lowercase().contains("ffmpeg"),
            "unexpected version: {version}"
        );

        let temp_dir =
            std::env::temp_dir().join(format!("easysrt_ffmpeg_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let src_audio = temp_dir.join("synth_input.wav");
        let out_wav = temp_dir.join("converted_16k.wav");

        // Build a 2.0s synthetic 16 kHz mono WAV: first 1.0s silent, second 1.0s 440 Hz tone
        let mut pcm = vec![0i16; 32_000];
        for (i, slot) in pcm[16_000..].iter_mut().enumerate() {
            let t = (i as f64) / 16_000.0;
            *slot = ((std::f64::consts::TAU * 440.0 * t).sin() * 22_000.0) as i16;
        }
        write_synthetic_wav(&src_audio, &pcm);

        // 1. Metadata duration probe (zero decode)
        let probed_ms = probe_media_duration_ms(ffmpeg_path, &src_audio)
            .await
            .unwrap();
        assert!(
            (1960..=2040).contains(&probed_ms),
            "expected ~2000ms probed duration, got {probed_ms}ms"
        );

        // 2. Full WAV conversion
        let cancel = Arc::new(AtomicBool::new(false));
        let converted = convert_to_16khz_mono_wav(ffmpeg_path, &src_audio, &out_wav, &cancel)
            .await
            .unwrap();
        assert!(converted.wav_path.exists());
        assert!(
            (1980..=2020).contains(&converted.duration_ms),
            "expected ~2000ms, got {}ms",
            converted.duration_ms
        );

        // 3. Sparse window extraction (0.5s window starting at 1.0s)
        let window_wav = temp_dir.join("window.wav");
        let win_ms = extract_pcm16_window_wav(
            ffmpeg_path,
            &src_audio,
            &window_wav,
            1.0,
            0.5,
            &cancel,
        )
        .await
        .unwrap();
        assert!(window_wav.is_file());
        assert!(
            (460..=540).contains(&win_ms),
            "expected ~500ms window, got {win_ms}ms"
        );

        // 4. Streaming waveform peaks
        let (peaks, peaks_dur_ms) = extract_waveform_peaks(ffmpeg_path, &src_audio, 200, &cancel)
            .await
            .unwrap();
        assert_eq!(peaks.len(), 200);
        assert!(
            (1980..=2020).contains(&peaks_dur_ms),
            "expected ~2000ms peaks duration, got {peaks_dur_ms}ms"
        );
        assert!(peaks.iter().all(|&p| p.is_finite() && (0.0..=1.0).contains(&p)));

        let first_half_max = peaks[..90]
            .iter()
            .copied()
            .fold(0.0_f32, f32::max);
        let second_half_avg =
            peaks[110..].iter().copied().sum::<f32>() / (peaks[110..].len() as f32);
        assert!(
            first_half_max < 0.05,
            "Expected quiet first half < 0.05, got {first_half_max}"
        );
        assert!(
            second_half_avg > 0.75,
            "Expected loud second half > 0.75, got {second_half_avg}"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
