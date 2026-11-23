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

/// Maps a GGML Whisper model filename to its corresponding `whisper-cli -dtw` architecture preset.
pub fn infer_dtw_model_preset(whisper_model_path: &Path) -> Option<&'static str> {
    let name = whisper_model_path
        .file_name()?
        .to_string_lossy()
        .to_ascii_lowercase();
    if name.contains("turbo") {
        Some("large.v3.turbo")
    } else if name.contains("large-v3") || name.contains("large.v3") {
        Some("large.v3")
    } else if name.contains("large-v2") || name.contains("large.v2") {
        Some("large.v2")
    } else if name.contains("large") {
        Some("large.v1")
    } else if name.contains("medium.en") || name.contains("medium-en") {
        Some("medium.en")
    } else if name.contains("medium") {
        Some("medium")
    } else if name.contains("small.en") || name.contains("small-en") {
        Some("small.en")
    } else if name.contains("small") {
        Some("small")
    } else if name.contains("base.en") || name.contains("base-en") {
        Some("base.en")
    } else if name.contains("base") {
        Some("base")
    } else if name.contains("tiny.en") || name.contains("tiny-en") {
        Some("tiny.en")
    } else if name.contains("tiny") {
        Some("tiny")
    } else {
        None
    }
}

/// Parses Silero VAD speech intervals `(start_ms, end_ms)` logged to `whisper-cli` stderr.
pub fn parse_vad_segments_from_stderr(stderr: &str) -> Vec<(u64, u64)> {
    let mut intervals = Vec::new();
    for line in stderr.lines() {
        if !line.contains("VAD segment") {
            continue;
        }
        let Some(start_pos) = line.find("start = ") else {
            continue;
        };
        let after_start = &line[start_pos + "start = ".len()..];
        let start_str = after_start
            .split(|c: char| c == ',' || c.is_whitespace())
            .next()
            .unwrap_or("");
        let Some(end_pos) = line.find("end = ") else {
            continue;
        };
        let after_end = &line[end_pos + "end = ".len()..];
        let end_str = after_end
            .split(|c: char| c.is_whitespace() || c == '(' || c == ',')
            .next()
            .unwrap_or("");
        if let (Ok(start_s), Ok(end_s)) = (start_str.parse::<f64>(), end_str.parse::<f64>()) {
            let start_ms = (start_s.max(0.0) * 1000.0).round() as u64;
            let end_ms = (end_s.max(0.0) * 1000.0).round() as u64;
            if end_ms > start_ms {
                intervals.push((start_ms, end_ms));
            }
        }
    }
    intervals.sort_by_key(|&(s, _)| s);
    intervals
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
}

struct VadTokenWord {
    start_vad: u64,
    end_vad: u64,
    text: String,
    probability: f64,
}

fn map_point_onto_vad_subs(
    t_raw: u64,
    seg_vad_min: u64,
    seg_vad_max: u64,
    subs: &[(u64, u64, u64, usize)],
) -> (u64, usize) {
    let total_speech: u64 = subs.iter().map(|s| s.2).sum();
    let span = seg_vad_max.saturating_sub(seg_vad_min).max(1);
    let clamped_rel_num = t_raw.saturating_sub(seg_vad_min).min(span);
    let target = (clamped_rel_num as u128 * total_speech as u128) / (span as u128);
    let mut acc: u128 = 0;

    for (idx, &(s, _e, d, _vi)) in subs.iter().enumerate() {
        let d_u128 = d as u128;
        if acc + d_u128 >= target || idx + 1 == subs.len() {
            let into_sub = target.saturating_sub(acc).min(d_u128) as u64;
            return (s.saturating_add(into_sub), idx);
        }
        acc += d_u128;
    }

    let last = subs.last().copied().unwrap_or((0, 0, 0, 0));
    (last.1, subs.len().saturating_sub(1))
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

/// Parses `whisper.cpp` `-ojf` full JSON output into `WhisperOutput`.
pub fn parse_whisper_ojf_json(json_str: &str) -> Result<WhisperOutput> {
    parse_whisper_ojf_json_with_vad(json_str, &[])
}

/// Parses `whisper.cpp` `-ojf` JSON using Silero VAD speech sub-intervals from stderr when
/// available so stripped silence gaps are preserved piecewise and DTW timestamps stay frame-accurate.
pub fn parse_whisper_ojf_json_with_vad(
    json_str: &str,
    vad_intervals: &[(u64, u64)],
) -> Result<WhisperOutput> {
    let root: WhisperJsonRoot =
        serde_json::from_str(json_str).context("Failed to parse whisper-cli -ojf JSON output")?;

    let detected_language = root
        .result
        .and_then(|r| r.language)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());

    let mut words: Vec<TimedWord> = Vec::new();
    let seg_count = root.transcription.len();
    let use_piecewise_vad = !vad_intervals.is_empty();
    let mut last_consumed_vad_idx: Option<usize> = None;

    for (seg_idx, seg) in root.transcription.iter().enumerate() {
        let raw_seg_from = seg.offsets.from.max(0) as u64;
        let raw_seg_to = seg.offsets.to.max(seg.offsets.from).max(0) as u64;

        if use_piecewise_vad {
            let mut subs: Vec<(u64, u64, u64, usize)> = Vec::new();
            for (vi, &(vs, ve)) in vad_intervals.iter().enumerate() {
                let s = raw_seg_from.max(vs);
                let e = raw_seg_to.min(ve);
                if e > s {
                    let has_later_vad = vad_intervals[vi + 1..]
                        .iter()
                        .any(|&(lvs, lve)| raw_seg_to.min(lve) > raw_seg_from.max(lvs));
                    if last_consumed_vad_idx == Some(vi)
                        && ve.saturating_sub(raw_seg_from) <= 600
                        && has_later_vad
                    {
                        if let Some(prev_word) = words.last_mut() {
                            prev_word.end_ms = prev_word.end_ms.max(ve);
                        }
                        continue;
                    }
                    subs.push((s, e, e - s, vi));
                }
            }
            if subs.is_empty() {
                subs.push((
                    raw_seg_from,
                    raw_seg_to,
                    raw_seg_to.saturating_sub(raw_seg_from).max(1),
                    last_consumed_vad_idx.unwrap_or(0),
                ));
            }
            if let Some(last_sub) = subs.last() {
                last_consumed_vad_idx = Some(last_sub.3);
            }

            let mut tok_off_min: Option<u64> = None;
            let mut tok_off_max: Option<u64> = None;
            let mut raw_words: Vec<VadTokenWord> = Vec::new();
            let mut current: Option<(u64, u64, String, f64, usize)> = None;

            for tok in &seg.tokens {
                if tok.offsets.from > 0 || tok.offsets.to > 0 {
                    let f = tok.offsets.from.max(0) as u64;
                    let t = tok.offsets.to.max(0) as u64;
                    tok_off_min = Some(tok_off_min.map_or(f, |m| m.min(f)));
                    tok_off_max = Some(tok_off_max.map_or(t, |m| m.max(t)));
                }
                if is_special_whisper_token(&tok.text) {
                    continue;
                }

                let (tok_from, tok_to) = if tok.t_dtw > 0 {
                    let dtw_ms = (tok.t_dtw as u64) * 10;
                    (dtw_ms, dtw_ms)
                } else {
                    let tf = tok.offsets.from.max(0) as u64;
                    let tt = tok.offsets.to.max(tok.offsets.from).max(0) as u64;
                    (tf, tt)
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

                if trimmed.split_whitespace().count() > 1 {
                    if let Some((s, e, txt, p_sum, p_cnt)) = current.take() {
                        if !txt.is_empty() {
                            raw_words.push(VadTokenWord {
                                start_vad: s,
                                end_vad: e.max(s),
                                text: txt,
                                probability: p_sum / (p_cnt.max(1) as f64),
                            });
                        }
                    }
                    let multi = split_text_proportionally(trimmed, tok_from, tok_to, tok.p);
                    for mw in multi {
                        raw_words.push(VadTokenWord {
                            start_vad: mw.start_ms,
                            end_vad: mw.end_ms,
                            text: mw.text,
                            probability: mw.probability,
                        });
                    }
                    continue;
                }

                if starts_with_space {
                    if let Some((s, e, txt, p_sum, p_cnt)) = current.take() {
                        if !txt.is_empty() {
                            raw_words.push(VadTokenWord {
                                start_vad: s,
                                end_vad: e.max(s),
                                text: txt,
                                probability: p_sum / (p_cnt.max(1) as f64),
                            });
                        }
                    }
                    current = Some((tok_from, tok_to, trimmed.to_string(), tok.p, 1));
                } else if let Some((_s, ref mut e, ref mut txt, ref mut p_sum, ref mut p_cnt)) =
                    current
                {
                    txt.push_str(trimmed);
                    *e = (*e).max(tok_to);
                    *p_sum += tok.p;
                    *p_cnt += 1;
                } else {
                    current = Some((tok_from, tok_to, trimmed.to_string(), tok.p, 1));
                }
            }

            if let Some((s, e, txt, p_sum, p_cnt)) = current.take() {
                if !txt.is_empty() {
                    raw_words.push(VadTokenWord {
                        start_vad: s,
                        end_vad: e.max(s),
                        text: txt,
                        probability: p_sum / (p_cnt.max(1) as f64),
                    });
                }
            }

            let mut seg_words: Vec<TimedWord> = Vec::new();
            if raw_words.is_empty() {
                if !seg.text.trim().is_empty() {
                    let sub_start = subs.first().map(|s| s.0).unwrap_or(raw_seg_from);
                    let sub_end = subs.last().map(|s| s.1).unwrap_or(raw_seg_to);
                    seg_words =
                        split_text_proportionally(seg.text.trim(), sub_start, sub_end, 1.0);
                }
            } else {
                let eff_vad_min =
                    tok_off_min.unwrap_or_else(|| raw_words[0].start_vad.saturating_sub(80));
                let eff_vad_max = tok_off_max
                    .unwrap_or(raw_words.last().map_or(eff_vad_min + 1, |w| w.end_vad))
                    .max(eff_vad_min + 1);

                let mut prev_end_vad = eff_vad_min;
                for (i, rw) in raw_words.iter().enumerate() {
                    let p_end = rw.end_vad.max(prev_end_vad);
                    let est_dur = ((rw.text.chars().count() as u64) * 45).clamp(90, 350);
                    let p_start = if i > 0 {
                        prev_end_vad
                    } else {
                        p_end.saturating_sub(est_dur)
                    }
                    .max(eff_vad_min);
                    prev_end_vad = p_end;

                    let (mut ms0, idx0) =
                        map_point_onto_vad_subs(p_start, eff_vad_min, eff_vad_max, &subs);
                    let (mut ms1, idx1) =
                        map_point_onto_vad_subs(p_end, eff_vad_min, eff_vad_max, &subs);
                    if idx1 > idx0 || i == 0 {
                        ms0 = subs[idx1].0;
                    }
                    if let Some(prev) = seg_words.last() {
                        ms0 = ms0.max(prev.end_ms);
                    }
                    if ms1 <= ms0 {
                        ms1 = subs[idx1].1.min(ms0.saturating_add(60)).max(ms0);
                    }
                    seg_words.push(TimedWord {
                        start_ms: ms0,
                        end_ms: ms1,
                        text: rw.text.clone(),
                        probability: rw.probability,
                    });
                }
            }

            if is_trailing_silence_hallucination(
                seg_idx,
                seg_count,
                raw_seg_from,
                raw_seg_to,
                &seg.text,
                &seg_words,
            ) {
                continue;
            }

            words.extend(seg_words);
            continue;
        }

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

struct WhisperProcessSpec<'a> {
    whisper_bin: &'a Path,
    whisper_model_path: &'a Path,
    vad_model_path: Option<&'a Path>,
    wav_path: &'a Path,
    output_prefix: &'a Path,
    lang: &'a str,
    glossary: &'a BTreeMap<String, String>,
    cancel_flag: &'a Arc<AtomicBool>,
    tick_base_progress: f64,
    emit_progress: bool,
}

/// Extracts `[start_ms, end_ms]` from a 16 kHz mono PCM16 WAV file into a standalone WAV slice.
pub fn extract_pcm16_wav_slice(
    src_wav: &Path,
    dst_wav: &Path,
    start_ms: u64,
    end_ms: u64,
) -> Result<()> {
    if end_ms <= start_ms {
        return Err(anyhow!(
            "Invalid WAV slice range: {start_ms} ms .. {end_ms} ms"
        ));
    }
    let bytes = fs::read(src_wav).with_context(|| {
        format!("Failed to read WAV file at {}", src_wav.to_string_lossy())
    })?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(anyhow!(
            "Invalid RIFF WAVE file at {}",
            src_wav.to_string_lossy()
        ));
    }

    let mut pos = 12usize;
    let mut data_slice: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start.saturating_add(chunk_size).min(bytes.len());
        if chunk_id == b"data" {
            data_slice = Some(&bytes[payload_start..payload_end]);
            break;
        }
        // RIFF chunks are padded to even byte offsets
        pos = payload_start.saturating_add(chunk_size + (chunk_size & 1));
    }

    let pcm = data_slice.ok_or_else(|| anyhow!("WAV file missing 'data' chunk"))?;
    const BYTES_PER_MS: usize = 32; // 16,000 Hz * 1 ch * 2 bytes / 1000
    let raw_start = (start_ms as usize).saturating_mul(BYTES_PER_MS);
    let raw_end = (end_ms as usize).saturating_mul(BYTES_PER_MS);
    let byte_start = (raw_start & !1).min(pcm.len());
    let byte_end = (raw_end & !1).min(pcm.len());
    if byte_end <= byte_start {
        return Err(anyhow!(
            "WAV slice {start_ms}..{end_ms} ms falls outside audio data ({} ms)",
            pcm.len() / BYTES_PER_MS
        ));
    }

    let payload = &pcm[byte_start..byte_end];
    let payload_len_u32 = u32::try_from(payload.len()).context("WAV slice exceeds 4 GB")?;
    let riff_len_u32 = payload_len_u32.saturating_add(36);

    let mut out = Vec::with_capacity(44 + payload.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len_u32.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format = 1
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
    out.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&payload_len_u32.to_le_bytes());
    out.extend_from_slice(payload);

    fs::write(dst_wav, out).with_context(|| {
        format!("Failed to write WAV slice at {}", dst_wav.to_string_lossy())
    })?;
    Ok(())
}

async fn run_whisper_process<F>(
    spec: WhisperProcessSpec<'_>,
    on_progress: &mut F,
) -> std::result::Result<(String, String), WhisperExecError>
where
    F: FnMut(f64, String),
{
    let expected_json_path =
        Path::new(&format!("{}.json", spec.output_prefix.to_string_lossy())).to_path_buf();
    let _ = fs::remove_file(&expected_json_path);

    let threads = std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(4, 16))
        .unwrap_or(4);

    let mut cmd = Command::new(spec.whisper_bin);
    configure_command_env(&mut cmd, spec.whisper_bin);

    cmd.arg("-m")
        .arg(spec.whisper_model_path)
        .arg("-f")
        .arg(spec.wav_path)
        .arg("-l")
        .arg(spec.lang)
        .arg("-t")
        .arg(threads.to_string())
        .arg("-bs")
        .arg("1")
        .arg("-bo")
        .arg("1");

    if let Some(dtw_preset) = infer_dtw_model_preset(spec.whisper_model_path) {
        cmd.arg("-nfa").arg("-dtw").arg(dtw_preset);
    } else {
        cmd.arg("-fa");
    }

    cmd.arg("-ojf").arg("-of").arg(spec.output_prefix);

    if let Some(vp) = spec.vad_model_path {
        if vp.exists() {
            cmd.arg("--vad").arg("-vm").arg(vp);
        }
    }

    if let Some(prompt) = build_whisper_prompt(spec.glossary) {
        cmd.arg("--prompt").arg(prompt);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if spec.emit_progress {
        on_progress(
            spec.tick_base_progress,
            "Starting Whisper ASR transcription...".to_string(),
        );
    }

    let mut child = cmd.spawn().map_err(|err| {
        WhisperExecError::Other(anyhow!(
            "Failed to spawn whisper-cli at {}: {err}",
            spec.whisper_bin.to_string_lossy()
        ))
    })?;

    let stderr_pipe = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        String::from_utf8_lossy(&buf).into_owned()
    });
    let mut tick_count: u64 = 0;

    let status = loop {
        if spec.cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            stderr_task.abort();
            let _ = fs::remove_file(&expected_json_path);
            return Err(WhisperExecError::Other(anyhow!("Job cancelled")));
        }

        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                tick_count += 1;
                if spec.emit_progress && tick_count.is_multiple_of(20) {
                    let step = ((tick_count / 20) as f64 * 2.5).min(70.0);
                    on_progress(
                        spec.tick_base_progress + step,
                        "Transcribing audio with Whisper...".to_string(),
                    );
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => {
                stderr_task.abort();
                return Err(WhisperExecError::Other(anyhow!(
                    "Failed polling whisper-cli process: {err}"
                )));
            }
        }
    };

    let stderr_buf = stderr_task.await.unwrap_or_default();

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

    Ok((raw_json, stderr_buf))
}

#[allow(clippy::too_many_arguments)]
async fn transcribe_wav_slice(
    whisper_bin: &Path,
    whisper_model_path: &Path,
    src_wav: &Path,
    slice_tag: &str,
    start_ms: u64,
    end_ms: u64,
    lang: &str,
    glossary: &BTreeMap<String, String>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<WhisperOutput> {
    let slice_wav = PathBuf::from(format!("{}.{slice_tag}.wav", src_wav.to_string_lossy()));
    let slice_out_prefix =
        PathBuf::from(format!("{}.{slice_tag}.out", src_wav.to_string_lossy()));
    let _ = fs::remove_file(&slice_wav);

    let res = (async {
        extract_pcm16_wav_slice(src_wav, &slice_wav, start_ms, end_ms)?;
        let mut noop = |_p: f64, _d: String| {};
        let (raw_json, _stderr) = run_whisper_process(
            WhisperProcessSpec {
                whisper_bin,
                whisper_model_path,
                vad_model_path: None,
                wav_path: &slice_wav,
                output_prefix: &slice_out_prefix,
                lang,
                glossary,
                cancel_flag,
                tick_base_progress: 88.0,
                emit_progress: false,
            },
            &mut noop,
        )
        .await
        .map_err(|e| match e {
            WhisperExecError::MissingSharedLibrary(d) => anyhow!(d),
            WhisperExecError::Other(err) => err,
        })?;

        let mut parsed = parse_whisper_ojf_json_with_vad(&raw_json, &[])?;
        for w in &mut parsed.words {
            w.start_ms = w.start_ms.saturating_add(start_ms);
            w.end_ms = w.end_ms.saturating_add(start_ms);
        }
        Ok(parsed)
    })
    .await;

    let _ = fs::remove_file(&slice_wav);
    let _ = fs::remove_file(format!("{}.json", slice_out_prefix.to_string_lossy()));
    res
}

fn word_slice_prob_stats(words: &[TimedWord]) -> (f64, f64) {
    if words.is_empty() {
        return (1.0, 0.0);
    }
    let avg = words.iter().map(|w| w.probability).sum::<f64>() / words.len() as f64;
    let low_cnt = words.iter().filter(|w| w.probability < 0.38).count();
    let low_ratio = low_cnt as f64 / words.len() as f64;
    (avg, low_ratio)
}

fn vad_speech_ms_in_range(vad_intervals: &[(u64, u64)], start_ms: u64, end_ms: u64) -> u64 {
    if end_ms <= start_ms {
        return 0;
    }
    vad_intervals
        .iter()
        .map(|&(vs, ve)| ve.min(end_ms).saturating_sub(vs.max(start_ms)))
        .sum()
}

fn find_opening_intro_boundary(words: &[TimedWord]) -> Option<(usize, u64, Option<u64>)> {
    if words.is_empty() {
        return None;
    }
    for (idx, w) in words.iter().enumerate() {
        if w.end_ms < 2_000 {
            continue;
        }
        if w.end_ms > 24_000 {
            break;
        }
        let next = words.get(idx + 1);
        match next {
            None => {
                // Entire transcript fits under 24s
                return Some((idx, w.end_ms, None));
            }
            Some(nw) => {
                let gap = nw.start_ms.saturating_sub(w.end_ms);
                if gap >= 1_500 {
                    return Some((idx, w.end_ms, Some(nw.start_ms)));
                }
                if gap >= 300 {
                    let (prefix_avg, prefix_low) = word_slice_prob_stats(&words[..=idx]);
                    if prefix_avg < 0.65 && prefix_low >= 0.22 {
                        let tail_end = (idx + 13).min(words.len());
                        let (tail_avg, _) = word_slice_prob_stats(&words[idx + 1..tail_end]);
                        if tail_avg >= 0.72 {
                            return Some((idx, w.end_ms, Some(nw.start_ms)));
                        }
                    }
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
async fn recover_bilingual_and_dropped_vad_windows<F>(
    whisper_bin: &Path,
    whisper_model_path: &Path,
    wav_path: &Path,
    vad_intervals: &[(u64, u64)],
    glossary: &BTreeMap<String, String>,
    cancel_flag: &Arc<AtomicBool>,
    output: &mut WhisperOutput,
    on_progress: &mut F,
) where
    F: FnMut(f64, String),
{
    if output.words.is_empty() || cancel_flag.load(Ordering::SeqCst) {
        return;
    }

    let primary_lang = output
        .detected_language
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let is_primary_balkan = matches!(primary_lang.as_str(), "sr" | "hr" | "bs");

    let mut retried_intro_until: Option<u64> = None;

    // 1. Check opening intro window (up to ~24s) for bilingual host intro mis-decoded under primary_lang
    if let Some((b_idx, intro_end_ms, next_word_start)) = find_opening_intro_boundary(&output.words)
    {
        let (old_avg, old_low) = word_slice_prob_stats(&output.words[..=b_idx]);
        let has_dropped_vad_after_intro = next_word_start
            .map(|ns| {
                ns.saturating_sub(intro_end_ms) >= 3_500
                    && vad_speech_ms_in_range(
                        vad_intervals,
                        intro_end_ms.saturating_add(200),
                        ns.saturating_sub(200),
                    ) >= 2_000
            })
            .unwrap_or(false);

        if (old_avg < 0.65 && old_low >= 0.22) || (has_dropped_vad_after_intro && old_avg < 0.72) {
            let mut slice_end_ms = vad_intervals
                .iter()
                .filter(|&&(vs, ve)| {
                    vs <= intro_end_ms + 300 && ve >= intro_end_ms.saturating_sub(300)
                })
                .map(|&(_vs, ve)| ve.saturating_add(120))
                .max()
                .unwrap_or_else(|| intro_end_ms.saturating_add(200));
            if let Some(ns) = next_word_start {
                slice_end_ms = slice_end_ms.min(ns.saturating_sub(50));
            }
            slice_end_ms = slice_end_ms.min(29_500);

            if slice_end_ms >= 2_000 {
                let retry_lang = if is_primary_balkan { "en" } else { "sr" };
                on_progress(
                    88.0,
                    "Checking opening speech segment language...".to_string(),
                );
                if let Ok(intro_out) = transcribe_wav_slice(
                    whisper_bin,
                    whisper_model_path,
                    wav_path,
                    "intro_retry",
                    0,
                    slice_end_ms,
                    retry_lang,
                    glossary,
                    cancel_flag,
                )
                .await
                {
                    if !intro_out.words.is_empty() {
                        let (new_avg, _) = word_slice_prob_stats(&intro_out.words);
                        if new_avg > old_avg + 0.08
                            || (has_dropped_vad_after_intro && new_avg >= old_avg)
                        {
                            let mut kept = Vec::with_capacity(
                                intro_out.words.len() + output.words.len(),
                            );
                            kept.extend(intro_out.words);
                            for w in output.words.drain(..) {
                                if w.start_ms >= slice_end_ms {
                                    kept.push(w);
                                }
                            }
                            output.words = kept;
                            retried_intro_until = Some(slice_end_ms);
                        }
                    }
                }
            }
        }
    }

    // 2. Recover any large silent gaps in `output.words` where Silero VAD reported >= 2.2s of speech
    if vad_intervals.is_empty() || cancel_flag.load(Ordering::SeqCst) {
        return;
    }

    let mut gap_candidates: Vec<(u64, u64)> = Vec::new();
    for pair in output.words.windows(2) {
        let mut g_start = pair[0].end_ms;
        if let Some(intro_limit) = retried_intro_until {
            if g_start < intro_limit && pair[1].start_ms > intro_limit {
                g_start = intro_limit;
            }
        }
        let g_end = pair[1].start_ms;
        if g_end.saturating_sub(g_start) >= 3_500
            && vad_speech_ms_in_range(
                vad_intervals,
                g_start.saturating_add(200),
                g_end.saturating_sub(200),
            ) >= 2_200
        {
            gap_candidates.push((g_start, g_end));
            if gap_candidates.len() >= 3 {
                break;
            }
        }
    }

    if gap_candidates.is_empty() {
        return;
    }

    let mut recovered_all: Vec<TimedWord> = Vec::new();
    for (idx, (g_start, g_end)) in gap_candidates.into_iter().enumerate() {
        if cancel_flag.load(Ordering::SeqCst) {
            break;
        }
        let slice_start = vad_intervals
            .iter()
            .filter(|&&(vs, ve)| ve > g_start.saturating_add(200) && vs < g_end.saturating_sub(200))
            .map(|&(vs, _)| vs)
            .min()
            .map(|vs| vs.max(g_start.saturating_add(40)))
            .unwrap_or_else(|| g_start.saturating_add(80));
        let slice_end = g_end.saturating_sub(40);
        if slice_end <= slice_start.saturating_add(1_000) {
            continue;
        }

        // Process in <= 28s chunks if the dropped gap happens to be longer
        let mut cursor = slice_start;
        let mut sub_idx = 0usize;
        while cursor.saturating_add(1_000) < slice_end && sub_idx < 3 {
            let chunk_end = (cursor + 28_000).min(slice_end);
            let tag = format!("gap_{idx}_{sub_idx}");
            on_progress(
                89.0,
                "Recovering speech gap missed across language switch...".to_string(),
            );
            if let Ok(gap_out) = transcribe_wav_slice(
                whisper_bin,
                whisper_model_path,
                wav_path,
                &tag,
                cursor,
                chunk_end,
                "auto",
                glossary,
                cancel_flag,
            )
            .await
            {
                for mut w in gap_out.words {
                    if w.start_ms >= g_start && w.start_ms < slice_end {
                        if w.end_ms > slice_end {
                            w.end_ms = slice_end;
                        }
                        recovered_all.push(w);
                    }
                }
            }
            cursor = chunk_end;
            sub_idx += 1;
        }
    }

    if !recovered_all.is_empty() {
        output.words.extend(recovered_all);
        output.words.sort_by_key(|w| (w.start_ms, w.end_ms));
    }
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
    let lang = {
        let trimmed = source_language.trim();
        if trimmed.is_empty() {
            "auto"
        } else {
            trimmed
        }
    };

    let (raw_json, stderr_buf) = run_whisper_process(
        WhisperProcessSpec {
            whisper_bin,
            whisper_model_path,
            vad_model_path,
            wav_path,
            output_prefix: &output_prefix,
            lang,
            glossary,
            cancel_flag,
            tick_base_progress: 15.0,
            emit_progress: true,
        },
        on_progress,
    )
    .await?;

    on_progress(90.0, "Parsing Whisper word timestamps...".to_string());
    let vad_intervals = parse_vad_segments_from_stderr(&stderr_buf);
    let mut output = parse_whisper_ojf_json_with_vad(&raw_json, &vad_intervals)
        .map_err(WhisperExecError::Other)?;

    if lang.eq_ignore_ascii_case("auto") && !output.words.is_empty() {
        recover_bilingual_and_dropped_vad_windows(
            whisper_bin,
            whisper_model_path,
            wav_path,
            &vad_intervals,
            glossary,
            cancel_flag,
            &mut output,
            on_progress,
        )
        .await;
    }

    Ok(output)
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

    #[test]
    fn infers_dtw_model_presets_and_parses_vad_stderr() {
        assert_eq!(
            infer_dtw_model_preset(Path::new("/models/ggml-large-v3-turbo-q5_0.bin")),
            Some("large.v3.turbo")
        );
        assert_eq!(
            infer_dtw_model_preset(Path::new("/models/ggml-medium.en.bin")),
            Some("medium.en")
        );
        assert_eq!(infer_dtw_model_preset(Path::new("/models/custom.bin")), None);

        let stderr = r#"
whisper_vad_segments_from_probs: VAD segment 0: start = 0.51, end = 1.89 (duration: 1.38)
whisper_vad_segments_from_probs: VAD segment 1: start = 2.79, end = 6.94 (duration: 4.15)
"#;
        let intervals = parse_vad_segments_from_stderr(stderr);
        assert_eq!(intervals, vec![(510, 1890), (2790, 6940)]);
    }

    #[test]
    fn maps_dtw_tokens_piecewise_across_multi_vad_intervals_and_skips_noise_intro() {
        // Case A: segment spanning two VAD speech intervals separated by a 900ms silence
        let vad_speech = vec![(510, 1890), (2790, 3400)];
        let payload = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 510, "to": 3400 },
                    "text": " My fellow citizens, I have requested",
                    "tokens": [
                        { "text": "[_BEG_]", "offsets": { "from": 0, "to": 0 }, "t_dtw": -1, "p": 0.99 },
                        { "text": " My", "offsets": { "from": 80, "to": 100 }, "t_dtw": 18, "p": 0.96 },
                        { "text": " fellow", "offsets": { "from": 100, "to": 420 }, "t_dtw": 46, "p": 0.97 },
                        { "text": " citizens", "offsets": { "from": 420, "to": 820 }, "t_dtw": 94, "p": 0.98 },
                        { "text": ",", "offsets": { "from": 820, "to": 960 }, "t_dtw": 144, "p": 0.99 },
                        { "text": " I", "offsets": { "from": 960, "to": 1000 }, "t_dtw": 174, "p": 0.96 },
                        { "text": " have", "offsets": { "from": 1030, "to": 1300 }, "t_dtw": 198, "p": 0.97 },
                        { "text": " requested", "offsets": { "from": 1610, "to": 2070 }, "t_dtw": 244, "p": 0.97 }
                    ]
                }
            ]
        }"#;

        let out = parse_whisper_ojf_json_with_vad(payload, &vad_speech).unwrap();
        assert_eq!(out.words.len(), 6);
        assert_eq!(out.words[0].text, "My");
        assert_eq!(out.words[0].start_ms, 510);
        assert_eq!(out.words[2].text, "citizens,");
        assert_eq!(out.words[3].text, "I");
        let silence_gap = out.words[3].start_ms.saturating_sub(out.words[2].end_ms);
        assert!(
            silence_gap >= 900,
            "expected >= 900ms silence preserved between VAD 0 (1890ms) and VAD 1 (2790ms), got {silence_gap}"
        );
        assert!(out.words[3].start_ms >= 2790);

        // Case B: opening non-speech static VAD burst [1120..1820] followed by real speech [5380..9890]
        let vad_with_static = vec![(1120, 1820), (5380, 9890)];
        let payload_static = r#"{
            "result": { "language": "en" },
            "transcription": [
                {
                    "offsets": { "from": 1120, "to": 9890 },
                    "text": " This chase after",
                    "tokens": [
                        { "text": " This", "offsets": { "from": 0, "to": 420 }, "t_dtw": 110, "p": 0.95 },
                        { "text": " chase", "offsets": { "from": 420, "to": 940 }, "t_dtw": 154, "p": 0.96 },
                        { "text": " after", "offsets": { "from": 940, "to": 1460 }, "t_dtw": 192, "p": 0.97 }
                    ]
                }
            ]
        }"#;
        let out_static = parse_whisper_ojf_json_with_vad(payload_static, &vad_with_static).unwrap();
        assert_eq!(out_static.words.len(), 3);
        assert!(
            out_static.words[0].start_ms >= 5380,
            "expected 'This' to snap to real speech start 5380ms rather than 1120ms static burst, got {}",
            out_static.words[0].start_ms
        );
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

    #[cfg(unix)]
    #[tokio::test]
    async fn drains_large_stderr_above_os_pipe_buffer_without_deadlocking() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_whisper_pipe_test_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let bin_path = temp_dir.join("whisper-large-stderr");
        let model = temp_dir.join("model.bin");
        let wav = temp_dir.join("audio.wav");
        fs::write(&model, b"model").unwrap();
        fs::write(&wav, b"RIFF0000WAVE").unwrap();

        {
            let mut f = fs::File::create(&bin_path).unwrap();
            f.write_all(
                br#"#!/usr/bin/env sh
OUT=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-of" ]; then OUT="$2"; shift 2; else shift 1; fi
done
i=0
while [ "$i" -lt 4000 ]; do
  echo "Speech segment $i: start = 1.00, end = 2.00" >&2
  i=$((i + 1))
done
printf '{"result":{"language":"en"},"transcription":[{"offsets":{"from":1000,"to":2000},"text":" Done.","tokens":[{"text":" Done.","offsets":{"from":1000,"to":2000},"p":0.99}]}]}' > "${OUT}.json"
"#,
            )
            .unwrap();
            f.sync_all().unwrap();
        }
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755)).unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;

        let cancel = Arc::new(AtomicBool::new(false));
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            run_whisper_cli(
                &bin_path,
                &model,
                None,
                &wav,
                "en",
                &BTreeMap::new(),
                &cancel,
                |_, _| {},
            ),
        )
        .await
        .expect("run_whisper_cli deadlocked while draining large stderr")
        .unwrap();

        assert_eq!(out.words.len(), 1);
        assert_eq!(out.words[0].text, "Done.");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extracts_pcm16_wav_slice_and_recovers_bilingual_intro_and_dropped_vad_gap() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_bilingual_recovery_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let full_wav = temp_dir.join("interview.16k.wav");
        let vad_model = temp_dir.join("vad.bin");
        let whisper_model = temp_dir.join("ggml-large-v3-turbo.bin");
        let bin_path = temp_dir.join("mock-whisper");
        fs::write(&vad_model, b"vad").unwrap();
        fs::write(&whisper_model, b"model").unwrap();

        // Build a 35,000 ms 16 kHz mono PCM16 WAV with an extra LIST/INFO chunk before "data"
        let total_pcm_bytes = 35_000usize * 32;
        let info_chunk_payload = b"test";
        let riff_size = (4 + 8 + 16 + 8 + info_chunk_payload.len() + 8 + total_pcm_bytes) as u32;
        let mut wav_bytes = Vec::with_capacity(riff_size as usize + 8);
        wav_bytes.extend_from_slice(b"RIFF");
        wav_bytes.extend_from_slice(&riff_size.to_le_bytes());
        wav_bytes.extend_from_slice(b"WAVE");
        wav_bytes.extend_from_slice(b"fmt ");
        wav_bytes.extend_from_slice(&16u32.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&1u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16_000u32.to_le_bytes());
        wav_bytes.extend_from_slice(&32_000u32.to_le_bytes());
        wav_bytes.extend_from_slice(&2u16.to_le_bytes());
        wav_bytes.extend_from_slice(&16u16.to_le_bytes());
        wav_bytes.extend_from_slice(b"LIST");
        wav_bytes.extend_from_slice(&(info_chunk_payload.len() as u32).to_le_bytes());
        wav_bytes.extend_from_slice(info_chunk_payload);
        wav_bytes.extend_from_slice(b"data");
        wav_bytes.extend_from_slice(&(total_pcm_bytes as u32).to_le_bytes());
        wav_bytes.resize(wav_bytes.len() + total_pcm_bytes, 0u8);
        fs::write(&full_wav, &wav_bytes).unwrap();

        // Verify exact WAV slice extraction across arbitrary chunks
        let slice_check = temp_dir.join("slice_check.wav");
        extract_pcm16_wav_slice(&full_wav, &slice_check, 1_000, 3_000).unwrap();
        let slice_bytes = fs::read(&slice_check).unwrap();
        assert_eq!(slice_bytes.len(), 44 + 2_000 * 32);
        assert_eq!(&slice_bytes[0..4], b"RIFF");
        assert_eq!(&slice_bytes[36..40], b"data");

        {
            let mut f = fs::File::create(&bin_path).unwrap();
            f.write_all(
                r#"#!/usr/bin/env sh
LANG=""
OUT=""
WAV=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -l) LANG="$2"; shift 2 ;;
    -of) OUT="$2"; shift 2 ;;
    -f) WAV="$2"; shift 2 ;;
    *) shift 1 ;;
  esac
done

case "$WAV" in
  *.intro_retry.wav)
    printf '{"result":{"language":"%s"},"transcription":[{"offsets":{"from":100,"to":6000},"text":" Poštovani gledaoci.","tokens":[{"text":" Poštovani","offsets":{"from":100,"to":3000},"p":0.95},{"text":" gledaoci.","offsets":{"from":3000,"to":6000},"p":0.93}]}]}' "$LANG" > "${OUT}.json"
    ;;
  *.gap_0_0.wav)
    printf '{"result":{"language":"en"},"transcription":[{"offsets":{"from":0,"to":5000},"text":" Derek, thank you.","tokens":[{"text":" Derek,","offsets":{"from":0,"to":1500},"p":0.96},{"text":" thank","offsets":{"from":1500,"to":3200},"p":0.95},{"text":" you.","offsets":{"from":3200,"to":5000},"p":0.94}]}]}' > "${OUT}.json"
    ;;
  *)
    cat >&2 << 'VADEOF'
whisper_vad_segments_from_probs: VAD segment 0: start = 0.13, end = 12.03 (duration: 11.90)
whisper_vad_segments_from_probs: VAD segment 1: start = 12.23, end = 29.50 (duration: 17.27)
whisper_vad_segments_from_probs: VAD segment 2: start = 30.46, end = 34.00 (duration: 3.54)
VADEOF
    cat << 'JSONEOF' > "${OUT}.json"
{
  "result": { "language": "en" },
  "transcription": [
    {
      "offsets": { "from": 130, "to": 11789 },
      "text": " Welcome Cabarela.",
      "tokens": [
        { "text": " Welcome", "offsets": { "from": 130, "to": 5000 }, "p": 0.25 },
        { "text": " Cabarela.", "offsets": { "from": 5000, "to": 11789 }, "p": 0.48 }
      ]
    },
    {
      "offsets": { "from": 30460, "to": 34000 },
      "text": " That was the plan.",
      "tokens": [
        { "text": " That", "offsets": { "from": 30460, "to": 31200 }, "p": 0.96 },
        { "text": " was", "offsets": { "from": 31200, "to": 32000 }, "p": 0.97 },
        { "text": " the", "offsets": { "from": 32000, "to": 33000 }, "p": 0.98 },
        { "text": " plan.", "offsets": { "from": 33000, "to": 34000 }, "p": 0.95 }
      ]
    }
  ]
}
JSONEOF
    ;;
esac
"#
                .as_bytes(),
            )
            .unwrap();
            f.sync_all().unwrap();
        }
        fs::set_permissions(&bin_path, fs::Permissions::from_mode(0o755)).unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;

        let cancel = Arc::new(AtomicBool::new(false));
        let out = run_whisper_cli(
            &bin_path,
            &whisper_model,
            Some(&vad_model),
            &full_wav,
            "auto",
            &BTreeMap::new(),
            &cancel,
            |_, _| {},
        )
        .await
        .unwrap();

        let texts: Vec<&str> = out.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Poštovani",
                "gledaoci.",
                "Derek,",
                "thank",
                "you.",
                "That",
                "was",
                "the",
                "plan."
            ]
        );
        assert!(out.words[0].start_ms <= 500);
        assert!(out.words[2].start_ms >= 12_230 && out.words[4].end_ms <= 30_420);
        assert_eq!(out.words[5].start_ms, 30_460);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
