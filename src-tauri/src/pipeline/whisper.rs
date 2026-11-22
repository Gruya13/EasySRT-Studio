//! Whisper ASR (`whisper-cli`) runner and `-ojf` full-JSON parser with token-level
//! and segment-level word timestamp extraction.

use crate::models::TimedWord;
use crate::pipeline::ffmpeg::configure_command_env;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
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

#[derive(Debug, Clone)]
pub struct WhisperOutput {
    pub detected_language: Option<String>,
    pub words: Vec<TimedWord>,
}

#[derive(Debug, Deserialize)]
struct WhisperJsonRoot {
    #[serde(default)]
    result: Option<WhisperJsonResult>,
    #[serde(default)]
    transcription: Vec<WhisperJsonSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperJsonResult {
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhisperJsonSegment {
    #[serde(default)]
    offsets: WhisperJsonOffsets,
    #[serde(default)]
    text: String,
    #[serde(default)]
    tokens: Vec<WhisperJsonToken>,
}

#[derive(Debug, Default, Deserialize)]
struct WhisperJsonOffsets {
    #[serde(default)]
    from: i64,
    #[serde(default)]
    to: i64,
}

#[derive(Debug, Deserialize)]
struct WhisperJsonToken {
    #[serde(default)]
    text: String,
    #[serde(default)]
    offsets: WhisperJsonOffsets,
    #[serde(default = "default_prob")]
    p: f64,
    #[serde(default = "default_dtw")]
    t_dtw: i64,
}

fn default_prob() -> f64 {
    1.0
}

fn default_dtw() -> i64 {
    -1
}

/// Formats the user's glossary terms into a concise `--prompt` string for Whisper.
pub fn build_whisper_prompt(glossary: &BTreeMap<String, String>) -> Option<String> {
    if glossary.is_empty() {
        return None;
    }

    let mut terms = Vec::new();
    for (k, v) in glossary {
        let kt = k.trim();
        let vt = v.trim();
        if !kt.is_empty() && !terms.iter().any(|t: &String| t.eq_ignore_ascii_case(kt)) {
            terms.push(kt.to_string());
        }
        if !vt.is_empty() && !terms.iter().any(|t: &String| t.eq_ignore_ascii_case(vt)) {
            terms.push(vt.to_string());
        }
    }

    if terms.is_empty() {
        None
    } else {
        Some(format!("Glossary: {}.", terms.join(", ")))
    }
}

fn is_special_whisper_token(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    (trimmed.starts_with("[_") && trimmed.ends_with(']'))
        || (trimmed.starts_with("<|") && trimmed.ends_with("|>"))
}

fn split_text_proportionally(
    text: &str,
    from_ms: u64,
    to_ms: u64,
    prob: f64,
) -> Vec<TimedWord> {
    let mut out: Vec<TimedWord> = text
        .split_whitespace()
        .map(|part| TimedWord {
            start_ms: from_ms,
            end_ms: from_ms,
            text: part.to_string(),
            probability: prob,
        })
        .collect();
    distribute_range_proportionally(&mut out, from_ms, to_ms);
    out
}

fn distribute_range_proportionally(slice: &mut [TimedWord], range_start: u64, range_end: u64) {
    if slice.is_empty() {
        return;
    }

    let total_duration = range_end.saturating_sub(range_start);
    let total_chars: usize = slice.iter().map(|w| w.text.chars().count().max(1)).sum();
    let count = slice.len();
    let mut elapsed_chars = 0usize;

    for (idx, word) in slice.iter_mut().enumerate() {
        let word_chars = word.text.chars().count().max(1);
        let word_start = if total_chars == 0 {
            range_start
        } else {
            range_start
                + ((total_duration as u128 * elapsed_chars as u128) / total_chars as u128) as u64
        };
        elapsed_chars += word_chars;
        let word_end = if idx + 1 == count || total_chars == 0 {
            range_end.max(word_start)
        } else {
            (range_start
                + ((total_duration as u128 * elapsed_chars as u128) / total_chars as u128) as u64)
                .max(word_start)
        };
        word.start_ms = word_start;
        word.end_ms = word_end;
    }
}

/// Anchors token-level word timestamps onto `[seg_from, seg_to]` when misaligned (e.g.
/// silence-compressed VAD token offsets) and redistributes collapsed zero-duration tokens.
fn align_segment_words_to_offsets(seg_words: &mut [TimedWord], seg_from: u64, seg_to: u64) {
    if seg_words.is_empty() || seg_to <= seg_from {
        return;
    }

    let raw_min = seg_words.iter().map(|w| w.start_ms).min().unwrap_or(seg_from);
    let raw_max = seg_words.iter().map(|w| w.end_ms).max().unwrap_or(seg_to);
    let is_misaligned = raw_min < seg_from
        || raw_max > seg_to
        || raw_max + 300 < seg_to
        || raw_min > seg_from + 300
        || raw_max <= raw_min;

    if is_misaligned {
        if raw_max > raw_min {
            let raw_span = raw_max - raw_min;
            let seg_span = seg_to - seg_from;
            for word in seg_words.iter_mut() {
                let rel_s = word.start_ms.saturating_sub(raw_min);
                let rel_e = word.end_ms.max(word.start_ms).saturating_sub(raw_min);
                word.start_ms =
                    seg_from + ((rel_s as u128 * seg_span as u128) / raw_span as u128) as u64;
                word.end_ms =
                    seg_from + ((rel_e as u128 * seg_span as u128) / raw_span as u128) as u64;
            }
        } else {
            distribute_range_proportionally(seg_words, seg_from, seg_to);
        }
    }

    let mut running_ms = seg_from;
    for word in seg_words.iter_mut() {
        word.start_ms = word.start_ms.max(running_ms).min(seg_to);
        word.end_ms = word.end_ms.max(word.start_ms).min(seg_to);
        running_ms = word.end_ms;
    }

    let mut i = 0;
    while i < seg_words.len() {
        if seg_words[i].end_ms > seg_words[i].start_ms {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < seg_words.len() && seg_words[j].end_ms <= seg_words[j].start_ms {
            j += 1;
        }
        let run_start = seg_words[i].start_ms;
        let next_bound = if j < seg_words.len() {
            seg_words[j].start_ms.max(run_start)
        } else {
            seg_to.max(run_start)
        };
        if next_bound > run_start {
            distribute_range_proportionally(&mut seg_words[i..j], run_start, next_bound);
            i = j;
        } else if j < seg_words.len() && seg_words[j].end_ms > run_start {
            let bound_end = seg_words[j].end_ms;
            distribute_range_proportionally(&mut seg_words[i..=j], run_start, bound_end);
            i = j + 1;
        } else if i > 0 && seg_words[i - 1].start_ms < run_start {
            let prev_start = seg_words[i - 1].start_ms;
            let bound_end = next_bound.max(run_start);
            distribute_range_proportionally(&mut seg_words[(i - 1)..j], prev_start, bound_end);
            i = j;
        } else {
            i = j;
        }
    }

    // Smooth artificial intra-segment token pauses (> 160ms) so the segmenter
    // does not split mid-phrase on Whisper BPE timing jitter inside one VAD chunk.
    for k in 0..seg_words.len().saturating_sub(1) {
        let next_start = seg_words[k + 1].start_ms;
        if next_start > seg_words[k].end_ms + 160 {
            seg_words[k].end_ms = next_start.saturating_sub(80).max(seg_words[k].end_ms);
        }
    }
}

fn is_trailing_silence_hallucination(
    seg_idx: usize,
    seg_count: usize,
    seg_from: u64,
    seg_to: u64,
    seg_text: &str,
    seg_words: &[TimedWord],
) -> bool {
    if seg_count <= 1 || seg_idx + 1 != seg_count || seg_to.saturating_sub(seg_from) > 1500 {
        return false;
    }
    let normalized = seg_text.trim().to_ascii_lowercase();
    let matches_phrase = matches!(
        normalized.as_str(),
        "thank you." | "thank you" | "thanks for watching." | "thanks for watching!"
    );
    if !matches_phrase {
        return false;
    }
    let avg_prob = if seg_words.is_empty() {
        1.0
    } else {
        seg_words.iter().map(|w| w.probability).sum::<f64>() / seg_words.len() as f64
    };
    avg_prob < 0.78
}

/// Parses `whisper.cpp` `-ojf` full JSON output into `WhisperOutput` (`detected_language` + `Vec<TimedWord>`).
pub fn parse_whisper_ojf_json(json_str: &str) -> Result<WhisperOutput> {
    let root: WhisperJsonRoot =
        serde_json::from_str(json_str).context("Failed to parse whisper-cli -ojf JSON output")?;

    let detected_language = root
        .result
        .and_then(|r| r.language)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());

    let mut words = Vec::new();
    let seg_count = root.transcription.len();

    for (seg_idx, seg) in root.transcription.iter().enumerate() {
        let raw_seg_from = seg.offsets.from.max(0) as u64;
        let raw_seg_to = seg.offsets.to.max(seg.offsets.from).max(0) as u64;
        let prev_seg_to = seg_idx
            .checked_sub(1)
            .map(|idx| root.transcription[idx].offsets.to.max(0) as u64);
        let next_seg_from = root
            .transcription
            .get(seg_idx + 1)
            .map(|next| next.offsets.from.max(0) as u64);

        // Preserve a readable pause gap (>= 200ms) when adjacent Silero VAD segments
        // touch or nearly touch (e.g. seg[k].to == seg[k+1].from).
        let seg_from = if let Some(prev_to) = prev_seg_to {
            if prev_to + 220 >= raw_seg_from && raw_seg_to > raw_seg_from + 900 {
                (raw_seg_from + 90).min(raw_seg_to)
            } else {
                raw_seg_from
            }
        } else {
            raw_seg_from
        };

        let seg_to = if let Some(next_from) = next_seg_from {
            if next_from <= raw_seg_to + 220 && raw_seg_to > seg_from + 900 {
                raw_seg_to.saturating_sub(110).max(seg_from + 200)
            } else {
                raw_seg_to
            }
        } else {
            raw_seg_to
        };

        let mut seg_words: Vec<TimedWord> = Vec::new();
        let mut current_word: Option<(u64, u64, String, f64, usize)> = None;

        for tok in &seg.tokens {
            if is_special_whisper_token(&tok.text) {
                continue;
            }

            let tok_from = if tok.offsets.from > 0 || tok.offsets.to > 0 {
                tok.offsets.from.max(0) as u64
            } else if tok.t_dtw > 0 {
                tok.t_dtw as u64
            } else {
                seg_from
            };
            let tok_to = if tok.offsets.to >= tok.offsets.from && tok.offsets.to > 0 {
                tok.offsets.to as u64
            } else if tok.t_dtw > 0 {
                (tok.t_dtw as u64).max(tok_from)
            } else {
                tok_from
            };

            let starts_with_space = tok
                .text
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(false);
            let trimmed = tok.text.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Check if a single token contains multiple whitespace-separated words
            if trimmed.split_whitespace().count() > 1 {
                if let Some((s, e, txt, p_sum, p_cnt)) = current_word.take() {
                    if !txt.is_empty() {
                        seg_words.push(TimedWord {
                            start_ms: s,
                            end_ms: e.max(s),
                            text: txt,
                            probability: p_sum / (p_cnt.max(1) as f64),
                        });
                    }
                }
                seg_words.extend(split_text_proportionally(trimmed, tok_from, tok_to, tok.p));
                continue;
            }

            if starts_with_space {
                if let Some((s, e, txt, p_sum, p_cnt)) = current_word.take() {
                    if !txt.is_empty() {
                        seg_words.push(TimedWord {
                            start_ms: s,
                            end_ms: e.max(s),
                            text: txt,
                            probability: p_sum / (p_cnt.max(1) as f64),
                        });
                    }
                }
                current_word = Some((tok_from, tok_to, trimmed.to_string(), tok.p, 1));
            } else if let Some((_s, ref mut e, ref mut txt, ref mut p_sum, ref mut p_cnt)) =
                current_word
            {
                txt.push_str(trimmed);
                *e = (*e).max(tok_to);
                *p_sum += tok.p;
                *p_cnt += 1;
            } else {
                current_word = Some((tok_from, tok_to, trimmed.to_string(), tok.p, 1));
            }
        }

        if let Some((s, e, txt, p_sum, p_cnt)) = current_word.take() {
            if !txt.is_empty() {
                seg_words.push(TimedWord {
                    start_ms: s,
                    end_ms: e.max(s),
                    text: txt,
                    probability: p_sum / (p_cnt.max(1) as f64),
                });
            }
        }

        if seg_words.is_empty() && !seg.text.trim().is_empty() {
            seg_words = split_text_proportionally(seg.text.trim(), seg_from, seg_to, 1.0);
        }

        align_segment_words_to_offsets(&mut seg_words, seg_from, seg_to);
        if is_trailing_silence_hallucination(
            seg_idx,
            seg_count,
            seg_from,
            seg_to,
            &seg.text,
            &seg_words,
        ) {
            continue;
        }

        words.extend(seg_words);
    }

    Ok(WhisperOutput {
        detected_language,
        words,
    })
}

#[derive(Debug)]
enum WhisperProbeResult {
    Runnable,
    MissingSharedLibrary(String),
    NonFatalProbeExit(String),
    Unexecutable(String),
}

fn is_missing_shared_library_error(code: Option<i32>, stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("error while loading shared libraries")
        || lower.contains("cannot open shared object file")
        || lower.contains("library not loaded")
        || lower.contains("the specified module could not be found")
        || lower.contains("was not found")
        || matches!(code, Some(127) | Some(-1073741515) | Some(-1073741511))
}

async fn probe_whisper_binary_detailed(whisper_bin: &Path) -> WhisperProbeResult {
    if !whisper_bin.is_file() {
        return WhisperProbeResult::Unexecutable(format!(
            "Whisper binary not found at {}",
            whisper_bin.display()
        ));
    }

    let mut cmd = Command::new(whisper_bin);
    configure_command_env(&mut cmd, whisper_bin);
    cmd.arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match tokio::time::timeout(Duration::from_secs(5), cmd.output()).await {
        Err(_) => WhisperProbeResult::Unexecutable(format!(
            "Timed out probing {} --help",
            whisper_bin.display()
        )),
        Ok(Err(err)) => WhisperProbeResult::Unexecutable(format!(
            "Failed to execute {} --help: {err}",
            whisper_bin.display()
        )),
        Ok(Ok(output)) => {
            if output.status.success() {
                return WhisperProbeResult::Runnable;
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("exited with status {}", output.status)
            };

            if is_missing_shared_library_error(output.status.code(), &detail) {
                WhisperProbeResult::MissingSharedLibrary(detail)
            } else {
                WhisperProbeResult::NonFatalProbeExit(detail)
            }
        }
    }
}

/// Lightweight pre-flight check verifying `whisper-cli --help` runs cleanly with
/// `configure_command_env` (detecting missing shared libraries such as `libvulkan.so.1`
/// or CUDA DLLs before running transcription).
pub async fn probe_whisper_binary_runnable(whisper_bin: &Path) -> bool {
    matches!(
        probe_whisper_binary_detailed(whisper_bin).await,
        WhisperProbeResult::Runnable
    )
}

async fn find_fallback_whisper_binary(failed_bin: &Path) -> Option<PathBuf> {
    let failed_canon = fs::canonicalize(failed_bin).ok();
    let mut candidates = Vec::new();

    if let Some(parent) = failed_bin.parent() {
        let mut search_roots = vec![parent.to_path_buf()];
        if let Some(grandparent) = parent.parent() {
            search_roots.push(grandparent.to_path_buf());
        }
        for root in search_roots {
            candidates.push(root.join("whisper-bin-ubuntu-x64").join("whisper-cli"));
            candidates.push(root.join("whisper-cpu").join("whisper-cli"));
            candidates.push(root.join("whisper-cli"));
            candidates.push(root.join("Release").join("whisper-cli.exe"));
            candidates.push(root.join("cpu").join("whisper-cli.exe"));
        }
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        let names: &[&str] = if cfg!(windows) {
            &["whisper-cli.exe", "whisper-cpp.exe"]
        } else {
            &["whisper-cli", "whisper-cpp"]
        };
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                candidates.push(dir.join(name));
            }
        }
    }

    for cand in candidates {
        if !cand.is_file() {
            continue;
        }
        if let Some(ref fc) = failed_canon {
            if fs::canonicalize(&cand).ok().as_ref() == Some(fc) {
                continue;
            }
        } else if cand == failed_bin {
            continue;
        }
        if probe_whisper_binary_runnable(&cand).await {
            return Some(cand);
        }
    }

    None
}

enum WhisperExecError {
    MissingSharedLibrary(String),
    Other(anyhow::Error),
}

#[allow(clippy::too_many_arguments)]
async fn execute_whisper_once<F>(
    whisper_bin: &Path,
    whisper_model_path: &Path,
    vad_model_path: Option<&Path>,
    wav_path: &Path,
    source_language: &str,
    glossary: &BTreeMap<String, String>,
    cancel_flag: &Arc<AtomicBool>,
    on_progress: &mut F,
) -> std::result::Result<WhisperOutput, WhisperExecError>
where
    F: FnMut(f64, String),
{
    let output_prefix = wav_path.with_extension("whisper_out");
    let expected_json_path =
        Path::new(&format!("{}.json", output_prefix.to_string_lossy())).to_path_buf();
    let _ = fs::remove_file(&expected_json_path);

    let lang = {
        let trimmed = source_language.trim();
        if trimmed.is_empty() {
            "auto"
        } else {
            trimmed
        }
    };

    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(4, 16))
        .unwrap_or(4);

    let mut cmd = Command::new(whisper_bin);
    configure_command_env(&mut cmd, whisper_bin);

    cmd.arg("-m")
        .arg(whisper_model_path)
        .arg("-f")
        .arg(wav_path)
        .arg("-l")
        .arg(lang)
        .arg("-t")
        .arg(threads.to_string())
        .arg("-bs")
        .arg("1")
        .arg("-bo")
        .arg("1")
        .arg("-fa")
        .arg("-ojf")
        .arg("-of")
        .arg(&output_prefix);

    if let Some(vp) = vad_model_path {
        if vp.exists() {
            cmd.arg("--vad").arg("-vm").arg(vp);
        }
    }

    if let Some(prompt) = build_whisper_prompt(glossary) {
        cmd.arg("--prompt").arg(prompt);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    on_progress(15.0, "Starting Whisper ASR transcription...".to_string());

    let mut child = cmd.spawn().map_err(|err| {
        WhisperExecError::Other(anyhow!(
            "Failed to spawn whisper-cli at {}: {err}",
            whisper_bin.to_string_lossy()
        ))
    })?;

    let mut stderr_pipe = child.stderr.take();
    let mut tick_count: u64 = 0;

    let status = loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = fs::remove_file(&expected_json_path);
            return Err(WhisperExecError::Other(anyhow!("Job cancelled")));
        }

        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                tick_count += 1;
                if tick_count.is_multiple_of(20) {
                    let step = ((tick_count / 20) as f64 * 2.5).min(70.0);
                    on_progress(15.0 + step, "Transcribing audio with Whisper...".to_string());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => {
                return Err(WhisperExecError::Other(anyhow!(
                    "Failed polling whisper-cli process: {err}"
                )));
            }
        }
    };

    let mut stderr_buf = String::new();
    if let Some(mut pipe) = stderr_pipe.take() {
        let _ = pipe.read_to_string(&mut stderr_buf).await;
    }

    if !status.success() {
        let _ = fs::remove_file(&expected_json_path);
        let trimmed = stderr_buf.trim().to_string();
        if is_missing_shared_library_error(status.code(), &trimmed) {
            return Err(WhisperExecError::MissingSharedLibrary(trimmed));
        }
        return Err(WhisperExecError::Other(anyhow!(
            "whisper-cli failed (status {}): {}",
            status,
            trimmed
        )));
    }

    let raw_json = fs::read_to_string(&expected_json_path).map_err(|err| {
        WhisperExecError::Other(anyhow!(
            "whisper-cli exited cleanly but JSON output was not found at {}: {err}",
            expected_json_path.to_string_lossy()
        ))
    })?;
    let _ = fs::remove_file(&expected_json_path);

    on_progress(90.0, "Parsing Whisper word timestamps...".to_string());
    parse_whisper_ojf_json(&raw_json).map_err(WhisperExecError::Other)
}

/// Runs `whisper-cli` on a 16 kHz mono WAV file and parses the resulting `-ojf` JSON output.
/// Guarantees that the `whisper-cli` process exits before returning so GPU VRAM is 100% freed.
#[allow(clippy::too_many_arguments)]
pub async fn run_whisper_cli<F>(
    whisper_bin: &Path,
    whisper_model_path: &Path,
    vad_model_path: Option<&Path>,
    wav_path: &Path,
    source_language: &str,
    glossary: &BTreeMap<String, String>,
    cancel_flag: &Arc<AtomicBool>,
    mut on_progress: F,
) -> Result<WhisperOutput>
where
    F: FnMut(f64, String),
{
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    if !whisper_bin.exists() {
        return Err(anyhow!(
            "Whisper binary not found at {}",
            whisper_bin.to_string_lossy()
        ));
    }
    if !whisper_model_path.exists() {
        return Err(anyhow!(
            "Whisper model not found at {}",
            whisper_model_path.to_string_lossy()
        ));
    }
    if !wav_path.exists() {
        return Err(anyhow!(
            "Input WAV file not found at {}",
            wav_path.to_string_lossy()
        ));
    }

    let mut effective_bin = whisper_bin.to_path_buf();
    let mut used_fallback = false;

    match probe_whisper_binary_detailed(whisper_bin).await {
        WhisperProbeResult::Runnable => {}
        WhisperProbeResult::MissingSharedLibrary(diag)
        | WhisperProbeResult::Unexecutable(diag) => {
            if let Some(fallback) = find_fallback_whisper_binary(whisper_bin).await {
                tracing::warn!(
                    "Whisper binary at {} failed pre-flight probe ({diag}); falling back to {}",
                    whisper_bin.display(),
                    fallback.display()
                );
                on_progress(
                    15.0,
                    format!(
                        "GPU/custom whisper-cli missing runtime libraries; falling back to {}",
                        fallback.display()
                    ),
                );
                effective_bin = fallback;
                used_fallback = true;
            } else {
                return Err(anyhow!(
                    "Whisper binary at {} cannot run due to missing shared libraries or runtime error ({diag}). Install Vulkan/CUDA runtime libraries (e.g., libvulkan1 on Linux) or switch to the CPU whisper-cli build in Setup.",
                    whisper_bin.display()
                ));
            }
        }
        WhisperProbeResult::NonFatalProbeExit(diag) => {
            tracing::debug!(
                "whisper-cli --help pre-flight exited non-zero without shared-library error ({diag}); proceeding"
            );
        }
    }

    match execute_whisper_once(
        &effective_bin,
        whisper_model_path,
        vad_model_path,
        wav_path,
        source_language,
        glossary,
        cancel_flag,
        &mut on_progress,
    )
    .await
    {
        Ok(out) => Ok(out),
        Err(WhisperExecError::MissingSharedLibrary(diag)) => {
            if !used_fallback {
                if let Some(fallback) = find_fallback_whisper_binary(&effective_bin).await {
                    tracing::warn!(
                        "Whisper binary at {} failed with missing shared library ({diag}); retrying with fallback {}",
                        effective_bin.display(),
                        fallback.display()
                    );
                    on_progress(
                        15.0,
                        format!(
                            "GPU/custom whisper-cli failed ({diag}); falling back to {}",
                            fallback.display()
                        ),
                    );
                    return match execute_whisper_once(
                        &fallback,
                        whisper_model_path,
                        vad_model_path,
                        wav_path,
                        source_language,
                        glossary,
                        cancel_flag,
                        &mut on_progress,
                    )
                    .await
                    {
                        Ok(out) => Ok(out),
                        Err(WhisperExecError::MissingSharedLibrary(d)) => Err(anyhow!(
                            "Whisper fallback binary also failed due to missing shared libraries: {d}"
                        )),
                        Err(WhisperExecError::Other(err)) => Err(err),
                    };
                }
            }
            Err(anyhow!(
                "Whisper binary at {} failed due to missing shared libraries ({diag}). Install Vulkan/CUDA runtime libraries (e.g., libvulkan1 on Linux) or switch to the CPU whisper-cli build in Setup.",
                effective_bin.display()
            ))
        }
        Err(WhisperExecError::Other(err)) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_ojf_with_bpe_subword_tokens_and_filters_special_tokens() {
        let payload = r#"{
            "result": { "language": "sr" },
            "transcription": [
                {
                    "offsets": { "from": 0, "to": 2400 },
                    "text": " Zdravo svima, dobrodošli!",
                    "tokens": [
                        { "text": "[_BEG_]", "offsets": { "from": 0, "to": 0 }, "p": 0.99 },
                        { "text": " Zdravo", "offsets": { "from": 120, "to": 640 }, "p": 0.96 },
                        { "text": " svi", "offsets": { "from": 680, "to": 900 }, "p": 0.92 },
                        { "text": "ma", "offsets": { "from": 900, "to": 1120 }, "p": 0.94 },
                        { "text": ",", "offsets": { "from": 1120, "to": 1160 }, "p": 0.98 },
                        { "text": " dobro", "offsets": { "from": 1300, "to": 1700 }, "p": 0.91 },
                        { "text": "došli", "offsets": { "from": 1700, "to": 2250 }, "p": 0.95 },
                        { "text": "!", "offsets": { "from": 2250, "to": 2350 }, "p": 0.99 },
                        { "text": "[_TT_120]", "offsets": { "from": 2350, "to": 2400 }, "p": 0.99 }
                    ]
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json(payload).unwrap();
        assert_eq!(out.detected_language.as_deref(), Some("sr"));
        assert_eq!(out.words.len(), 3);

        assert_eq!(out.words[0].text, "Zdravo");
        assert_eq!(out.words[0].start_ms, 120);
        assert_eq!(out.words[0].end_ms, 640);

        assert_eq!(out.words[1].text, "svima,");
        assert_eq!(out.words[1].start_ms, 680);
        assert_eq!(out.words[1].end_ms, 1160);

        assert_eq!(out.words[2].text, "dobrodošli!");
        assert_eq!(out.words[2].start_ms, 1300);
        assert_eq!(out.words[2].end_ms, 2350);
    }

    #[test]
    fn falls_back_to_proportional_segment_split_when_tokens_empty() {
        let payload = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 1000, "to": 3000 },
                    "text": "Hello world today",
                    "tokens": []
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json(payload).unwrap();
        assert_eq!(out.detected_language.as_deref(), Some("en"));
        assert_eq!(out.words.len(), 3);
        assert_eq!(out.words[0].text, "Hello");
        assert_eq!(out.words[0].start_ms, 1000);
        assert_eq!(out.words[2].text, "today");
        assert_eq!(out.words[2].end_ms, 3000);
    }

    #[test]
    fn builds_glossary_prompt_cleanly() {
        let mut glossary = BTreeMap::new();
        glossary.insert("EasySRT".to_string(), "EasySRT Studio".to_string());
        glossary.insert("Beograd".to_string(), "Београд".to_string());
        let prompt = build_whisper_prompt(&glossary).unwrap();
        assert_eq!(prompt, "Glossary: Beograd, Београд, EasySRT, EasySRT Studio.");
    }

    #[test]
    fn remaps_vad_compressed_token_offsets_to_segment_timeline() {
        let payload = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 81890, "to": 88960 },
                    "text": " This is a decision that I make.",
                    "tokens": [
                        { "text": "[_BEG_]", "offsets": { "from": 53460, "to": 53460 }, "p": 0.99 },
                        { "text": " This", "offsets": { "from": 53460, "to": 54120 }, "p": 0.96 },
                        { "text": " is", "offsets": { "from": 54120, "to": 54600 }, "p": 0.97 },
                        { "text": " a", "offsets": { "from": 54600, "to": 54900 }, "p": 0.98 },
                        { "text": " decision", "offsets": { "from": 54900, "to": 56200 }, "p": 0.95 },
                        { "text": " that", "offsets": { "from": 56200, "to": 56800 }, "p": 0.94 },
                        { "text": " I", "offsets": { "from": 56800, "to": 57200 }, "p": 0.99 },
                        { "text": " make", "offsets": { "from": 57200, "to": 58100 }, "p": 0.98 },
                        { "text": ".", "offsets": { "from": 58100, "to": 58220 }, "p": 0.99 }
                    ]
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json(payload).unwrap();
        assert_eq!(out.words.len(), 7);
        assert_eq!(out.words.first().unwrap().start_ms, 81890);
        assert_eq!(out.words.last().unwrap().end_ms, 88960);
        for pair in out.words.windows(2) {
            assert!(pair[0].end_ms > pair[0].start_ms);
            assert!(pair[1].start_ms >= pair[0].end_ms);
        }
        assert!(out.words.last().unwrap().end_ms > out.words.last().unwrap().start_ms);
    }

    #[test]
    fn redistributes_zero_duration_tokens_within_segment() {
        let payload = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 1000, "to": 3000 },
                    "text": " This is a recording.",
                    "tokens": [
                        { "text": " This", "offsets": { "from": 1000, "to": 1000 }, "p": 0.95 },
                        { "text": " is", "offsets": { "from": 1000, "to": 1000 }, "p": 0.95 },
                        { "text": " a", "offsets": { "from": 1000, "to": 1000 }, "p": 0.95 },
                        { "text": " recording", "offsets": { "from": 1000, "to": 1900 }, "p": 0.95 },
                        { "text": ".", "offsets": { "from": 2000, "to": 3000 }, "p": 0.95 }
                    ]
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json(payload).unwrap();
        assert_eq!(out.words.len(), 4);
        assert_eq!(out.words.first().unwrap().start_ms, 1000);
        assert!(out.words.last().unwrap().end_ms <= 3000);
        for word in &out.words {
            assert!(
                word.end_ms > word.start_ms,
                "word '{}' kept zero duration: {}..{}",
                word.text,
                word.start_ms,
                word.end_ms
            );
        }
        for pair in out.words.windows(2) {
            assert!(pair[1].start_ms >= pair[0].end_ms);
        }
    }

    #[test]
    fn preserves_pause_gap_between_touching_vad_segments() {
        let payload = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 5410, "to": 9670 },
                    "text": " First segment words",
                    "tokens": [
                        { "text": " First", "offsets": { "from": 0, "to": 1400 }, "p": 0.96 },
                        { "text": " segment", "offsets": { "from": 1400, "to": 2800 }, "p": 0.96 },
                        { "text": " words", "offsets": { "from": 2800, "to": 4260 }, "p": 0.96 }
                    ]
                },
                {
                    "offsets": { "from": 9670, "to": 12730 },
                    "text": " Second segment.",
                    "tokens": [
                        { "text": " Second", "offsets": { "from": 4260, "to": 5700 }, "p": 0.97 },
                        { "text": " segment", "offsets": { "from": 5700, "to": 6980 }, "p": 0.97 },
                        { "text": ".", "offsets": { "from": 6980, "to": 7040 }, "p": 0.99 }
                    ]
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json(payload).unwrap();
        assert_eq!(out.words.len(), 5);
        assert_eq!(out.words[0].start_ms, 5410);
        assert_eq!(out.words[2].text, "words");
        assert_eq!(out.words[3].text, "Second");
        let boundary_gap = out.words[3].start_ms.saturating_sub(out.words[2].end_ms);
        assert!(
            boundary_gap >= 200,
            "expected >= 200ms gap between touching VAD segments, got {boundary_gap}"
        );
        assert_eq!(out.words[4].end_ms, 12730);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_whisper_binary_runnable_and_falls_back_on_missing_shared_libs() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_whisper_probe_test_{}",
            uuid::Uuid::new_v4()
        ));
        let gpu_dir = temp_dir.join("whisper-bin-ubuntu-vulkan-x64");
        let cpu_dir = temp_dir.join("whisper-bin-ubuntu-x64");
        fs::create_dir_all(&gpu_dir).unwrap();
        fs::create_dir_all(&cpu_dir).unwrap();

        let gpu_bin = gpu_dir.join("whisper-cli");
        let cpu_bin = cpu_dir.join("whisper-cli");
        let model = temp_dir.join("model.bin");
        let wav = temp_dir.join("audio.wav");
        fs::write(&model, b"model").unwrap();
        fs::write(&wav, b"RIFF0000WAVE").unwrap();

        // Missing-shared-library simulated GPU binary
        {
            let mut f = fs::File::create(&gpu_bin).unwrap();
            f.write_all(
                b"#!/usr/bin/env sh\necho 'error while loading shared libraries: libvulkan.so.1: cannot open shared object file: No such file or directory' >&2\nexit 127\n",
            )
            .unwrap();
            f.sync_all().unwrap();
        }
        fs::set_permissions(&gpu_bin, fs::Permissions::from_mode(0o755)).unwrap();

        // Runnable simulated CPU fallback binary
        {
            let mut f = fs::File::create(&cpu_bin).unwrap();
            f.write_all(
                br#"#!/usr/bin/env sh
if [ "$1" = "--help" ]; then
  echo "usage: whisper-cli"
  exit 0
fi
OUT_PREFIX=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -of) OUT_PREFIX="$2"; shift 2 ;;
    *) shift 1 ;;
  esac
done
cat << 'EOF' > "${OUT_PREFIX}.json"
{
  "result": { "language": "en" },
  "transcription": [
    { "offsets": { "from": 0, "to": 1000 }, "text": "Fallback ok.", "tokens": [] }
  ]
}
EOF
"#,
            )
            .unwrap();
            f.sync_all().unwrap();
        }
        fs::set_permissions(&cpu_bin, fs::Permissions::from_mode(0o755)).unwrap();

        tokio::time::sleep(Duration::from_millis(15)).await;

        assert!(!probe_whisper_binary_runnable(&gpu_bin).await);
        assert!(probe_whisper_binary_runnable(&cpu_bin).await);
        assert!(
            !probe_whisper_binary_runnable(&temp_dir.join("nonexistent-whisper")).await
        );

        let cancel = Arc::new(AtomicBool::new(false));
        let mut progress_msgs = Vec::new();
        let out = run_whisper_cli(
            &gpu_bin,
            &model,
            None,
            &wav,
            "en",
            &BTreeMap::new(),
            &cancel,
            |_, detail| progress_msgs.push(detail),
        )
        .await
        .unwrap();

        assert_eq!(out.words.len(), 2);
        assert_eq!(out.words[0].text, "Fallback");
        assert!(progress_msgs
            .iter()
            .any(|m| m.contains("falling back to")));

        // Without any sibling fallback, diagnostic error must explain missing shared library
        fs::remove_file(&cpu_bin).unwrap();
        let err = run_whisper_cli(
            &gpu_bin,
            &model,
            None,
            &wav,
            "en",
            &BTreeMap::new(),
            &cancel,
            |_, _| {},
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("missing shared libraries"));
        assert!(err.contains("libvulkan.so.1"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
