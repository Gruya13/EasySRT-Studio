//! Word-to-cue subtitle segmentation, balanced line wrapping, and CPS/warning metrics.

use crate::models::{SegmentPreset, SegmentRules, SubtitleCue, TimedWord};

const PAUSE_SPLIT_THRESHOLD_MS: u64 = 350;
const SOFT_PAUSE_THRESHOLD_MS: u64 = 150;

/// Returns preset rules for `Standard` (2x42) or `Reels` (1x20), or preserves custom values.
pub fn normalize_rules(rules: &SegmentRules) -> SegmentRules {
    match rules.preset {
        SegmentPreset::Standard => SegmentRules {
            preset: SegmentPreset::Standard,
            max_chars_per_line: if rules.max_chars_per_line == 0 {
                42
            } else {
                rules.max_chars_per_line
            },
            max_lines: if rules.max_lines == 0 {
                2
            } else {
                rules.max_lines
            },
            min_duration_ms: if rules.min_duration_ms == 0 {
                1000
            } else {
                rules.min_duration_ms
            },
            max_duration_ms: if rules.max_duration_ms == 0 {
                7000
            } else {
                rules.max_duration_ms
            },
            max_cps: if rules.max_cps <= 0.0 {
                17.0
            } else {
                rules.max_cps
            },
        },
        SegmentPreset::Reels => SegmentRules {
            preset: SegmentPreset::Reels,
            max_chars_per_line: if rules.max_chars_per_line == 0 || rules.max_chars_per_line == 42 {
                20
            } else {
                rules.max_chars_per_line
            },
            max_lines: if rules.max_lines == 0 || rules.max_lines == 2 {
                1
            } else {
                rules.max_lines
            },
            min_duration_ms: if rules.min_duration_ms == 0 {
                600
            } else {
                rules.min_duration_ms
            },
            max_duration_ms: if rules.max_duration_ms == 0 || rules.max_duration_ms == 7000 {
                3500
            } else {
                rules.max_duration_ms
            },
            max_cps: if rules.max_cps <= 0.0 {
                18.0
            } else {
                rules.max_cps
            },
        },
        SegmentPreset::Custom => SegmentRules {
            preset: SegmentPreset::Custom,
            max_chars_per_line: rules.max_chars_per_line.max(10),
            max_lines: rules.max_lines.clamp(1, 4),
            min_duration_ms: rules.min_duration_ms.max(200),
            max_duration_ms: rules.max_duration_ms.max(1000),
            max_cps: if rules.max_cps <= 0.0 {
                17.0
            } else {
                rules.max_cps
            },
        },
    }
}

fn ends_with_sentence_punct(word: &str) -> bool {
    let trimmed = word.trim_end_matches(['"', '\'', '”', '’', ')', ']']);
    if trimmed.is_empty() {
        return false;
    }
    // Avoid splitting after common short Serbian/English abbreviations like "dr.", "g.", "br."
    let lower = trimmed.to_lowercase();
    for abbr in ["dr.", "mr.", "sv.", "br.", "st.", "prof.", "tj.", "npr."] {
        if lower == abbr {
            return false;
        }
    }
    trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || trimmed.ends_with('…')
}

fn ends_with_clause_punct(word: &str) -> bool {
    let trimmed = word.trim_end_matches(['"', '\'', '”', '’', ')', ']']);
    trimmed.ends_with(',')
        || trimmed.ends_with(';')
        || trimmed.ends_with(':')
        || trimmed.ends_with('—')
        || trimmed.ends_with('–')
}

/// Groups `TimedWord`s into readable `SubtitleCue`s according to `rules`.
pub fn segment_words(words: &[TimedWord], rules: &SegmentRules) -> Vec<SubtitleCue> {
    let rules = normalize_rules(rules);
    let cleaned_words: Vec<TimedWord> = words
        .iter()
        .filter_map(|w| {
            let t = w.text.trim();
            if t.is_empty() {
                None
            } else {
                Some(TimedWord {
                    start_ms: w.start_ms,
                    end_ms: w.end_ms.max(w.start_ms),
                    text: t.to_string(),
                    probability: w.probability,
                })
            }
        })
        .collect();

    if cleaned_words.is_empty() {
        return Vec::new();
    }

    let max_total_chars = (rules.max_chars_per_line.max(1) * rules.max_lines.max(1)) as usize;
    let mut cues = Vec::new();
    let mut start_idx = 0;

    while start_idx < cleaned_words.len() {
        let cue_start_ms = cleaned_words[start_idx].start_ms;
        let mut last_fit_idx = start_idx;
        let mut char_len = cleaned_words[start_idx].text.chars().count();
        let mut hit_hard_limit = false;

        for i in start_idx..cleaned_words.len() {
            if i > start_idx {
                let added_chars = 1 + cleaned_words[i].text.chars().count();
                let candidate_chars = char_len + added_chars;
                let candidate_duration = cleaned_words[i].end_ms.saturating_sub(cue_start_ms);

                if candidate_chars > max_total_chars || candidate_duration > rules.max_duration_ms {
                    hit_hard_limit = true;
                    break;
                }
                char_len = candidate_chars;
                last_fit_idx = i;
            }

            // Priority 1: Break immediately on sentence-ending punctuation
            if ends_with_sentence_punct(&cleaned_words[i].text) {
                last_fit_idx = i;
                hit_hard_limit = false;
                break;
            }

            // Priority 3: Break on speech pause >= 350ms between consecutive words
            if let Some(next_word) = cleaned_words.get(i + 1) {
                let gap = next_word.start_ms.saturating_sub(cleaned_words[i].end_ms);
                if gap >= PAUSE_SPLIT_THRESHOLD_MS {
                    last_fit_idx = i;
                    hit_hard_limit = false;
                    break;
                }
            }
        }

        // If we stopped because adding the next word would exceed max chars or max duration,
        // look back within start_idx..=last_fit_idx for a natural clause boundary or pause.
        let mut end_idx = last_fit_idx;
        if hit_hard_limit && last_fit_idx > start_idx {
            // Priority 2: Clause punctuation (`,`, `;`, `:`, `—`)
            if let Some(clause_idx) = (start_idx..=last_fit_idx)
                .rev()
                .find(|&idx| ends_with_clause_punct(&cleaned_words[idx].text))
            {
                end_idx = clause_idx;
            } else if let Some(pause_idx) = (start_idx..last_fit_idx).rev().find(|&idx| {
                cleaned_words[idx + 1]
                    .start_ms
                    .saturating_sub(cleaned_words[idx].end_ms)
                    >= SOFT_PAUSE_THRESHOLD_MS
            }) {
                end_idx = pause_idx;
            }
        }

        let raw_text = cleaned_words[start_idx..=end_idx]
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        let wrapped_text =
            wrap_balanced_lines(&raw_text, rules.max_chars_per_line, rules.max_lines);

        let start_ms = cleaned_words[start_idx].start_ms;
        let mut end_ms = cleaned_words[end_idx].end_ms;

        // Enforce min_duration_ms where possible without overlapping the next word
        let desired_min_end = start_ms.saturating_add(rules.min_duration_ms);
        if end_ms < desired_min_end {
            if let Some(next_word) = cleaned_words.get(end_idx + 1) {
                end_ms = desired_min_end.min(next_word.start_ms).max(end_ms);
            } else {
                end_ms = desired_min_end;
            }
        }

        let mut cue = SubtitleCue {
            id: (cues.len() + 1) as u32,
            start_ms,
            end_ms,
            source_text: wrapped_text,
            translated_text: String::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut cue, &rules);
        cues.push(cue);

        start_idx = end_idx + 1;
    }

    cues
}

/// Balances text into at most `max_lines` lines (preferring 1 line if it fits within
/// `max_chars_per_line`, or splitting into 2 balanced lines at a word/punctuation boundary).
pub fn wrap_balanced_lines(text: &str, max_chars_per_line: u32, max_lines: u32) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }

    let single_line = words.join(" ");
    let max_chars = max_chars_per_line.max(1) as usize;

    if max_lines <= 1 || single_line.chars().count() <= max_chars || words.len() == 1 {
        return single_line;
    }

    // Find optimal 2-line split near the visual midpoint
    let mut best_split = 1;
    let mut best_score = isize::MAX;

    for k in 1..words.len() {
        let line1 = words[..k].join(" ");
        let line2 = words[k..].join(" ");
        let len1 = line1.chars().count();
        let len2 = line2.chars().count();

        let overflow1 = len1.saturating_sub(max_chars) as isize;
        let overflow2 = len2.saturating_sub(max_chars) as isize;
        let overflow_penalty = (overflow1 + overflow2) * 1000;

        let diff = (len1 as isize - len2 as isize).abs();
        let punct_bonus = if ends_with_clause_punct(words[k - 1])
            || ends_with_sentence_punct(words[k - 1])
        {
            6
        } else {
            0
        };

        let score = overflow_penalty + diff - punct_bonus;
        if score < best_score {
            best_score = score;
            best_split = k;
        }
    }

    let line1 = words[..best_split].join(" ");
    let line2 = words[best_split..].join(" ");
    format!("{line1}\n{line2}")
}

/// Computes `cue.cps` (rounded to 1 decimal place) and populates `cue.warnings`
/// (`"high_cps"`, `"line_too_long"`, `"duration_too_short"`, `"duration_too_long"`).
pub fn compute_cue_metrics(cue: &mut SubtitleCue, rules: &SegmentRules) {
    let visible = if !cue.translated_text.trim().is_empty() {
        cue.translated_text.as_str()
    } else {
        cue.source_text.as_str()
    };

    let char_count = visible
        .chars()
        .filter(|&c| c != '\n' && c != '\r')
        .count();
    let duration_ms = cue.end_ms.saturating_sub(cue.start_ms);

    let cps = if duration_ms == 0 {
        if char_count > 0 {
            999.0
        } else {
            0.0
        }
    } else {
        let raw = (char_count as f64) / (duration_ms as f64 / 1000.0);
        (raw * 10.0).round() / 10.0
    };
    cue.cps = cps;

    let mut warnings = Vec::new();
    if cps > rules.max_cps {
        warnings.push("high_cps".to_string());
    }

    let max_chars = rules.max_chars_per_line as usize;
    let lines: Vec<&str> = visible.lines().collect();
    if lines.len() > rules.max_lines as usize
        || lines.iter().any(|l| l.chars().count() > max_chars)
    {
        warnings.push("line_too_long".to_string());
    }

    if duration_ms < rules.min_duration_ms {
        warnings.push("duration_too_short".to_string());
    }
    if duration_ms > rules.max_duration_ms {
        warnings.push("duration_too_long".to_string());
    }

    cue.warnings = warnings;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start_ms: u64, end_ms: u64, text: &str) -> TimedWord {
        TimedWord {
            start_ms,
            end_ms,
            text: text.to_string(),
            probability: 0.99,
        }
    }

    #[test]
    fn splits_on_sentence_punctuation_and_enforces_min_duration() {
        let words = vec![
            word(0, 200, "Zdravo"),
            word(200, 450, "svima."),
            word(1200, 1500, "Kako"),
            word(1500, 1800, "ste"),
            word(1800, 2200, "danas?"),
        ];
        let rules = SegmentRules::default(); // min_duration_ms = 1000
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].id, 1);
        assert_eq!(cues[0].source_text, "Zdravo svima.");
        // Extended from 450ms to 1000ms since next word starts at 1200ms
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].end_ms, 1000);

        assert_eq!(cues[1].id, 2);
        assert_eq!(cues[1].source_text, "Kako ste danas?");
        assert_eq!(cues[1].start_ms, 1200);
        assert_eq!(cues[1].end_ms, 2200);
    }

    #[test]
    fn splits_on_speech_pause_and_clause_punctuation() {
        // Pause >= 350ms between "ovde" (end 800) and "idemo" (start 1300)
        let words = vec![
            word(0, 400, "Čekaj"),
            word(400, 800, "ovde"),
            word(1300, 1700, "idemo"),
            word(1700, 2100, "dalje."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].source_text, "Čekaj ovde");
        assert_eq!(cues[1].source_text, "idemo dalje.");
    }

    #[test]
    fn reels_preset_produces_short_single_line_cues() {
        let words = vec![
            word(0, 300, "Ovo"),
            word(300, 600, "je"),
            word(600, 900, "kratak"),
            word(900, 1200, "Reels"),
            word(1200, 1500, "titl"),
            word(1500, 1800, "za"),
            word(1800, 2200, "društvene"),
            word(2200, 2600, "mreže."),
        ];
        let rules = SegmentRules {
            preset: SegmentPreset::Reels,
            max_chars_per_line: 20,
            max_lines: 1,
            min_duration_ms: 500,
            max_duration_ms: 3000,
            max_cps: 18.0,
        };
        let cues = segment_words(&words, &rules);
        assert!(cues.len() >= 2);
        for cue in &cues {
            assert!(!cue.source_text.contains('\n'));
            assert!(cue.source_text.chars().count() <= 20);
        }
    }

    #[test]
    fn wrap_balanced_lines_splits_near_midpoint() {
        let text = "Ovo je jedna prilično dugačka rečenica koja mora da se podeli u dva reda.";
        let wrapped = wrap_balanced_lines(text, 42, 2);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].chars().count() <= 42);
        assert!(lines[1].chars().count() <= 42);
        let diff = (lines[0].chars().count() as isize - lines[1].chars().count() as isize).abs();
        assert!(diff <= 12, "lines should be balanced: {:?}", lines);
    }

    #[test]
    fn compute_cue_metrics_flags_high_cps_and_line_too_long() {
        let rules = SegmentRules::default(); // max_cps = 17.0, max_chars_per_line = 42
        let mut cue = SubtitleCue {
            id: 1,
            start_ms: 0,
            end_ms: 500, // 0.5s -> duration_too_short & high_cps
            source_text: "Kratko".to_string(),
            translated_text:
                "Ово је превод који је превише дугачак за једну линију од четрдесет два карактера."
                    .to_string(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut cue, &rules);
        assert!(cue.cps > 17.0);
        assert!(cue.warnings.contains(&"high_cps".to_string()));
        assert!(cue.warnings.contains(&"line_too_long".to_string()));
        assert!(cue.warnings.contains(&"duration_too_short".to_string()));
    }
}
