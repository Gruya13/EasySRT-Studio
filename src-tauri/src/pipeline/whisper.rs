//! Whisper ASR (`whisper-cli`) runner and `-ojf` full-JSON parser with token-level
//! and segment-level word timestamp extraction.

use crate::models::TimedWord;
use crate::pipeline::ffmpeg::configure_command_env;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
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
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.is_empty() {
        return Vec::new();
    }

    let total_duration = to_ms.saturating_sub(from_ms);
    let total_chars: usize = parts.iter().map(|p| p.chars().count().max(1)).sum();
    let mut out = Vec::with_capacity(parts.len());
    let mut elapsed_chars = 0usize;

    for (idx, part) in parts.iter().enumerate() {
        let part_chars = part.chars().count().max(1);
        let word_start = if total_chars == 0 {
            from_ms
        } else {
            from_ms + ((total_duration as u128 * elapsed_chars as u128) / total_chars as u128) as u64
        };
        elapsed_chars += part_chars;
        let word_end = if idx + 1 == parts.len() || total_chars == 0 {
            to_ms.max(word_start)
        } else {
            (from_ms
                + ((total_duration as u128 * elapsed_chars as u128) / total_chars as u128) as u64)
                .max(word_start)
        };

        out.push(TimedWord {
            start_ms: word_start,
            end_ms: word_end,
            text: (*part).to_string(),
            probability: prob,
        });
    }

    out
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

    for seg in &root.transcription {
        let seg_from = seg.offsets.from.max(0) as u64;
        let seg_to = seg.offsets.to.max(seg.offsets.from).max(0) as u64;

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

        words.extend(seg_words);
    }

    Ok(WhisperOutput {
        detected_language,
        words,
    })
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

    let output_prefix = wav_path.with_extension("whisper_out");
    let expected_json_path = Path::new(&format!("{}.json", output_prefix.to_string_lossy())).to_path_buf();
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
        .arg("-nf")
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

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to spawn whisper-cli at {}",
            whisper_bin.to_string_lossy()
        )
    })?;

    let mut stderr_pipe = child.stderr.take();
    let mut tick_count: u64 = 0;

    let status = loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = fs::remove_file(&expected_json_path);
            return Err(anyhow!("Job cancelled"));
        }

        match child.try_wait().context("Failed polling whisper-cli process")? {
            Some(status) => break status,
            None => {
                tick_count += 1;
                if tick_count % 20 == 0 {
                    let step = ((tick_count / 20) as f64 * 2.5).min(70.0);
                    on_progress(15.0 + step, "Transcribing audio with Whisper...".to_string());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };

    let mut stderr_buf = String::new();
    if let Some(mut pipe) = stderr_pipe.take() {
        let _ = pipe.read_to_string(&mut stderr_buf).await;
    }

    if !status.success() {
        let _ = fs::remove_file(&expected_json_path);
        return Err(anyhow!(
            "whisper-cli failed (status {}): {}",
            status,
            stderr_buf.trim()
        ));
    }

    let raw_json = fs::read_to_string(&expected_json_path).with_context(|| {
        format!(
            "whisper-cli exited cleanly but JSON output was not found at {}",
            expected_json_path.to_string_lossy()
        )
    })?;
    let _ = fs::remove_file(&expected_json_path);

    on_progress(90.0, "Parsing Whisper word timestamps...".to_string());
    parse_whisper_ojf_json(&raw_json)
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
}
