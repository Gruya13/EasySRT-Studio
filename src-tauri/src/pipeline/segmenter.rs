//! Word-to-cue subtitle segmentation, balanced line wrapping, and CPS/warning metrics.

use crate::models::{SegmentPreset, SegmentRules, SubtitleCue, TimedWord};

const INTER_SEGMENT_PAUSE_MS: u64 = 200;
const HARD_PAUSE_MS: u64 = 500;
const SOFT_PAUSE_THRESHOLD_MS: u64 = 150;
const SHORT_SILENCE_BRIDGE_MS: u64 = 500;
const SHORT_SILENCE_BRIDGE_REELS_MS: u64 = 350;
const POST_SPEECH_HOLD_MS: u64 = 350;
const POST_SPEECH_HOLD_REELS_MS: u64 = 250;
const MIN_PAUSE_CLEAR_GAP_MS: u64 = 200;

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

fn is_dangling_function_word(word: &str) -> bool {
    let clean = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(
        clean.as_str(),
        "the"
            | "a"
            | "an"
            | "to"
            | "of"
            | "in"
            | "on"
            | "at"
            | "by"
            | "for"
            | "with"
            | "from"
            | "into"
            | "onto"
            | "upon"
            | "without"
            | "within"
            | "between"
            | "through"
            | "during"
            | "against"
            | "among"
            | "around"
            | "behind"
            | "beyond"
            | "towards"
            | "toward"
            | "and"
            | "or"
            | "but"
            | "which"
            | "that"
            | "who"
            | "whom"
            | "whose"
            | "where"
            | "when"
            | "while"
            | "because"
            | "although"
            | "since"
            | "until"
            | "unless"
            | "before"
            | "after"
            | "has"
            | "have"
            | "had"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "we"
            | "i"
            | "you"
            | "they"
            | "he"
            | "she"
            | "this"
            | "these"
            | "those"
            | "our"
            | "my"
            | "your"
            | "his"
            | "her"
            | "their"
            | "its"
            | "not"
            | "no"
            | "u"
            | "na"
            | "za"
            | "od"
            | "do"
            | "sa"
            | "s"
            | "iz"
            | "kroz"
            | "o"
            | "po"
            | "pri"
            | "ka"
            | "k"
            | "ali"
            | "ili"
            | "da"
            | "je"
            | "su"
            | "sam"
            | "smo"
            | "ste"
            | "ću"
            | "će"
            | "bi"
            | "ne"
            | "koji"
            | "koja"
            | "koje"
            | "kojeg"
            | "kojem"
            | "kojim"
            | "čiji"
            | "čija"
            | "čije"
            | "što"
            | "jer"
            | "ako"
            | "dok"
            | "kad"
            | "kada"
            | "nego"
            | "već"
            | "niti"
            | "moj"
            | "tvoj"
            | "njegov"
            | "njen"
            | "naš"
            | "vaš"
            | "njihov"
            | "ovaj"
            | "ova"
            | "ovo"
    )
}

fn slice_char_len(words: &[TimedWord]) -> usize {
    let mut len = 0usize;
    for (idx, w) in words.iter().enumerate() {
        len += w.text.chars().count();
        if idx > 0 {
            len += 1;
        }
    }
    len
}

/// Returns `true` for relative pronouns, subordinating conjunctions, and prepositions
/// that naturally continue a clause after a brief speaker breath pause.
fn is_continuation_starter(word: &str) -> bool {
    let clean = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(
        clean.as_str(),
        "which"
            | "that"
            | "who"
            | "whom"
            | "whose"
            | "where"
            | "when"
            | "while"
            | "because"
            | "although"
            | "since"
            | "until"
            | "unless"
            | "of"
            | "to"
            | "for"
            | "with"
            | "by"
            | "from"
            | "into"
            | "onto"
            | "upon"
            | "without"
            | "within"
            | "between"
            | "through"
            | "koji"
            | "koja"
            | "koje"
            | "kojeg"
            | "kojem"
            | "kojim"
            | "čiji"
            | "čija"
            | "čije"
            | "što"
            | "jer"
            | "gdje"
            | "gde"
            | "kako"
    )
}

fn distance_to_next_natural_break(words: &[TimedWord], from_idx: usize, max_chars: usize) -> usize {
    let mut len = 0usize;
    for idx in from_idx..words.len() {
        let word_len = words[idx].text.chars().count();
        len = if idx == from_idx {
            word_len
        } else {
            len + 1 + word_len
        };
        if len > max_chars || ends_with_sentence_punct(&words[idx].text) {
            return len;
        }
        if let Some(next_word) = words.get(idx + 1) {
            let gap = next_word.start_ms.saturating_sub(words[idx].end_ms);
            if gap >= INTER_SEGMENT_PAUSE_MS
                || (ends_with_clause_punct(&words[idx].text) && gap >= 120)
            {
                return len;
            }
        }
    }
    len
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

            // Priority 2/3: Break on clause punctuation or inter-segment pauses
            // without leaving dangling function words or tiny orphan fragments.
            if let Some(next_word) = cleaned_words.get(i + 1) {
                let gap = next_word.start_ms.saturating_sub(cleaned_words[i].end_ms);
                let candidate_duration = cleaned_words[i].end_ms.saturating_sub(cue_start_ms);

                if ends_with_clause_punct(&cleaned_words[i].text)
                    && ((char_len >= 14
                        && candidate_duration >= 900
                        && gap >= INTER_SEGMENT_PAUSE_MS)
                        || gap >= 380)
                {
                    last_fit_idx = i;
                    hit_hard_limit = false;
                    break;
                }

                if !is_dangling_function_word(&cleaned_words[i].text)
                    && gap >= INTER_SEGMENT_PAUSE_MS
                {
                    let next_is_closing_word = (ends_with_clause_punct(&next_word.text)
                        || ends_with_sentence_punct(&next_word.text))
                        && (char_len + 1 + next_word.text.chars().count()) <= max_total_chars
                        && next_word.end_ms.saturating_sub(cue_start_ms) <= rules.max_duration_ms;

                    if !next_is_closing_word {
                        let next_chunk_len =
                            distance_to_next_natural_break(&cleaned_words, i + 1, max_total_chars);
                        let would_overflow_next =
                            char_len + 1 + next_chunk_len > max_total_chars;
                        let words_in_cue = i - start_idx + 1;
                        let single_short = words_in_cue == 1 && char_len < 8;
                        let next_is_cont = is_continuation_starter(&next_word.text);

                        if would_overflow_next
                            || (!single_short
                                && !(next_is_cont && gap < 1100)
                                && ((char_len >= 26 && candidate_duration >= 1200)
                                    || gap >= HARD_PAUSE_MS))
                        {
                            last_fit_idx = i;
                            hit_hard_limit = false;
                            break;
                        }
                    }
                }
            }
        }

        // If we stopped because adding the next word would exceed max chars or max duration,
        // look back within start_idx..=last_fit_idx for a balanced prepositional/clause break
        // so we don't strand a 1-2 word tail like "of an accident." on the next cue.
        let mut end_idx = last_fit_idx;
        if hit_hard_limit && last_fit_idx > start_idx {
            let rem = distance_to_next_natural_break(
                &cleaned_words,
                last_fit_idx + 1,
                max_total_chars,
            );
            let balanced_mid_split = if rem < 24 && max_total_chars >= 36 {
                let target_half = (char_len + rem) / 2;
                let mut best_idx: Option<usize> = None;
                let mut best_score = isize::MAX;
                for idx in start_idx..last_fit_idx {
                    if is_dangling_function_word(&cleaned_words[idx].text) {
                        continue;
                    }
                    let prefix_len = slice_char_len(&cleaned_words[start_idx..=idx]);
                    if prefix_len < 24 {
                        continue;
                    }
                    let bonus = if is_continuation_starter(&cleaned_words[idx + 1].text) {
                        10
                    } else {
                        0
                    };
                    let score = (prefix_len as isize - target_half as isize).abs() - bonus;
                    if score < best_score {
                        best_score = score;
                        best_idx = Some(idx);
                    }
                }
                best_idx
            } else {
                None
            };

            if let Some(mid_idx) = balanced_mid_split {
                end_idx = mid_idx;
            } else {
                let min_prefix_chars = if max_total_chars >= 36 {
                    (max_total_chars / 3).max(18)
                } else {
                    (max_total_chars / 3).max(1)
                };

                if let Some(clause_idx) = (start_idx..=last_fit_idx).rev().find(|&idx| {
                    ends_with_clause_punct(&cleaned_words[idx].text)
                        && !is_dangling_function_word(&cleaned_words[idx].text)
                        && slice_char_len(&cleaned_words[start_idx..=idx]) >= min_prefix_chars
                }) {
                    end_idx = clause_idx;
                } else if let Some(pause_idx) = (start_idx..last_fit_idx).rev().find(|&idx| {
                    cleaned_words[idx + 1]
                        .start_ms
                        .saturating_sub(cleaned_words[idx].end_ms)
                        >= SOFT_PAUSE_THRESHOLD_MS
                        && !is_dangling_function_word(&cleaned_words[idx].text)
                        && slice_char_len(&cleaned_words[start_idx..=idx]) >= min_prefix_chars
                }) {
                    end_idx = pause_idx;
                } else if is_dangling_function_word(&cleaned_words[last_fit_idx].text) {
                    if let Some(non_dangling) = (start_idx..last_fit_idx)
                        .rev()
                        .take(4)
                        .find(|&idx| !is_dangling_function_word(&cleaned_words[idx].text))
                    {
                        end_idx = non_dangling;
                    }
                }
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
        let speech_end_ms = cleaned_words[end_idx].end_ms;
        let max_allowed_end = start_ms
            .saturating_add(rules.max_duration_ms)
            .max(speech_end_ms);
        let desired_min_end = start_ms.saturating_add(rules.min_duration_ms);
        let (bridge_threshold_ms, hold_ms) = match rules.preset {
            SegmentPreset::Reels => (SHORT_SILENCE_BRIDGE_REELS_MS, POST_SPEECH_HOLD_REELS_MS),
            SegmentPreset::Standard | SegmentPreset::Custom => {
                (SHORT_SILENCE_BRIDGE_MS, POST_SPEECH_HOLD_MS)
            }
        };
        let desired_comfort_end = speech_end_ms
            .saturating_add(hold_ms)
            .max(desired_min_end)
            .min(max_allowed_end);

        let end_ms = if let Some(next_word) = cleaned_words.get(end_idx + 1) {
            let silence_gap_ms = next_word.start_ms.saturating_sub(speech_end_ms);
            if silence_gap_ms <= bridge_threshold_ms {
                // Short silence: bridge the end of this cue directly to the start of the next cue
                next_word.start_ms.min(max_allowed_end).max(speech_end_ms)
            } else {
                // Longer pause: extend by post-speech comfort hold / min_duration, then leave a clear blank gap before the next cue
                let safe_cap = next_word
                    .start_ms
                    .saturating_sub(MIN_PAUSE_CLEAR_GAP_MS)
                    .max(speech_end_ms);
                desired_comfort_end.min(safe_cap).max(speech_end_ms)
            }
        } else {
            desired_comfort_end
        };

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
        // Final cue gets 350ms post-speech hold (2200 + 350 = 2550)
        assert_eq!(cues[1].end_ms, 2550);
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

    #[test]
    fn avoids_splitting_after_dangling_function_words() {
        let words = vec![
            word(0, 300, "Villages"),
            word(300, 600, "were"),
            word(600, 900, "forced"),
            word(900, 1100, "to"),
            // 420ms BPE timestamp jitter right after "to"
            word(1520, 1900, "fly"),
            word(1900, 2300, "away."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].source_text, "Villages were forced to fly away.");
    }

    #[test]
    fn avoids_micro_orphan_prefix_on_hard_limit_lookback_and_dangling_relatives() {
        let words = vec![
            word(0, 400, "This"),
            word(400, 800, "report,"),
            word(960, 1300, "which"),
            word(1300, 1700, "explains"),
            word(1700, 2100, "everything"),
            word(2100, 2600, "about"),
            word(2600, 3000, "the"),
            word(3000, 3500, "situation"),
            word(3500, 4000, "clearly"),
            word(4000, 4500, "today"),
            word(4500, 5000, "should"),
            word(5000, 5400, "help"),
            word(5400, 5800, "us"),
            word(5800, 6200, "decide"),
            word(6200, 6600, "soon."),
        ];
        let rules = SegmentRules::default(); // max_total_chars = 84
        let cues = segment_words(&words, &rules);
        assert!(cues.len() >= 2);
        // Must NOT split into tiny "This report," (12 chars < min_prefix_chars)
        assert!(
            cues[0].source_text.replace('\n', " ").chars().count() >= 28,
            "first cue should not be an orphan fragment: {:?}",
            cues[0].source_text
        );
        // Must NOT leave relative pronoun "which" dangling at end of a cue
        for cue in &cues {
            let last_word = cue.source_text.split_whitespace().last().unwrap_or("");
            assert!(
                !last_word.eq_ignore_ascii_case("which"),
                "cue ended with dangling relative pronoun 'which': {:?}",
                cue.source_text
            );
        }
    }

    #[test]
    fn bridges_short_silence_and_adds_comfort_hold_before_longer_pause() {
        let words = vec![
            // Cue 1: 0..1400ms, followed by 320ms silence (<= 500ms -> should bridge to 1720ms)
            word(0, 700, "Prva"),
            word(700, 1400, "rečenica."),
            // Cue 2: 1720..3200ms, followed by 1100ms pause (> 500ms -> should hold +350ms to 3550ms)
            word(1720, 2400, "Druga"),
            word(2400, 3200, "rečenica."),
            // Cue 3: 4300..5600ms (final cue -> should hold +350ms to 5950ms)
            word(4300, 4900, "Treća"),
            word(4900, 5600, "rečenica."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].end_ms, 1720);
        assert_eq!(cues[1].start_ms, 1720);
        assert_eq!(cues[1].end_ms, 3550);
        assert_eq!(cues[2].start_ms, 4300);
        assert_eq!(cues[2].end_ms, 5950);
    }
}
