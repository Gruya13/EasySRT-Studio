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
const COMMA_EARLY_SPLIT_MIN_PREFIX_CHARS: usize = 22;
const COMMA_EARLY_SPLIT_MIN_REM_CHARS: usize = 12;
const CLAUSE_PUNCT_SPLIT_BONUS: isize = 24;
const LEADING_CONJ_HESITATION_WINDOW_MS: u64 = 2_200;
const LEADING_CONJ_TIGHTEN_GAP_MS: u64 = 600;
const LEADING_CONJ_MAX_PRE_HOLD_MS: u64 = 350;

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
            split_on_speaker_change: rules.split_on_speaker_change,
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
            split_on_speaker_change: rules.split_on_speaker_change,
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
            split_on_speaker_change: rules.split_on_speaker_change,
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
            | "so"
            | "well"
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
            | "pa"
            | "te"
            | "и"
            | "а"
            | "али"
            | "или"
            | "да"
            | "па"
            | "те"
            | "јер"
            | "него"
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

/// Recognizes single-word Serbian and English sentence-opening conjunctions/fillers
/// that Whisper often isolates with spurious punctuation during brief hesitations.
/// Explicit emphatic exclamations (`"Da!"`, `"Ne!"`, `"Stop!"`) and standalone questions
/// (`"Da?"`, `"I?"`) are excluded so they remain standalone cues.
fn is_leading_conjunction_or_filler(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let without_closing_quotes = trimmed.trim_end_matches(['"', '\'', '”', '’', '»', ')', ']']);
    if without_closing_quotes.ends_with('!') || without_closing_quotes.ends_with('?') {
        return false;
    }
    if trimmed.split_whitespace().count() != 1 {
        return false;
    }
    let clean = trimmed
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(
        clean.as_str(),
        "i" | "a"
            | "ali"
            | "pa"
            | "nego"
            | "te"
            | "jer"
            | "da"
            | "и"
            | "а"
            | "али"
            | "па"
            | "него"
            | "те"
            | "јер"
            | "да"
            | "and"
            | "but"
            | "so"
            | "or"
            | "well"
    )
}

fn starts_with_lowercase_alpha(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| c.is_lowercase())
}

fn is_sentence_capable_filler(word: &str) -> bool {
    let core = word
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(core.as_str(), "da" | "да" | "well")
}

/// Returns `true` if `conj` is a leading conjunction/filler that should stay attached
/// to `next_text` across a hesitation pause.
///
/// Standalone sentence-capable fillers ending in a single period (e.g. `"Da."`, `"Well."`)
/// followed by a non-lowercase new sentence (e.g. `"Idemo sutra."`) are genuine complete
/// utterances (`"Yes."`) and stay standalone, whereas `"Da."` + lowercase `"vidimo..."`,
/// `"Da..."` + continuation, and `"I."` + `"Zatim..."` still attach.
fn should_attach_leading_conjunction(conj: &str, next_text: &str) -> bool {
    if !is_leading_conjunction_or_filler(conj) {
        return false;
    }
    if is_sentence_capable_filler(conj) {
        let without_closing_quotes = conj
            .trim()
            .trim_end_matches(['"', '\'', '”', '’', '»', ')', ']']);
        let ends_with_single_period =
            without_closing_quotes.ends_with('.') && !without_closing_quotes.ends_with("..");
        if ends_with_single_period && !starts_with_lowercase_alpha(next_text) {
            return false;
        }
    }
    true
}

/// Strips spurious Whisper trailing `.`, `,`, `...`, `…` from a leading conjunction
/// when attaching it to a continuing clause, preserving casing on the conjunction itself.
/// For sentence-capable fillers (`"da"`, `"да"`, `"well"`), trailing commas are always
/// preserved so `"Da, idemo sutra."` and `"Well, we left."` retain their meaning, while
/// spurious `.`/`…` before a lowercase continuation (`"Da."` + `"vidimo..."`) are stripped.
fn clean_attached_leading_conjunction(conj: &str, next_text: &str) -> String {
    let trimmed = conj.trim();
    let next_is_lower = starts_with_lowercase_alpha(next_text);
    let sentence_capable = is_sentence_capable_filler(trimmed);
    let has_sentence_break = trimmed.ends_with('.') || trimmed.ends_with('…');
    let has_clause_break = trimmed.ends_with(',');

    let should_strip = if sentence_capable {
        next_is_lower && has_sentence_break
    } else {
        has_sentence_break || has_clause_break
    };

    if should_strip {
        let stripped = trimmed.trim_end_matches(['.', ',', '…']);
        if !stripped.is_empty() {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

/// Computes a safe start timestamp when a leading conjunction is attached across a
/// hesitation pause so long pauses (> 600 ms) don't hold the merged subtitle silently
/// on screen before the continuing clause begins.
fn tighten_leading_conjunction_start(
    conj_start_ms: u64,
    conj_speech_end_ms: u64,
    next_start_ms: u64,
) -> u64 {
    let speech_gap_ms = next_start_ms.saturating_sub(conj_speech_end_ms);
    let tightened = if speech_gap_ms <= LEADING_CONJ_TIGHTEN_GAP_MS {
        conj_start_ms
    } else {
        let conj_dur = conj_speech_end_ms.saturating_sub(conj_start_ms);
        conj_start_ms.max(
            next_start_ms.saturating_sub(conj_dur.saturating_add(LEADING_CONJ_MAX_PRE_HOLD_MS)),
        )
    };
    tightened.min(next_start_ms)
}

/// Measures remaining characters starting at `from_idx` (inclusive, including single spaces
/// between words) until the next word ending with sentence punctuation (`.`, `!`, `?`, `…`)
/// or end of slice, stopping once `len > cap`.
fn distance_to_next_sentence_end(words: &[TimedWord], from_idx: usize, cap: usize) -> usize {
    let mut len = 0usize;
    for (idx, word) in words.iter().enumerate().skip(from_idx) {
        let word_len = word.text.chars().count();
        len = if idx == from_idx {
            word_len
        } else {
            len + 1 + word_len
        };
        if len > cap || ends_with_sentence_punct(&word.text) {
            return len;
        }
    }
    len
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

#[derive(Debug, Clone)]
struct CueWithMeta {
    cue: SubtitleCue,
    speech_end_ms: u64,
    starts_new_speaker: bool,
}

fn try_merge_leading_conjunction_cue(
    curr: &CueWithMeta,
    next: &mut CueWithMeta,
    rules: &SegmentRules,
    max_total_chars: usize,
) -> bool {
    if next.starts_new_speaker {
        return false;
    }
    let speech_gap_ms = next.cue.start_ms.saturating_sub(curr.speech_end_ms);
    if speech_gap_ms > LEADING_CONJ_HESITATION_WINDOW_MS {
        return false;
    }

    let next_flat = next
        .cue
        .source_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if next_flat.is_empty() {
        return false;
    }
    if !should_attach_leading_conjunction(&curr.cue.source_text, &next_flat) {
        return false;
    }

    let cleaned_conj = clean_attached_leading_conjunction(&curr.cue.source_text, &next_flat);
    let merged_flat = format!("{cleaned_conj} {next_flat}");
    if merged_flat.chars().count() > max_total_chars {
        return false;
    }

    let merged_start =
        tighten_leading_conjunction_start(curr.cue.start_ms, curr.speech_end_ms, next.cue.start_ms);
    if next.speech_end_ms.saturating_sub(merged_start) > rules.max_duration_ms {
        return false;
    }
    let merged_end = next
        .cue
        .end_ms
        .min(merged_start.saturating_add(rules.max_duration_ms))
        .max(next.speech_end_ms);

    next.cue.start_ms = merged_start;
    next.cue.end_ms = merged_end;
    next.cue.source_text =
        wrap_balanced_lines(&merged_flat, rules.max_chars_per_line, rules.max_lines);
    compute_cue_metrics(&mut next.cue, rules);
    next.starts_new_speaker = curr.starts_new_speaker;
    true
}

/// Post-pass safety net: merges any remaining isolated single-word leading conjunction cue
/// into the immediately following same-speaker cue when within the hesitation window and
/// character/duration limits.
fn merge_isolated_leading_conjunction_cues(
    items: Vec<CueWithMeta>,
    rules: &SegmentRules,
    max_total_chars: usize,
) -> Vec<SubtitleCue> {
    let mut merged_rev: Vec<CueWithMeta> = Vec::with_capacity(items.len());
    for item in items.into_iter().rev() {
        let merged = if let Some(next) = merged_rev.last_mut() {
            try_merge_leading_conjunction_cue(&item, next, rules, max_total_chars)
        } else {
            false
        };
        if !merged {
            merged_rev.push(item);
        }
    }
    merged_rev.reverse();

    let mut cues: Vec<SubtitleCue> = merged_rev.into_iter().map(|m| m.cue).collect();
    for (idx, cue) in cues.iter_mut().enumerate() {
        cue.id = (idx + 1) as u32;
    }
    cues
}

/// Groups `TimedWord`s into readable `SubtitleCue`s according to `rules`.
pub fn segment_words(words: &[TimedWord], rules: &SegmentRules) -> Vec<SubtitleCue> {
    let speaker_change_indices: Vec<usize> = words
        .iter()
        .enumerate()
        .filter_map(|(idx, word)| word.speaker_change.then_some(idx))
        .collect();
    segment_words_with_speaker_changes(
        words,
        rules,
        &speaker_change_indices,
        rules.split_on_speaker_change,
    )
}

/// Groups `TimedWord`s into readable `SubtitleCue`s according to `rules`, optionally
/// forcing immediate cue breaks at `speaker_change_indices` (0-based indices into `words`).
pub fn segment_words_with_speaker_changes(
    words: &[TimedWord],
    rules: &SegmentRules,
    speaker_change_indices: &[usize],
    split_on_speaker_change: bool,
) -> Vec<SubtitleCue> {
    let rules = normalize_rules(rules);

    let mut raw_speaker_changes = vec![false; words.len()];
    if split_on_speaker_change {
        for &idx in speaker_change_indices {
            if idx < raw_speaker_changes.len() {
                raw_speaker_changes[idx] = true;
            }
        }
    }

    let mut cleaned_words = Vec::with_capacity(words.len());
    let mut effective_speaker_breaks = Vec::with_capacity(words.len());
    let mut pending_speaker_change = false;
    for (orig_idx, w) in words.iter().enumerate() {
        pending_speaker_change |= raw_speaker_changes[orig_idx];
        let t = w.text.trim();
        if !t.is_empty() {
            let mut cleaned = w.clone();
            cleaned.end_ms = cleaned.end_ms.max(cleaned.start_ms);
            cleaned.text = t.to_string();
            cleaned_words.push(cleaned);
            effective_speaker_breaks.push(pending_speaker_change);
            pending_speaker_change = false;
        }
    }

    if cleaned_words.is_empty() {
        return Vec::new();
    }
    effective_speaker_breaks[0] = false;

    let max_total_chars = (rules.max_chars_per_line.max(1) * rules.max_lines.max(1)) as usize;
    let mut cues_with_meta: Vec<CueWithMeta> = Vec::new();
    let mut start_idx = 0;

    while start_idx < cleaned_words.len() {
        // Bound the current cue candidate strictly to the active speaker's turn so
        // lookahead, leading-conjunction attachment, and hard-limit lookback never
        // cross a speaker-change boundary.
        let turn_end = ((start_idx + 1)..cleaned_words.len())
            .find(|&idx| effective_speaker_breaks[idx])
            .unwrap_or(cleaned_words.len());
        let turn_words = &cleaned_words[..turn_end];

        let mut cue_start_ms = turn_words[start_idx].start_ms;
        let mut last_fit_idx = start_idx;
        let mut char_len = turn_words[start_idx].text.chars().count();
        let mut hit_hard_limit = false;
        let mut attached_leading_conj: Option<String> = None;

        for i in start_idx..turn_end {
            if i > start_idx {
                let added_chars = 1 + turn_words[i].text.chars().count();
                let candidate_chars = char_len + added_chars;
                let candidate_duration = turn_words[i].end_ms.saturating_sub(cue_start_ms);

                if candidate_chars > max_total_chars || candidate_duration > rules.max_duration_ms {
                    hit_hard_limit = true;
                    break;
                }
                char_len = candidate_chars;
                last_fit_idx = i;
            }

            let next_word = turn_words.get(i + 1);
            let gap = next_word.map_or(0, |nw| nw.start_ms.saturating_sub(turn_words[i].end_ms));

            // Workstream 2: Suppress premature break when cue consists solely of one
            // leading conjunction/filler followed within the hesitation window by a
            // same-speaker continuation.
            if i == start_idx && is_leading_conjunction_or_filler(&turn_words[i].text) {
                if let Some(nw) = next_word {
                    if should_attach_leading_conjunction(&turn_words[i].text, &nw.text) {
                        if gap <= LEADING_CONJ_HESITATION_WINDOW_MS {
                            let cleaned_conj =
                                clean_attached_leading_conjunction(&turn_words[i].text, &nw.text);
                            char_len = cleaned_conj.chars().count();
                            cue_start_ms = tighten_leading_conjunction_start(
                                turn_words[i].start_ms,
                                turn_words[i].end_ms,
                                nw.start_ms,
                            );
                            attached_leading_conj = Some(cleaned_conj);
                            continue;
                        }
                        // Pause exceeds hesitation window (> 2.2s): keep standalone.
                        last_fit_idx = i;
                        hit_hard_limit = false;
                        break;
                    }
                } else {
                    // End of speaker turn or transcript.
                    last_fit_idx = i;
                    hit_hard_limit = false;
                    break;
                }
            }

            // Priority 1: Break immediately on sentence-ending punctuation
            if ends_with_sentence_punct(&turn_words[i].text) {
                last_fit_idx = i;
                hit_hard_limit = false;
                break;
            }

            // Priority 2/3: Break on clause punctuation or inter-segment pauses
            // without leaving dangling function words or tiny orphan fragments.
            if let Some(next_word) = next_word {
                let candidate_duration = turn_words[i].end_ms.saturating_sub(cue_start_ms);

                if ends_with_clause_punct(&turn_words[i].text)
                    && !is_dangling_function_word(&turn_words[i].text)
                {
                    let rem_sentence = distance_to_next_sentence_end(
                        turn_words,
                        i + 1,
                        max_total_chars.max(COMMA_EARLY_SPLIT_MIN_REM_CHARS),
                    );
                    let long_sentence_comma_split = char_len + 1 + rem_sentence > max_total_chars
                        && char_len >= COMMA_EARLY_SPLIT_MIN_PREFIX_CHARS
                        && rem_sentence >= COMMA_EARLY_SPLIT_MIN_REM_CHARS;

                    if long_sentence_comma_split
                        || (char_len >= 14
                            && candidate_duration >= 900
                            && gap >= INTER_SEGMENT_PAUSE_MS)
                        || gap >= 380
                    {
                        last_fit_idx = i;
                        hit_hard_limit = false;
                        break;
                    }
                }

                if !is_dangling_function_word(&turn_words[i].text)
                    && gap >= INTER_SEGMENT_PAUSE_MS
                {
                    let next_is_closing_word = (ends_with_clause_punct(&next_word.text)
                        || ends_with_sentence_punct(&next_word.text))
                        && (char_len + 1 + next_word.text.chars().count()) <= max_total_chars
                        && next_word.end_ms.saturating_sub(cue_start_ms) <= rules.max_duration_ms;

                    if !next_is_closing_word {
                        let next_chunk_len =
                            distance_to_next_natural_break(turn_words, i + 1, max_total_chars);
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
            let rem =
                distance_to_next_natural_break(turn_words, last_fit_idx + 1, max_total_chars);
            let balanced_mid_split = if rem < 24 && max_total_chars >= 36 {
                let target_half = (char_len + rem) / 2;
                let mut best_idx: Option<usize> = None;
                let mut best_score = isize::MAX;
                let search_end = if ends_with_clause_punct(&turn_words[last_fit_idx].text)
                    && rem >= COMMA_EARLY_SPLIT_MIN_REM_CHARS
                {
                    last_fit_idx
                } else {
                    last_fit_idx - 1
                };
                for idx in start_idx..=search_end {
                    if is_dangling_function_word(&turn_words[idx].text) {
                        continue;
                    }
                    let prefix_len = slice_char_len(&turn_words[start_idx..=idx]);
                    if prefix_len < 24 {
                        continue;
                    }
                    let tail_len = if idx == last_fit_idx {
                        rem
                    } else {
                        slice_char_len(&turn_words[(idx + 1)..=last_fit_idx]) + 1 + rem
                    };
                    let clause_bonus = if ends_with_clause_punct(&turn_words[idx].text)
                        && tail_len >= COMMA_EARLY_SPLIT_MIN_REM_CHARS
                    {
                        CLAUSE_PUNCT_SPLIT_BONUS
                    } else {
                        0
                    };
                    let cont_bonus = if turn_words
                        .get(idx + 1)
                        .is_some_and(|w| is_continuation_starter(&w.text))
                    {
                        10
                    } else {
                        0
                    };
                    let bonus = clause_bonus + cont_bonus;
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
                    ends_with_clause_punct(&turn_words[idx].text)
                        && !is_dangling_function_word(&turn_words[idx].text)
                        && slice_char_len(&turn_words[start_idx..=idx]) >= min_prefix_chars
                }) {
                    end_idx = clause_idx;
                } else if let Some(pause_idx) = (start_idx..last_fit_idx).rev().find(|&idx| {
                    turn_words[idx + 1]
                        .start_ms
                        .saturating_sub(turn_words[idx].end_ms)
                        >= SOFT_PAUSE_THRESHOLD_MS
                        && !is_dangling_function_word(&turn_words[idx].text)
                        && slice_char_len(&turn_words[start_idx..=idx]) >= min_prefix_chars
                }) {
                    end_idx = pause_idx;
                } else if is_dangling_function_word(&turn_words[last_fit_idx].text) {
                    if let Some(non_dangling) = (start_idx..last_fit_idx)
                        .rev()
                        .take(4)
                        .find(|&idx| !is_dangling_function_word(&turn_words[idx].text))
                    {
                        end_idx = non_dangling;
                    }
                }
            }
        }

        let raw_text = if end_idx > start_idx {
            if let Some(ref cleaned_conj) = attached_leading_conj {
                let mut parts = Vec::with_capacity(end_idx - start_idx + 1);
                parts.push(cleaned_conj.as_str());
                parts.extend(
                    cleaned_words[(start_idx + 1)..=end_idx]
                        .iter()
                        .map(|w| w.text.as_str()),
                );
                parts.join(" ")
            } else {
                cleaned_words[start_idx..=end_idx]
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        } else {
            cleaned_words[start_idx].text.clone()
        };

        let wrapped_text =
            wrap_balanced_lines(&raw_text, rules.max_chars_per_line, rules.max_lines);

        let start_ms = if end_idx > start_idx && attached_leading_conj.is_some() {
            cue_start_ms
        } else {
            cleaned_words[start_idx].start_ms
        };
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
            id: (cues_with_meta.len() + 1) as u32,
            start_ms,
            end_ms,
            source_text: wrapped_text,
            translated_text: String::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut cue, &rules);
        cues_with_meta.push(CueWithMeta {
            cue,
            speech_end_ms,
            starts_new_speaker: effective_speaker_breaks[start_idx],
        });

        start_idx = end_idx + 1;
    }

    merge_isolated_leading_conjunction_cues(cues_with_meta, &rules, max_total_chars)
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
            speaker_change: false,
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
            split_on_speaker_change: true,
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

    fn continuous_words(sentence: &str, start_ms: u64, step_ms: u64) -> Vec<TimedWord> {
        sentence
            .split_whitespace()
            .enumerate()
            .map(|(i, token)| {
                let s = start_ms + (i as u64) * step_ms;
                word(s, s + step_ms, token)
            })
            .collect()
    }

    #[test]
    fn splits_long_continuous_sentence_right_after_comma_without_pauses() {
        // 102 chars total (> 84 max_total_chars), zero pauses between words.
        // Prefix before comma is 39 chars (>= 22), remainder after comma is 62 chars (>= 12).
        let sentence = "Kada smo stigli na glavni ulaz stadiona, odmah smo primetili veliku gužvu ispred samih kapija večeras.";
        assert!(sentence.chars().count() > 90);
        let words = continuous_words(sentence, 0, 250);
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 2, "expected clean 2-cue split at comma: {:?}", cues);
        let flat0 = cues[0].source_text.replace('\n', " ");
        let flat1 = cues[1].source_text.replace('\n', " ");
        assert_eq!(flat0, "Kada smo stigli na glavni ulaz stadiona,");
        assert_eq!(
            flat1,
            "odmah smo primetili veliku gužvu ispred samih kapija večeras."
        );
    }

    #[test]
    fn keeps_short_sentence_with_comma_intact_in_single_cue() {
        // 51 chars (<= 60), contains a comma with zero pause.
        let sentence = "Kada dođeš kući, javi mi se odmah porukom na telefon.";
        assert!(sentence.chars().count() <= 60);
        let words = continuous_words(sentence, 0, 250);
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 1, "short sentence should stay intact: {:?}", cues);
        assert_eq!(cues[0].source_text.replace('\n', " "), sentence);
    }

    #[test]
    fn does_not_strand_short_one_word_tail_after_comma_on_overflow() {
        // 90 chars total (> 84), comma appears near the end leaving only "danas." (6 chars < 12).
        let sentence = "Pripremili smo detaljan izveštaj za sve članove tima koji učestvuju u novom projektu, danas.";
        assert!(sentence.chars().count() > 84);
        let words = continuous_words(sentence, 0, 250);
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 2, "expected balanced 2-cue split: {:?}", cues);
        let tail = cues[1].source_text.replace('\n', " ");
        assert_ne!(tail, "danas.", "must not strand 1-word tail after comma");
        assert!(
            tail.chars().count() >= 12,
            "second cue should be substantive, got: {:?}",
            tail
        );
    }

    #[test]
    fn balanced_mid_split_prefers_comma_on_duration_hard_limit() {
        // 74 chars (<= 84 so forward comma-overflow split does not fire), but slow speech
        // exceeds max_duration_ms (7000 ms), forcing balanced_mid_split lookback.
        // Comma is near midpoint and should win via +24 clause bonus.
        let sentence = "Nakon dugog razgovora sa svim partnerima, odlučili smo da nastavimo sutra.";
        assert!(sentence.chars().count() <= 84);
        let words = continuous_words(sentence, 0, 700); // 11 words * 700ms = 7700ms > 7000ms
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 2, "expected comma-preferred split: {:?}", cues);
        assert_eq!(
            cues[0].source_text.replace('\n', " "),
            "Nakon dugog razgovora sa svim partnerima,"
        );
        assert_eq!(
            cues[1].source_text.replace('\n', " "),
            "odlučili smo da nastavimo sutra."
        );
    }

    #[test]
    fn attaches_leading_conjunction_with_comma_across_pause() {
        let words = vec![
            word(0, 200, "I,"),
            // 600 ms hesitation pause
            word(800, 1100, "onda"),
            word(1100, 1400, "smo"),
            word(1400, 1800, "otišli."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 1, "leading 'I,' should attach: {:?}", cues);
        assert_eq!(cues[0].source_text, "I onda smo otišli.");
        assert_eq!(cues[0].start_ms, 0);
    }

    #[test]
    fn attaches_leading_conjunction_with_spurious_period_across_pause() {
        let words = vec![
            word(0, 200, "I."),
            // 500 ms hesitation pause
            word(700, 1000, "zatim"),
            word(1000, 1200, "je"),
            word(1200, 1500, "počeo"),
            word(1500, 1900, "film."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 1, "leading 'I.' should attach: {:?}", cues);
        assert_eq!(cues[0].source_text, "I zatim je počeo film.");
    }

    #[test]
    fn tightens_start_when_leading_conjunction_has_long_hesitation_pause() {
        let words = vec![
            word(0, 200, "I..."),
            // 1600 ms hesitation pause (<= 2200 ms, > 600 ms)
            word(1800, 2100, "onda"),
            word(2100, 2400, "smo"),
            word(2400, 2800, "otišli."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].source_text, "I onda smo otišli.");
        // Tightened from 0ms to 1800 - (200 + 350) = 1250ms so cue doesn't sit silently for 1.6s
        assert_eq!(cues[0].start_ms, 1250);
    }

    #[test]
    fn keeps_exclamations_and_long_pause_conjunctions_standalone() {
        let rules = SegmentRules::default();

        // Explicit emphatic exclamation "Da!" stays standalone even with short 500 ms pause.
        let excl_da = vec![
            word(0, 300, "Da!"),
            word(800, 1200, "Idemo"),
            word(1200, 1600, "odmah."),
        ];
        let cues_da = segment_words(&excl_da, &rules);
        assert_eq!(cues_da.len(), 2, "'Da!' must stay standalone: {:?}", cues_da);
        assert_eq!(cues_da[0].source_text, "Da!");
        assert_eq!(cues_da[1].source_text, "Idemo odmah.");

        // Explicit exclamation "Ne!" stays standalone.
        let excl_ne = vec![
            word(0, 300, "Ne!"),
            word(700, 1100, "Nema"),
            word(1100, 1500, "šanse."),
        ];
        let cues_ne = segment_words(&excl_ne, &rules);
        assert_eq!(cues_ne.len(), 2, "'Ne!' must stay standalone: {:?}", cues_ne);
        assert_eq!(cues_ne[0].source_text, "Ne!");

        // Conjunction followed by > 2.5 s pause (with period) stays standalone.
        let long_pause_period = vec![
            word(0, 200, "I."),
            word(2800, 3100, "onda"),
            word(3100, 3400, "smo"),
            word(3400, 3800, "otišli."),
        ];
        let cues_lp1 = segment_words(&long_pause_period, &rules);
        assert_eq!(
            cues_lp1.len(),
            2,
            "'I.' + 2.6s pause must stay standalone: {:?}",
            cues_lp1
        );
        assert_eq!(cues_lp1[0].source_text, "I.");
        assert_eq!(cues_lp1[1].source_text, "onda smo otišli.");

        // Conjunction followed by > 2.5 s pause (with comma) also stays standalone.
        let long_pause_comma = vec![
            word(0, 200, "I,"),
            word(2900, 3200, "onda"),
            word(3200, 3500, "smo"),
            word(3500, 3900, "otišli."),
        ];
        let cues_lp2 = segment_words(&long_pause_comma, &rules);
        assert_eq!(
            cues_lp2.len(),
            2,
            "'I,' + 2.7s pause must stay standalone: {:?}",
            cues_lp2
        );
        assert_eq!(cues_lp2[0].source_text, "I,");
        assert_eq!(cues_lp2[1].source_text, "onda smo otišli.");
    }

    #[test]
    fn post_pass_merges_isolated_leading_conjunction_and_respects_guards() {
        let rules = normalize_rules(&SegmentRules::default());
        let max_total_chars = (rules.max_chars_per_line * rules.max_lines) as usize;

        let make_meta = |id: u32, start_ms: u64, speech_end_ms: u64, end_ms: u64, text: &str, sb: bool| {
            let mut cue = SubtitleCue {
                id,
                start_ms,
                end_ms,
                source_text: text.to_string(),
                translated_text: String::new(),
                cps: 0.0,
                warnings: Vec::new(),
            };
            compute_cue_metrics(&mut cue, &rules);
            CueWithMeta {
                cue,
                speech_end_ms,
                starts_new_speaker: sb,
            }
        };

        // 1. Isolated "I," + 1500ms pause -> merges with tightened start & renumbered IDs
        let merged = merge_isolated_leading_conjunction_cues(
            vec![
                make_meta(1, 0, 200, 1000, "I,", false),
                make_meta(2, 1700, 2600, 2950, "onda smo krenuli kući.", false),
            ],
            &rules,
            max_total_chars,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, 1);
        assert_eq!(merged[0].source_text, "I onda smo krenuli kući.");
        // Tightened from 0ms to 1700 - (200 + 350) = 1150ms
        assert_eq!(merged[0].start_ms, 1150);
        assert_eq!(merged[0].end_ms, 2950);

        // 2. Speaker change between k and k + 1 prevents merge
        let blocked_speaker = merge_isolated_leading_conjunction_cues(
            vec![
                make_meta(1, 0, 200, 600, "I,", false),
                make_meta(2, 600, 1500, 1850, "onda smo krenuli kući.", true),
            ],
            &rules,
            max_total_chars,
        );
        assert_eq!(blocked_speaker.len(), 2);
        assert_eq!(blocked_speaker[0].source_text, "I,");
        assert_eq!(blocked_speaker[1].source_text, "onda smo krenuli kući.");

        // 3. True speech gap > 2200ms prevents merge even if cue[0].end_ms was min-duration padded
        let blocked_gap = merge_isolated_leading_conjunction_cues(
            vec![
                make_meta(1, 0, 200, 1000, "I.", false),
                make_meta(2, 2600, 3500, 3850, "Onda smo krenuli.", false),
            ],
            &rules,
            max_total_chars,
        );
        assert_eq!(blocked_gap.len(), 2);
    }

    #[test]
    fn speaker_change_forces_cue_break_and_blocks_leading_conjunction_merge() {
        let rules = SegmentRules::default();

        // Two speakers with 0ms gap and no punctuation between word 1 and word 2
        let words = vec![
            word(0, 300, "Zdravo"),
            word(300, 600, "svima"),
            word(600, 900, "hvala"),
            word(900, 1200, "lepo."),
        ];

        let split_cues = segment_words_with_speaker_changes(&words, &rules, &[2], true);
        assert_eq!(split_cues.len(), 2, "must split at speaker change: {:?}", split_cues);
        assert_eq!(split_cues[0].source_text, "Zdravo svima");
        assert_eq!(split_cues[1].source_text, "hvala lepo.");
        assert!(split_cues[0].end_ms <= split_cues[1].start_ms);

        // When split_on_speaker_change == false, speaker-change marker is ignored
        let unsplit_cues = segment_words_with_speaker_changes(&words, &rules, &[2], false);
        assert_eq!(unsplit_cues.len(), 1);
        assert_eq!(unsplit_cues[0].source_text, "Zdravo svima hvala lepo.");

        // Leading conjunction "I," on Speaker A must NOT merge into Speaker B across speaker change
        let conj_across_speakers = vec![
            word(0, 200, "I,"),
            word(600, 900, "onda"),
            word(900, 1200, "smo"),
            word(1200, 1600, "otišli."),
        ];
        let separated =
            segment_words_with_speaker_changes(&conj_across_speakers, &rules, &[1], true);
        assert_eq!(
            separated.len(),
            2,
            "must not merge leading conjunction across speaker change: {:?}",
            separated
        );
        assert_eq!(separated[0].source_text, "I,");
        assert_eq!(separated[1].source_text, "onda smo otišli.");

        // With split_on_speaker_change == false, it merges normally
        let merged =
            segment_words_with_speaker_changes(&conj_across_speakers, &rules, &[1], false);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source_text, "I onda smo otišli.");
    }

    #[test]
    fn speaker_change_indices_survive_empty_word_tokens_and_bound_lookahead() {
        let rules = SegmentRules::default();

        // Empty token at index 1; speaker change marked at index 2 ("Dobro")
        let words = vec![
            word(0, 300, "Ćao"),
            word(300, 350, "   "),
            word(400, 700, "Dobro"),
            word(700, 1000, "veče."),
        ];
        let cues = segment_words_with_speaker_changes(&words, &rules, &[2], true);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].source_text, "Ćao");
        assert_eq!(cues[1].source_text, "Dobro veče.");

        // Speaker A's short sentence with a comma (45 chars) must NOT split early even when
        // Speaker B immediately follows with a long 80-char sentence.
        let mut combined = continuous_words("Kada završimo ovaj kratak test, bićemo gotovi", 0, 200);
        let speaker_b_idx = combined.len();
        let next_start = combined.last().unwrap().end_ms + 100;
        combined.extend(continuous_words(
            "Ovo je potpuno nova dugačka rečenica drugog govornika koja ima mnogo više od sedamdeset karaktera.",
            next_start,
            200,
        ));
        let turn_cues =
            segment_words_with_speaker_changes(&combined, &rules, &[speaker_b_idx], true);
        assert_eq!(
            turn_cues[0].source_text.replace('\n', " "),
            "Kada završimo ovaj kratak test, bićemo gotovi"
        );
    }

    #[test]
    fn preserves_comma_on_sentence_capable_fillers_and_keeps_standalone_da_period_separate() {
        let rules = SegmentRules::default();

        // 1. "Da," + 500ms + "idemo sutra." -> single cue preserving the comma ("Yes, we're going tomorrow.")
        let da_comma = vec![
            word(0, 250, "Da,"),
            word(750, 1100, "idemo"),
            word(1100, 1500, "sutra."),
        ];
        let cues_da_comma = segment_words(&da_comma, &rules);
        assert_eq!(
            cues_da_comma.len(),
            1,
            "'Da,' + 500ms should stay in 1 cue: {:?}",
            cues_da_comma
        );
        assert_eq!(cues_da_comma[0].source_text, "Da, idemo sutra.");

        // 2. "Well," + 500ms + "we decided to leave." -> single cue preserving comma
        let well_comma = vec![
            word(0, 250, "Well,"),
            word(750, 1000, "we"),
            word(1000, 1300, "decided"),
            word(1300, 1500, "to"),
            word(1500, 1800, "leave."),
        ];
        let cues_well_comma = segment_words(&well_comma, &rules);
        assert_eq!(cues_well_comma.len(), 1);
        assert_eq!(cues_well_comma[0].source_text, "Well, we decided to leave.");

        // 3. "Da." + 1500ms + "Idemo sutra." (capitalized new sentence) -> stays 2 distinct cues
        let da_period_upper = vec![
            word(0, 300, "Da."),
            word(1800, 2150, "Idemo"),
            word(2150, 2550, "sutra."),
        ];
        let cues_da_upper = segment_words(&da_period_upper, &rules);
        assert_eq!(
            cues_da_upper.len(),
            2,
            "'Da.' + 1500ms + capitalized 'Idemo sutra.' must stay 2 cues: {:?}",
            cues_da_upper
        );
        assert_eq!(cues_da_upper[0].source_text, "Da.");
        assert_eq!(cues_da_upper[0].start_ms, 0);
        assert_eq!(cues_da_upper[1].source_text, "Idemo sutra.");
        assert_eq!(cues_da_upper[1].start_ms, 1800);

        // 4. "Da." + 500ms + lowercase "vidimo šta ćemo." -> attaches & strips spurious period
        let da_period_lower = vec![
            word(0, 250, "Da."),
            word(750, 1100, "vidimo"),
            word(1100, 1350, "šta"),
            word(1350, 1700, "ćemo."),
        ];
        let cues_da_lower = segment_words(&da_period_lower, &rules);
        assert_eq!(cues_da_lower.len(), 1);
        assert_eq!(cues_da_lower[0].source_text, "Da vidimo šta ćemo.");

        // 5. "Da..." + 500ms + "Idemo sutra." (ellipsis hesitation, not single period) -> attaches
        let da_ellipsis = vec![
            word(0, 250, "Da..."),
            word(750, 1100, "Idemo"),
            word(1100, 1500, "sutra."),
        ];
        let cues_da_ellipsis = segment_words(&da_ellipsis, &rules);
        assert_eq!(cues_da_ellipsis.len(), 1);
        assert_eq!(cues_da_ellipsis[0].source_text, "Da... Idemo sutra.");

        // 6. Pure conjunction "I." + 500ms + capitalized "Zatim je počeo film." -> attaches & strips period
        let i_period_upper = vec![
            word(0, 200, "I."),
            word(700, 1000, "Zatim"),
            word(1000, 1200, "je"),
            word(1200, 1500, "počeo"),
            word(1500, 1900, "film."),
        ];
        let cues_i_upper = segment_words(&i_period_upper, &rules);
        assert_eq!(cues_i_upper.len(), 1);
        assert_eq!(cues_i_upper[0].source_text, "I Zatim je počeo film.");

        // 7. Post-pass merge safety net also keeps "Da." + "Idemo sutra." separate and preserves "Da," comma
        let norm = normalize_rules(&rules);
        let max_chars = (norm.max_chars_per_line * norm.max_lines) as usize;
        let mk = |id: u32, start_ms: u64, speech_end_ms: u64, end_ms: u64, text: &str| {
            let mut cue = SubtitleCue {
                id,
                start_ms,
                end_ms,
                source_text: text.to_string(),
                translated_text: String::new(),
                cps: 0.0,
                warnings: Vec::new(),
            };
            compute_cue_metrics(&mut cue, &norm);
            CueWithMeta {
                cue,
                speech_end_ms,
                starts_new_speaker: false,
            }
        };

        let kept_separate = merge_isolated_leading_conjunction_cues(
            vec![
                mk(1, 0, 300, 1000, "Da."),
                mk(2, 1800, 2550, 2900, "Idemo sutra."),
            ],
            &norm,
            max_chars,
        );
        assert_eq!(kept_separate.len(), 2);
        assert_eq!(kept_separate[0].source_text, "Da.");
        assert_eq!(kept_separate[1].source_text, "Idemo sutra.");

        let kept_comma = merge_isolated_leading_conjunction_cues(
            vec![
                mk(1, 0, 250, 750, "Da,"),
                mk(2, 750, 1500, 1850, "idemo sutra."),
            ],
            &norm,
            max_chars,
        );
        assert_eq!(kept_comma.len(), 1);
        assert_eq!(kept_comma[0].source_text, "Da, idemo sutra.");
    }

    #[test]
    fn segment_words_breaks_on_timed_word_speaker_change_and_respects_rule_toggle() {
        // Verify normalize_rules preserves split_on_speaker_change across all presets
        for preset in [
            SegmentPreset::Standard,
            SegmentPreset::Reels,
            SegmentPreset::Custom,
        ] {
            for flag in [true, false] {
                let normalized = normalize_rules(&SegmentRules {
                    preset,
                    split_on_speaker_change: flag,
                    ..SegmentRules::default()
                });
                assert_eq!(
                    normalized.split_on_speaker_change, flag,
                    "normalize_rules must preserve split_on_speaker_change={flag} for {preset:?}"
                );
            }
        }

        // Words with no sentence punctuation and a short 120ms gap (below inter-segment pause
        // threshold), so splitting occurs solely because of `TimedWord.speaker_change == true`.
        let mut words = vec![
            word(0, 300, "Mislim"),
            word(320, 620, "da"),
            word(740, 1050, "jeste"),
            word(1070, 1400, "tako."),
        ];
        words[2].speaker_change = true;

        let enabled_rules = SegmentRules {
            split_on_speaker_change: true,
            ..SegmentRules::default()
        };
        let split_cues = segment_words(&words, &enabled_rules);
        assert_eq!(
            split_cues.len(),
            2,
            "segment_words must break on TimedWord.speaker_change when enabled: {:?}",
            split_cues
        );
        assert_eq!(split_cues[0].source_text, "Mislim da");
        assert_eq!(split_cues[1].source_text, "jeste tako.");

        let disabled_rules = SegmentRules {
            split_on_speaker_change: false,
            ..SegmentRules::default()
        };
        let merged_cues = segment_words(&words, &disabled_rules);
        assert_eq!(
            merged_cues.len(),
            1,
            "segment_words must ignore TimedWord.speaker_change when disabled: {:?}",
            merged_cues
        );
        assert_eq!(merged_cues[0].source_text, "Mislim da jeste tako.");
    }
}
