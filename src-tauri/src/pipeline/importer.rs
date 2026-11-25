//! Subtitle file importer and parser for `.srt` (SubRip) and `.vtt` (WebVTT) files.
//!
//! Handles optional UTF-8 BOM (`\u{FEFF}`), CRLF/CR line-ending normalization,
//! `.vtt` header/metadata/NOTE/STYLE/REGION blocks, standard and short timestamps
//! (`HH:MM:SS,mmm`, `HH:MM:SS.mmm`, `MM:SS.mmm`), trailing WebVTT cue settings,
//! multi-line cue text, CPS/warning calculation, and optional Serbian script normalization.

use crate::models::{JobOptions, JobPhase, SegmentRules, SubtitleCue, SubtitleProject};
use crate::pipeline::{segmenter, sr_script};
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;

/// Parses a timestamp string (`HH:MM:SS,mmm`, `HH:MM:SS.mmm`, `MM:SS.mmm`, or `MM:SS,mmm`)
/// into milliseconds.
pub fn parse_timestamp_ms(raw: &str) -> Result<u64> {
    let s = raw.trim();
    let (time_part, millis_str) = s.split_once([',', '.']).ok_or_else(|| {
        anyhow!("Timestamp missing millisecond separator (',' or '.'): '{s}'")
    })?;

    if millis_str.is_empty()
        || millis_str.len() > 3
        || !millis_str.chars().all(|c| c.is_ascii_digit())
    {
        return Err(anyhow!("Invalid milliseconds in timestamp: '{s}'"));
    }

    let millis_val: u64 = millis_str
        .parse()
        .with_context(|| format!("Failed to parse milliseconds in '{s}'"))?;
    let millis = match millis_str.len() {
        1 => millis_val * 100,
        2 => millis_val * 10,
        _ => millis_val,
    };

    let parts: Vec<&str> = time_part.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [h_str, m_str, s_str] => {
            if h_str.is_empty()
                || m_str.is_empty()
                || s_str.is_empty()
                || !h_str.chars().all(|c| c.is_ascii_digit())
                || !m_str.chars().all(|c| c.is_ascii_digit())
                || !s_str.chars().all(|c| c.is_ascii_digit())
            {
                return Err(anyhow!("Invalid HH:MM:SS fields in timestamp: '{s}'"));
            }
            let h: u64 = h_str.parse()?;
            let m: u64 = m_str.parse()?;
            let sec: u64 = s_str.parse()?;
            (h, m, sec)
        }
        [m_str, s_str] => {
            if m_str.is_empty()
                || s_str.is_empty()
                || !m_str.chars().all(|c| c.is_ascii_digit())
                || !s_str.chars().all(|c| c.is_ascii_digit())
            {
                return Err(anyhow!("Invalid MM:SS fields in timestamp: '{s}'"));
            }
            let m: u64 = m_str.parse()?;
            let sec: u64 = s_str.parse()?;
            (0, m, sec)
        }
        _ => {
            return Err(anyhow!("Invalid timestamp format: '{s}'"));
        }
    };

    if minutes >= 60 || seconds >= 60 {
        return Err(anyhow!(
            "Timestamp minutes or seconds out of range (0..59): '{s}'"
        ));
    }

    Ok(hours * 3_600_000 + minutes * 60_000 + seconds * 1_000 + millis)
}

fn parse_timing_line(line: &str) -> Result<(u64, u64)> {
    let (left, right) = line
        .split_once("-->")
        .ok_or_else(|| anyhow!("Missing '-->' in subtitle timing line: '{line}'"))?;

    if right.contains("-->") {
        return Err(anyhow!(
            "Malformed subtitle timing line with multiple '-->': '{line}'"
        ));
    }

    let start_ms = parse_timestamp_ms(left.trim())?;
    let end_token = right
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("Missing end timestamp in subtitle timing line: '{line}'"))?;
    let end_ms = parse_timestamp_ms(end_token)?;

    if end_ms < start_ms {
        return Err(anyhow!(
            "Cue end timestamp ({end_ms} ms) is earlier than start timestamp ({start_ms} ms) in '{line}'"
        ));
    }

    Ok((start_ms, end_ms))
}

fn is_webvtt_header_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "WEBVTT"
        || trimmed.starts_with("WEBVTT ")
        || trimmed.starts_with("WEBVTT\t")
        || trimmed.starts_with("WEBVTT-")
}

fn is_vtt_non_cue_block(first_line: &str) -> bool {
    let trimmed = first_line.trim();
    trimmed == "NOTE"
        || trimmed.starts_with("NOTE ")
        || trimmed.starts_with("NOTE\t")
        || trimmed == "STYLE"
        || trimmed.starts_with("STYLE ")
        || trimmed.starts_with("STYLE\t")
        || trimmed == "REGION"
        || trimmed.starts_with("REGION ")
        || trimmed.starts_with("REGION\t")
}

fn is_vtt_metadata_block(lines: &[&str]) -> bool {
    !lines.is_empty()
        && lines.iter().all(|l| {
            let t = l.trim();
            !t.contains("-->") && (t.contains(':') || t.contains('='))
        })
}

fn parse_subtitle_content_internal(
    content: &str,
    rules: &SegmentRules,
    is_vtt_hint: bool,
) -> Result<Vec<SubtitleCue>> {
    let without_bom = content.strip_prefix('\u{FEFF}').unwrap_or(content);
    let normalized = without_bom.replace("\r\n", "\n").replace('\r', "\n");

    if normalized.trim().is_empty() {
        return Err(anyhow!("Subtitle file is empty"));
    }

    let normalized_rules = segmenter::normalize_rules(rules);

    // Group non-empty lines into blocks separated by one or more blank lines
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut current_block: Vec<&str> = Vec::new();

    for line in normalized.lines() {
        if line.trim().is_empty() {
            if !current_block.is_empty() {
                blocks.push(std::mem::take(&mut current_block));
            }
        } else {
            current_block.push(line);
        }
    }
    if !current_block.is_empty() {
        blocks.push(current_block);
    }

    if blocks.is_empty() {
        return Err(anyhow!("Subtitle file contains no content blocks"));
    }

    let starts_with_webvtt = blocks
        .first()
        .and_then(|b| b.first())
        .map(|l| is_webvtt_header_line(l))
        .unwrap_or(false);
    let is_vtt = is_vtt_hint || starts_with_webvtt;

    let mut cues: Vec<SubtitleCue> = Vec::new();

    for (block_idx, block) in blocks.iter().enumerate() {
        if block.is_empty() {
            continue;
        }

        let mut slice: &[&str] = block.as_slice();

        // Handle leading WEBVTT header block
        if block_idx == 0 && is_webvtt_header_line(slice[0]) {
            if let Some(arrow_pos) = slice.iter().position(|l| l.contains("-->")) {
                // In case a cue immediately follows the WEBVTT header without a blank line
                let start_pos = if arrow_pos > 1
                    && !slice[arrow_pos - 1].contains(':')
                    && !slice[arrow_pos - 1].contains('=')
                {
                    arrow_pos - 1
                } else {
                    arrow_pos
                };
                slice = &slice[start_pos..];
            } else {
                continue;
            }
        }

        if is_vtt {
            if is_vtt_non_cue_block(slice[0]) && !slice.iter().any(|l| l.contains("-->")) {
                continue;
            }
            if cues.is_empty() && is_vtt_metadata_block(slice) {
                continue;
            }
        }

        // Locate the timing line containing `-->` (either line 0, or line 1 after a cue ID/number)
        let timing_idx = if slice[0].contains("-->") {
            0
        } else if slice.len() >= 2 && slice[1].contains("-->") {
            1
        } else {
            return Err(anyhow!(
                "Malformed subtitle block starting with '{}': missing '-->' timestamp line",
                slice[0].trim()
            ));
        };

        let (start_ms, end_ms) = parse_timing_line(slice[timing_idx])?;
        let text_lines: Vec<&str> = slice[timing_idx + 1..]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        if text_lines.is_empty() {
            return Err(anyhow!(
                "Subtitle cue at '{}' has no text lines",
                slice[timing_idx].trim()
            ));
        }

        let source_text = text_lines.join("\n");
        let mut cue = SubtitleCue {
            id: (cues.len() + 1) as u32,
            start_ms,
            end_ms,
            source_text,
            translated_text: String::new(),
            translated_variants: std::collections::BTreeMap::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        segmenter::compute_cue_metrics(&mut cue, &normalized_rules);
        cues.push(cue);
    }

    if cues.is_empty() {
        return Err(anyhow!("No valid subtitle cues found in input"));
    }

    Ok(cues)
}

/// Parses raw `.srt` or `.vtt` subtitle text into sequential `SubtitleCue`s with computed CPS and warnings.
pub fn parse_subtitle_content(content: &str, rules: &SegmentRules) -> Result<Vec<SubtitleCue>> {
    parse_subtitle_content_internal(content, rules, false)
}

/// Imports an `.srt` or `.vtt` file from disk into a `SubtitleProject`.
pub fn import_subtitle_path(path: &Path, options: &JobOptions) -> Result<SubtitleProject> {
    if !path.exists() || !path.is_file() {
        return Err(anyhow!(
            "Subtitle file does not exist: {}",
            path.to_string_lossy()
        ));
    }

    let raw = fs::read_to_string(path).with_context(|| {
        format!(
            "Failed to read subtitle file at {}",
            path.to_string_lossy()
        )
    })?;

    let is_vtt_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("vtt"))
        .unwrap_or(false);

    let normalized_rules = segmenter::normalize_rules(&options.segment_rules);
    let mut cues = parse_subtitle_content_internal(&raw, &normalized_rules, is_vtt_ext)?;

    if options.source_language.trim().eq_ignore_ascii_case("sr") {
        for cue in &mut cues {
            cue.source_text = sr_script::convert_script(&cue.source_text, options.script);
            segmenter::compute_cue_metrics(cue, &normalized_rules);
        }
    }

    let duration_ms = cues.iter().map(|c| c.end_ms).max().unwrap_or(0);
    let audio_file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("subtitles.srt")
        .to_string();

    let detected_language = {
        let src = options.source_language.trim();
        if src.is_empty() || src.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(src.to_string())
        }
    };

    Ok(SubtitleProject {
        job_id: uuid::Uuid::new_v4().to_string(),
        audio_path: path.to_string_lossy().to_string(),
        audio_file_name,
        duration_ms,
        detected_language,
        options: JobOptions {
            segment_rules: normalized_rules,
            ..options.clone()
        },
        words: Vec::new(),
        cues,
        phase: JobPhase::Completed,
        completed_translation_batches: 0,
        topic_context: None,
        completed_translation_style: None,
        error_message: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[derive(Debug, Clone)]
struct CompanionSubtitleCandidate {
    path: std::path::PathBuf,
    is_vtt: bool,
    tag: Option<String>,
}

fn parse_companion_file(
    candidate: &CompanionSubtitleCandidate,
    rules: &SegmentRules,
) -> Result<Vec<SubtitleCue>> {
    let raw = fs::read_to_string(&candidate.path).with_context(|| {
        format!(
            "Failed to read companion subtitle file at {}",
            candidate.path.to_string_lossy()
        )
    })?;
    parse_subtitle_content_internal(&raw, rules, candidate.is_vtt)
}

fn pick_preferred_candidate<'a>(
    candidates: &[&'a CompanionSubtitleCandidate],
) -> Option<&'a CompanionSubtitleCandidate> {
    candidates
        .iter()
        .copied()
        .min_by_key(|c| (c.is_vtt, c.path.as_os_str().to_os_string()))
}

fn pick_source_tag_language(tag: Option<&str>, options: &JobOptions) -> Option<String> {
    if let Some(t) = tag {
        let base = t.strip_suffix(".source").unwrap_or(t).trim();
        if !base.is_empty() && !base.eq_ignore_ascii_case("src") {
            return Some(base.to_string());
        }
    }
    let src = options.source_language.trim();
    if src.is_empty() || src.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(src.to_string())
    }
}

/// Searches the directory of `audio_path` for companion `.srt`/`.vtt` files sharing its stem
/// (`{stem}.srt`, `{stem}.{src}.srt`, `{stem}.{dst}.srt`, etc.) and returns a populated
/// `SubtitleProject` bound to `audio_path` if found.
pub fn try_load_companion_subtitles(
    audio_path: &Path,
    options: &JobOptions,
) -> Result<Option<SubtitleProject>> {
    if !audio_path.exists() || !audio_path.is_file() {
        return Ok(None);
    }

    let ext_lower = audio_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(ext_lower.as_str(), "srt" | "vtt") {
        return Ok(None);
    }

    let parent_dir = match audio_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => return Ok(None),
    };
    let stem = match audio_path.file_stem().and_then(|s| s.to_str()) {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Ok(None),
    };

    let stem_lower = stem.to_lowercase();
    let entries = match fs::read_dir(parent_dir) {
        Ok(rd) => rd,
        Err(_) => return Ok(None),
    };

    let mut candidates: Vec<CompanionSubtitleCandidate> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(sub_ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let is_vtt = if sub_ext.eq_ignore_ascii_case("srt") {
            false
        } else if sub_ext.eq_ignore_ascii_case("vtt") {
            true
        } else {
            continue;
        };

        let Some(sub_stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let sub_stem_lower = sub_stem.to_lowercase();
        let tag = if sub_stem_lower == stem_lower {
            None
        } else if let Some(after) = sub_stem_lower.strip_prefix(&stem_lower) {
            let Some(rest) = after.strip_prefix('.') else {
                continue;
            };
            if rest.trim().is_empty() {
                continue;
            }
            Some(rest.to_string())
        } else {
            continue;
        };

        candidates.push(CompanionSubtitleCandidate { path, is_vtt, tag });
    }

    if candidates.is_empty() {
        return Ok(None);
    }

    let target_lang = {
        let t = options.target_language.trim().to_ascii_lowercase();
        if t.is_empty() {
            "sr".to_string()
        } else {
            t
        }
    };
    let source_lang_opt = {
        let s = options.source_language.trim().to_ascii_lowercase();
        if s.is_empty() || s == "auto" {
            None
        } else {
            Some(s)
        }
    };

    let target_matches: Vec<&CompanionSubtitleCandidate> = candidates
        .iter()
        .filter(|c| c.tag.as_deref() == Some(target_lang.as_str()))
        .collect();
    let target_cand = pick_preferred_candidate(&target_matches);

    let explicitly_sourced: Vec<&CompanionSubtitleCandidate> = candidates
        .iter()
        .filter(|c| {
            c.tag
                .as_deref()
                .map(|t| t.ends_with(".source"))
                .unwrap_or(false)
        })
        .collect();
    let matching_src_lang: Vec<&CompanionSubtitleCandidate> = candidates
        .iter()
        .filter(|c| {
            source_lang_opt
                .as_deref()
                .map(|src| src != target_lang && c.tag.as_deref() == Some(src))
                .unwrap_or(false)
        })
        .collect();
    let other_tagged: Vec<&CompanionSubtitleCandidate> = candidates
        .iter()
        .filter(|c| match c.tag.as_deref() {
            Some(t) => t != target_lang && !t.ends_with(".source"),
            None => false,
        })
        .collect();
    let untagged: Vec<&CompanionSubtitleCandidate> =
        candidates.iter().filter(|c| c.tag.is_none()).collect();

    let source_cand = pick_preferred_candidate(&explicitly_sourced)
        .or_else(|| pick_preferred_candidate(&matching_src_lang))
        .or_else(|| pick_preferred_candidate(&other_tagged))
        .or_else(|| pick_preferred_candidate(&untagged));

    let normalized_rules = segmenter::normalize_rules(&options.segment_rules);
    let (cues, detected_language): (Vec<SubtitleCue>, Option<String>) =
        match (source_cand, target_cand) {
            (Some(src), Some(tgt)) => {
                let mut cues = parse_companion_file(src, &normalized_rules)?;
                let target_cues = parse_companion_file(tgt, &normalized_rules)?;

                if cues.len() == target_cues.len() {
                    for (cue, tgt_cue) in cues.iter_mut().zip(target_cues.iter()) {
                        let converted =
                            sr_script::convert_script(&tgt_cue.source_text, options.script);
                        cue.translated_text = converted.clone();
                        if !converted.trim().is_empty() {
                            cue.translated_variants
                                .insert(options.translation_style, converted);
                        }
                        segmenter::compute_cue_metrics(cue, &normalized_rules);
                    }
                } else {
                    for cue in &mut cues {
                        let overlapping: Vec<String> = target_cues
                            .iter()
                            .filter(|tc| {
                                let overlap_start = cue.start_ms.max(tc.start_ms);
                                let overlap_end = cue.end_ms.min(tc.end_ms);
                                overlap_end > overlap_start
                            })
                            .map(|tc| sr_script::convert_script(&tc.source_text, options.script))
                            .collect();
                        if !overlapping.is_empty() {
                            let joined = overlapping.join("\n");
                            cue.translated_text = joined.clone();
                            cue.translated_variants
                                .insert(options.translation_style, joined);
                        }
                        segmenter::compute_cue_metrics(cue, &normalized_rules);
                    }
                }

                (
                    cues,
                    pick_source_tag_language(src.tag.as_deref(), options),
                )
            }
            (Some(src), None) => {
                let mut cues = parse_companion_file(src, &normalized_rules)?;
                if options.source_language.trim().eq_ignore_ascii_case("sr") {
                    for cue in &mut cues {
                        cue.source_text = sr_script::convert_script(&cue.source_text, options.script);
                        segmenter::compute_cue_metrics(cue, &normalized_rules);
                    }
                }
                (
                    cues,
                    pick_source_tag_language(src.tag.as_deref(), options),
                )
            }
            (None, Some(tgt)) => {
                let mut cues = parse_companion_file(tgt, &normalized_rules)?;
                for cue in &mut cues {
                    let converted = sr_script::convert_script(&cue.source_text, options.script);
                    cue.translated_text = converted.clone();
                    if !converted.trim().is_empty() {
                        cue.translated_variants
                            .insert(options.translation_style, converted);
                    }
                    segmenter::compute_cue_metrics(cue, &normalized_rules);
                }
                (cues, pick_source_tag_language(None, options))
            }
            (None, None) => return Ok(None),
        };

    let duration_ms = cues.iter().map(|c| c.end_ms).max().unwrap_or(0);
    let audio_file_name = audio_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio-input")
        .to_string();
    let has_translations = cues.iter().any(|c| !c.translated_text.trim().is_empty());

    Ok(Some(SubtitleProject {
        job_id: uuid::Uuid::new_v4().to_string(),
        audio_path: audio_path.to_string_lossy().to_string(),
        audio_file_name,
        duration_ms,
        detected_language,
        options: JobOptions {
            segment_rules: normalized_rules,
            ..options.clone()
        },
        words: Vec::new(),
        cues,
        phase: JobPhase::Completed,
        completed_translation_batches: 0,
        topic_context: None,
        completed_translation_style: has_translations.then_some(options.translation_style),
        error_message: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SerbianScript;

    #[test]
    fn parses_standard_srt_with_bom_crlf_serbian_diacritics_and_cyrillic() {
        let srt = "\u{FEFF}10\r\n\
                   00:00:00,500 --> 00:00:02,750\r\n\
                   Здраво свима!\r\n\
                   Честитамо Ђорђу.\r\n\
                   \r\n\
                   20\r\n\
                   00:00:03,000 --> 00:00:05,125\r\n\
                   Тест слова: čćžšđ ČĆŽŠĐ ЉЊЏ.\r\n";

        let rules = SegmentRules::default();
        let cues = parse_subtitle_content(srt, &rules).unwrap();

        assert_eq!(cues.len(), 2);
        // Sequential 1-based IDs regardless of original cue numbering (10, 20 -> 1, 2)
        assert_eq!(cues[0].id, 1);
        assert_eq!(cues[0].start_ms, 500);
        assert_eq!(cues[0].end_ms, 2750);
        assert_eq!(cues[0].source_text, "Здраво свима!\nЧеститамо Ђорђу.");
        assert!(cues[0].cps > 0.0);

        assert_eq!(cues[1].id, 2);
        assert_eq!(cues[1].start_ms, 3000);
        assert_eq!(cues[1].end_ms, 5125);
        assert_eq!(cues[1].source_text, "Тест слова: čćžšđ ČĆŽŠĐ ЉЊЏ.");
    }

    #[test]
    fn parses_vtt_with_header_metadata_notes_cue_settings_and_short_timestamps() {
        let vtt = "\u{FEFF}WEBVTT - EasySRT Studio Subtitles\n\
                   Kind: captions\n\
                   Language: sr\n\
                   \n\
                   NOTE This is a comment block that must be ignored\n\
                   \n\
                   intro-cue\n\
                   00:01.250 --> 00:03.500 align:start position:0% line:85%\n\
                   Dobrodošli u Beograd!\n\
                   Drugi red titla.\n\
                   \n\
                   00:00:04.000 --> 00:00:06.200 size:90%\n\
                   Čćžšđ i Љубав у џепу.\n";

        let rules = SegmentRules::default();
        let cues = parse_subtitle_content(vtt, &rules).unwrap();

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].id, 1);
        assert_eq!(cues[0].start_ms, 1250);
        assert_eq!(cues[0].end_ms, 3500);
        assert_eq!(
            cues[0].source_text,
            "Dobrodošli u Beograd!\nDrugi red titla."
        );

        assert_eq!(cues[1].id, 2);
        assert_eq!(cues[1].start_ms, 4000);
        assert_eq!(cues[1].end_ms, 6200);
        assert_eq!(cues[1].source_text, "Čćžšđ i Љубав у џепу.");
    }

    #[test]
    fn import_subtitle_path_normalizes_serbian_script_and_computes_duration() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_import_test_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let srt_path = temp_dir.join("epizoda.srt");

        let content = "1\n00:00:01,000 --> 00:00:03,200\nLjubav i pažnja u Beogradu.\n\n\
                       2\n00:00:03,500 --> 00:00:06,750\nInjekcija i nadživeti.\n";
        fs::write(&srt_path, content.as_bytes()).unwrap();

        let opts = JobOptions {
            source_language: "sr".to_string(),
            target_language: "sr".to_string(),
            script: SerbianScript::Cyrillic,
            ..JobOptions::default()
        };

        let project = import_subtitle_path(&srt_path, &opts).unwrap();
        assert_eq!(project.duration_ms, 6750);
        assert_eq!(project.cues.len(), 2);
        assert_eq!(project.cues[0].source_text, "Љубав и пажња у Београду.");
        assert_eq!(project.cues[1].source_text, "Инјекција и надживети.");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rejects_empty_and_malformed_subtitle_inputs() {
        let rules = SegmentRules::default();

        // Empty or BOM-only input
        assert!(parse_subtitle_content("", &rules).is_err());
        assert!(parse_subtitle_content("\u{FEFF}   \r\n  ", &rules).is_err());

        // Header-only WebVTT with no cues
        assert!(parse_subtitle_content("WEBVTT\n\nNOTE nothing here\n", &rules).is_err());

        // Missing timestamp arrow
        assert!(parse_subtitle_content("1\n00:00:01,000 00:00:02,000\nHello", &rules).is_err());

        // Malformed timestamp digits
        assert!(parse_subtitle_content("1\n00:00:XX,000 --> 00:00:02,000\nHello", &rules).is_err());

        // End timestamp earlier than start timestamp
        assert!(
            parse_subtitle_content("1\n00:00:05,000 --> 00:00:02,000\nHello", &rules).is_err()
        );

        // Cue block missing text lines
        assert!(parse_subtitle_content("1\n00:00:01,000 --> 00:00:02,000\n", &rules).is_err());
    }

    #[test]
    fn companion_subtitles_loads_paired_source_and_translated_srts() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_companion_pair_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let audio_path = temp_dir.join("interview.mp3");
        let en_srt = temp_dir.join("interview.en.srt");
        let sr_srt = temp_dir.join("interview.sr.srt");
        let unrelated_srt = temp_dir.join("interview_draft.srt");

        fs::write(&audio_path, b"dummy audio").unwrap();
        fs::write(
            &en_srt,
            "1\n00:00:00,500 --> 00:00:02,000\nHello everyone.\n\n2\n00:00:02,500 --> 00:00:04,500\nWelcome back.\n",
        )
        .unwrap();
        fs::write(
            &sr_srt,
            "1\n00:00:00,500 --> 00:00:02,000\nZdravo svima.\n\n2\n00:00:02,500 --> 00:00:04,500\nDobro došli nazad.\n",
        )
        .unwrap();
        fs::write(&unrelated_srt, "1\n00:00:00,000 --> 00:00:01,000\nUnrelated\n").unwrap();

        let opts = JobOptions {
            target_language: "sr".to_string(),
            script: SerbianScript::Latin,
            ..JobOptions::default()
        };

        let loaded = try_load_companion_subtitles(&audio_path, &opts)
            .unwrap()
            .expect("Expected companion subtitles project");

        assert_eq!(loaded.audio_path, audio_path.to_string_lossy());
        assert_eq!(loaded.audio_file_name, "interview.mp3");
        assert_eq!(loaded.phase, JobPhase::Completed);
        assert_eq!(loaded.detected_language.as_deref(), Some("en"));
        assert_eq!(loaded.cues.len(), 2);
        assert_eq!(loaded.cues[0].source_text, "Hello everyone.");
        assert_eq!(loaded.cues[0].translated_text, "Zdravo svima.");
        assert_eq!(loaded.cues[1].source_text, "Welcome back.");
        assert_eq!(loaded.cues[1].translated_text, "Dobro došli nazad.");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn companion_subtitles_loads_single_untagged_srt_beside_video() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_companion_single_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let video_path = temp_dir.join("clip.mp4");
        let srt_path = temp_dir.join("clip.srt");

        fs::write(&video_path, b"dummy video").unwrap();
        fs::write(
            &srt_path,
            "1\n00:00:01,000 --> 00:00:03,000\nSingle companion cue.\n",
        )
        .unwrap();

        let opts = JobOptions::default();
        let loaded = try_load_companion_subtitles(&video_path, &opts)
            .unwrap()
            .expect("Expected single companion project");

        assert_eq!(loaded.audio_path, video_path.to_string_lossy());
        assert_eq!(loaded.phase, JobPhase::Completed);
        assert_eq!(loaded.cues.len(), 1);
        assert_eq!(loaded.cues[0].source_text, "Single companion cue.");
        assert!(loaded.cues[0].translated_text.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn companion_subtitles_returns_none_when_no_matching_subtitles_exist() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_companion_none_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let audio_path = temp_dir.join("speech.wav");
        let other_srt = temp_dir.join("speech_other.srt");
        fs::write(&audio_path, b"dummy wav").unwrap();
        fs::write(&other_srt, "1\n00:00:00,500 --> 00:00:01,500\nOther.\n").unwrap();

        let loaded = try_load_companion_subtitles(&audio_path, &JobOptions::default()).unwrap();
        assert!(loaded.is_none());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
