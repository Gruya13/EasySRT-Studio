//! Word-to-cue subtitle segmentation, balanced line wrapping, and CPS/warning metrics.

use crate::models::{SegmentPreset, SegmentRules, SubtitleCue, TimedWord};

/// Minimum breathing gap between consecutive subtitle cues (2 frames @ 25 fps).
pub const MIN_INTER_CUE_GAP_MS: u64 = 80;

const INTER_SEGMENT_PAUSE_MS: u64 = 200;
const HARD_PAUSE_MS: u64 = 500;
const SOFT_PAUSE_THRESHOLD_MS: u64 = 150;
const SHORT_SILENCE_BRIDGE_MS: u64 = 500;
const SHORT_SILENCE_BRIDGE_REELS_MS: u64 = 350;
const POST_SPEECH_HOLD_MS: u64 = 350;
const POST_SPEECH_HOLD_REELS_MS: u64 = 250;
const MIN_PAUSE_CLEAR_GAP_MS: u64 = 200;
const MIN_GAP_PULLBACK_DURATION_MS: u64 = 300;
const MIN_GAP_SPLIT_DURATION_MS: u64 = 160;
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
    for abbr in [
        "dr.", "mr.", "sv.", "br.", "st.", "prof.", "tj.", "npr.", "др.", "мр.", "св.", "бр.",
        "проф.", "тј.", "нпр.",
    ] {
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

fn is_vocal_hesitation(word: &str) -> bool {
    let clean = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    matches!(
        clean.as_str(),
        "uh" | "um" | "erm" | "er" | "hmm" | "mm" | "uhm"
    )
}

fn is_standalone_comma(word: &str) -> bool {
    let trimmed = word.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| c == ',')
}

/// Strips unambiguous non-lexical vocal hesitations (`"uh"`, `"um"`, `"erm"`, `"er"`,
/// `"hmm"`, `"mm"`, `"uhm"`) before segmentation, propagating `speaker_change` onto the
/// next surviving word and cleaning up adjacent orphan or doubled commas.
pub fn strip_vocal_hesitations(words: &[TimedWord]) -> Vec<TimedWord> {
    let mut out: Vec<TimedWord> = Vec::with_capacity(words.len());
    let mut pending_speaker_change = false;
    let mut just_stripped = false;

    for w in words {
        let trimmed = w.text.trim();
        if trimmed.is_empty() {
            pending_speaker_change |= w.speaker_change;
            continue;
        }

        if is_vocal_hesitation(trimmed) {
            pending_speaker_change |= w.speaker_change;
            while out.last().is_some_and(|prev| is_standalone_comma(&prev.text)) {
                if let Some(popped) = out.pop() {
                    pending_speaker_change |= popped.speaker_change;
                }
            }
            if let Some(prev) = out.last_mut() {
                while prev.text.ends_with(",,") {
                    prev.text.pop();
                }
            }
            just_stripped = true;
            continue;
        }

        if is_standalone_comma(trimmed)
            && (just_stripped
                || out.is_empty()
                || out.last().is_some_and(|prev| {
                    ends_with_clause_punct(&prev.text) || ends_with_sentence_punct(&prev.text)
                }))
        {
            pending_speaker_change |= w.speaker_change;
            continue;
        }

        let mut kept = w.clone();
        kept.text = trimmed.to_string();
        if just_stripped {
            let cleaned_leading = kept.text.trim_start_matches([',', ' ', '\t']);
            if cleaned_leading.is_empty() {
                pending_speaker_change |= kept.speaker_change;
                continue;
            }
            if cleaned_leading.len() != kept.text.len() {
                kept.text = cleaned_leading.to_string();
            }
        }
        if pending_speaker_change {
            kept.speaker_change = true;
            pending_speaker_change = false;
        }
        out.push(kept);
        just_stripped = false;
    }

    out
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
            | "what"
            | "why"
            | "how"
            | "if"
            | "whether"
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
            | "šta"
            | "sta"
            | "ko"
            | "koga"
            | "kom"
            | "kome"
            | "čemu"
            | "cemu"
            | "zašto"
            | "zasto"
            | "kako"
            | "gde"
            | "gdje"
            | "kuda"
            | "odakle"
            | "otkud"
            | "koliko"
            | "zar"
            | "li"
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
            | "шта"
            | "ко"
            | "кога"
            | "ком"
            | "коме"
            | "чему"
            | "зашто"
            | "како"
            | "где"
            | "гдје"
            | "куда"
            | "одакле"
            | "откуд"
            | "колико"
            | "зар"
            | "ли"
            | "кад"
            | "када"
            | "док"
            | "ако"
            | "у"
            | "на"
            | "за"
            | "од"
            | "до"
            | "са"
            | "с"
            | "из"
            | "кроз"
            | "о"
            | "по"
            | "при"
            | "ка"
            | "к"
            | "је"
            | "су"
            | "сам"
            | "смо"
            | "сте"
            | "ћу"
            | "ће"
            | "би"
            | "не"
            | "који"
            | "која"
            | "које"
            | "којег"
            | "којем"
            | "којим"
            | "чији"
            | "чија"
            | "чије"
            | "што"
            | "већ"
            | "нити"
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

/// Enforces at least `MIN_INTER_CUE_GAP_MS` (80 ms / 2 frames @ 25 fps) between
/// consecutive cues so back-to-back subtitles remain visually distinct on screen.
pub fn enforce_min_inter_cue_gap(cues: &mut [SubtitleCue], rules: &SegmentRules) {
    if cues.len() < 2 {
        return;
    }

    for k in 0..(cues.len() - 1) {
        let curr_start = cues[k].start_ms;
        let curr_end = cues[k].end_ms;
        let next_start = cues[k + 1].start_ms;
        let next_end = cues[k + 1].end_ms;

        if next_start >= curr_end && next_start - curr_end >= MIN_INTER_CUE_GAP_MS {
            continue;
        }

        let mut curr_changed = false;
        let mut next_changed = false;

        let target_curr_end = next_start.saturating_sub(MIN_INTER_CUE_GAP_MS);
        if next_start >= MIN_INTER_CUE_GAP_MS
            && target_curr_end >= curr_start.saturating_add(MIN_GAP_PULLBACK_DURATION_MS)
        {
            if cues[k].end_ms != target_curr_end {
                cues[k].end_ms = target_curr_end;
                curr_changed = true;
            }
        } else {
            let needed = if next_start >= curr_end {
                MIN_INTER_CUE_GAP_MS - (next_start - curr_end)
            } else {
                (curr_end - next_start).saturating_add(MIN_INTER_CUE_GAP_MS)
            };

            let curr_dur = curr_end.saturating_sub(curr_start);
            let next_dur = next_end.saturating_sub(next_start);
            let left_slack = curr_dur.saturating_sub(MIN_GAP_SPLIT_DURATION_MS);
            let right_slack = next_dur.saturating_sub(MIN_GAP_SPLIT_DURATION_MS);

            let half = needed / 2;
            let mut left_cut = half.min(left_slack);
            let rem_after_left = needed.saturating_sub(left_cut);
            let right_cut = rem_after_left.min(right_slack);
            let rem_after_right = needed.saturating_sub(left_cut).saturating_sub(right_cut);
            if rem_after_right > 0 {
                left_cut = left_cut.saturating_add(rem_after_right.min(left_slack - left_cut));
            }

            if left_cut > 0 {
                cues[k].end_ms = curr_end.saturating_sub(left_cut);
                curr_changed = true;
            }
            if right_cut > 0 {
                cues[k + 1].start_ms = next_start.saturating_add(right_cut);
                next_changed = true;
            }
        }

        if curr_changed {
            compute_cue_metrics(&mut cues[k], rules);
        }
        if next_changed {
            compute_cue_metrics(&mut cues[k + 1], rules);
        }
    }
}

/// Groups `TimedWord`s into readable `SubtitleCue`s according to `rules`.
pub fn segment_words(words: &[TimedWord], rules: &SegmentRules) -> Vec<SubtitleCue> {
    let speaker_change_indices: Vec<usize> = words
        .iter()
        .enumerate()
        .filter_map(|(idx, word)| word.speaker_change.then_some(idx))
        .collect();
    segment_words_with_options(
        words,
        rules,
        &speaker_change_indices,
        rules.split_on_speaker_change,
        false,
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
    segment_words_with_options(
        words,
        rules,
        speaker_change_indices,
        split_on_speaker_change,
        false,
    )
}

/// Groups `TimedWord`s into readable `SubtitleCue`s according to `rules`, optionally
/// forcing immediate cue breaks at `speaker_change_indices` (0-based indices into `words`)
/// and optionally stripping non-lexical vocal hesitations (`remove_fillers`) before
/// mapping speaker-turn boundaries so speaker indices stay aligned.
pub fn segment_words_with_options(
    words: &[TimedWord],
    rules: &SegmentRules,
    speaker_change_indices: &[usize],
    split_on_speaker_change: bool,
    remove_fillers: bool,
) -> Vec<SubtitleCue> {
    let rules = normalize_rules(rules);

    let (working_words, raw_speaker_changes): (Vec<TimedWord>, Vec<bool>) = if remove_fillers {
        let mut tagged = words.to_vec();
        if split_on_speaker_change {
            for &idx in speaker_change_indices {
                if let Some(w) = tagged.get_mut(idx) {
                    w.speaker_change = true;
                }
            }
        } else {
            for w in &mut tagged {
                w.speaker_change = false;
            }
        }
        let stripped = strip_vocal_hesitations(&tagged);
        let breaks = if split_on_speaker_change {
            stripped.iter().map(|w| w.speaker_change).collect()
        } else {
            vec![false; stripped.len()]
        };
        (stripped, breaks)
    } else {
        let mut breaks = vec![false; words.len()];
        if split_on_speaker_change {
            for (idx, w) in words.iter().enumerate() {
                if w.speaker_change {
                    breaks[idx] = true;
                }
            }
            for &idx in speaker_change_indices {
                if idx < breaks.len() {
                    breaks[idx] = true;
                }
            }
        }
        (words.to_vec(), breaks)
    };

    let mut cleaned_words = Vec::with_capacity(working_words.len());
    let mut effective_speaker_breaks = Vec::with_capacity(working_words.len());
    let mut pending_speaker_change = false;
    for (orig_idx, w) in working_words.into_iter().enumerate() {
        pending_speaker_change |= raw_speaker_changes[orig_idx];
        let trimmed = w.text.trim().to_string();
        if !trimmed.is_empty() {
            let mut cleaned = w;
            cleaned.end_ms = cleaned.end_ms.max(cleaned.start_ms);
            cleaned.text = trimmed;
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

            // Suppress premature break when cue consists solely of one leading
            // conjunction/filler followed within the hesitation window by a
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

                // If a hard pause follows a trailing unpunctuated clause/question starter
                // immediately after sentence/clause punctuation (e.g. "Okay, alright, what" + 520ms),
                // roll back cleanly to the punctuation boundary so the starter opens the next cue.
                if is_dangling_function_word(&turn_words[i].text)
                    && gap >= HARD_PAUSE_MS
                    && i > start_idx
                    && !(i == start_idx + 1 && attached_leading_conj.is_some())
                    && (ends_with_sentence_punct(&turn_words[i - 1].text)
                        || ends_with_clause_punct(&turn_words[i - 1].text))
                    && !ends_with_sentence_punct(&turn_words[i].text)
                    && !ends_with_clause_punct(&turn_words[i].text)
                    && slice_char_len(&turn_words[start_idx..=(i - 1)]) >= 4
                {
                    last_fit_idx = i - 1;
                    hit_hard_limit = false;
                    break;
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

        // General Structural Rollback Rule: never strand a single unpunctuated word after
        // sentence/clause punctuation at the tail of a cue when speech continues in the same turn.
        // Complete closing vocatives (e.g. "Hvala ti, brate.") remain intact because the trailing
        // word carries terminal punctuation.
        if end_idx > start_idx
            && end_idx + 1 < turn_end
            && !(end_idx == start_idx + 1 && attached_leading_conj.is_some())
            && (ends_with_sentence_punct(&cleaned_words[end_idx - 1].text)
                || ends_with_clause_punct(&cleaned_words[end_idx - 1].text))
            && !ends_with_sentence_punct(&cleaned_words[end_idx].text)
            && !ends_with_clause_punct(&cleaned_words[end_idx].text)
            && slice_char_len(&cleaned_words[start_idx..=(end_idx - 1)]) >= 4
        {
            end_idx -= 1;
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
                // Short silence: bridge while keeping at least MIN_INTER_CUE_GAP_MS (80 ms) clearance
                let bridge_cap = next_word.start_ms.saturating_sub(MIN_INTER_CUE_GAP_MS);
                bridge_cap.min(max_allowed_end).max(speech_end_ms)
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
            translated_variants: std::collections::BTreeMap::new(),
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

    let mut cues =
        merge_isolated_leading_conjunction_cues(cues_with_meta, &rules, max_total_chars);
    enforce_min_inter_cue_gap(&mut cues, &rules);
    cues
}

fn normalize_wrap_token(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric())
        .replace('’', "'")
        .to_lowercase()
}

fn starts_with_uppercase_alpha(word: &str) -> bool {
    let trimmed = word.trim_start_matches(['"', '\'', '„', '“', '‘', '«', '(', '[']);
    trimmed
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() && c.is_uppercase())
}

fn is_wrap_negation(clean: &str) -> bool {
    matches!(
        clean,
        "ne" | "ni"
            | "niti"
            | "nije"
            | "nisam"
            | "nisi"
            | "nismo"
            | "niste"
            | "nisu"
            | "neću"
            | "necu"
            | "nećeš"
            | "neces"
            | "neće"
            | "nece"
            | "nećemo"
            | "necemo"
            | "nećete"
            | "necete"
            | "nemoj"
            | "nemojte"
            | "nemam"
            | "nema"
            | "nemaju"
            | "не"
            | "ни"
            | "нити"
            | "није"
            | "нисам"
            | "ниси"
            | "нисмо"
            | "нисте"
            | "нису"
            | "нећу"
            | "нећеш"
            | "неће"
            | "нећемо"
            | "нећете"
            | "немој"
            | "немојте"
            | "немам"
            | "нема"
            | "немају"
            | "not"
            | "no"
            | "never"
            | "don't"
            | "doesn't"
            | "didn't"
            | "won't"
            | "wouldn't"
            | "can't"
            | "cannot"
            | "couldn't"
            | "shouldn't"
            | "isn't"
            | "aren't"
            | "wasn't"
            | "weren't"
            | "haven't"
            | "hasn't"
            | "hadn't"
    )
}

fn is_wrap_preposition_or_article(clean: &str) -> bool {
    matches!(
        clean,
        "u" | "na"
            | "za"
            | "sa"
            | "s"
            | "od"
            | "do"
            | "iz"
            | "o"
            | "po"
            | "pri"
            | "ka"
            | "k"
            | "uz"
            | "niz"
            | "nad"
            | "pod"
            | "pred"
            | "među"
            | "medju"
            | "kroz"
            | "kod"
            | "bez"
            | "osim"
            | "prema"
            | "protiv"
            | "oko"
            | "posle"
            | "pre"
            | "у"
            | "на"
            | "за"
            | "са"
            | "с"
            | "од"
            | "до"
            | "из"
            | "о"
            | "по"
            | "при"
            | "ка"
            | "к"
            | "уз"
            | "низ"
            | "над"
            | "под"
            | "пред"
            | "међу"
            | "кроз"
            | "код"
            | "без"
            | "осим"
            | "према"
            | "против"
            | "око"
            | "после"
            | "пре"
            | "a"
            | "an"
            | "the"
            | "in"
            | "on"
            | "at"
            | "to"
            | "of"
            | "for"
            | "with"
            | "from"
            | "by"
            | "into"
            | "onto"
            | "upon"
            | "about"
            | "against"
            | "between"
            | "through"
            | "during"
            | "without"
            | "within"
            | "under"
            | "over"
    )
}

fn is_wrap_auxiliary_or_enclitic(clean: &str) -> bool {
    matches!(
        clean,
        "sam" | "si"
            | "je"
            | "smo"
            | "ste"
            | "su"
            | "ću"
            | "cu"
            | "ćeš"
            | "ces"
            | "će"
            | "ce"
            | "ćemo"
            | "cemo"
            | "ćete"
            | "cete"
            | "bih"
            | "bi"
            | "bismo"
            | "biste"
            | "se"
            | "li"
            | "bio"
            | "bila"
            | "bilo"
            | "bili"
            | "bile"
            | "сам"
            | "си"
            | "је"
            | "смо"
            | "сте"
            | "су"
            | "ћу"
            | "ћеш"
            | "ће"
            | "ћемо"
            | "ћете"
            | "бих"
            | "би"
            | "бисмо"
            | "бисте"
            | "се"
            | "ли"
            | "био"
            | "била"
            | "било"
            | "били"
            | "биле"
            | "is"
            | "am"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "has"
            | "have"
            | "had"
            | "do"
            | "does"
            | "did"
            | "will"
            | "would"
            | "shall"
            | "should"
            | "can"
            | "could"
            | "may"
            | "might"
            | "must"
    )
}

fn is_wrap_conjunction(clean: &str) -> bool {
    matches!(
        clean,
        "i" | "a"
            | "ali"
            | "ili"
            | "da"
            | "pa"
            | "te"
            | "jer"
            | "što"
            | "sto"
            | "ako"
            | "dok"
            | "kad"
            | "kada"
            | "nego"
            | "već"
            | "vec"
            | "kako"
            | "gde"
            | "gdje"
            | "koji"
            | "koja"
            | "koje"
            | "čiji"
            | "ciji"
            | "и"
            | "а"
            | "али"
            | "или"
            | "да"
            | "па"
            | "те"
            | "јер"
            | "што"
            | "ако"
            | "док"
            | "кад"
            | "када"
            | "него"
            | "већ"
            | "како"
            | "где"
            | "гдје"
            | "који"
            | "која"
            | "које"
            | "чији"
            | "and"
            | "or"
            | "but"
            | "so"
            | "yet"
            | "nor"
            | "that"
            | "which"
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
            | "if"
            | "whether"
    )
}

fn is_wrap_pronoun(clean: &str) -> bool {
    matches!(
        clean,
        "ja" | "ti"
            | "on"
            | "ona"
            | "ono"
            | "mi"
            | "vi"
            | "oni"
            | "one"
            | "moj"
            | "moja"
            | "moje"
            | "tvoj"
            | "tvoja"
            | "tvoje"
            | "njegov"
            | "njen"
            | "naš"
            | "nas"
            | "vaš"
            | "vas"
            | "njihov"
            | "svoj"
            | "svoja"
            | "svoje"
            | "ovaj"
            | "ova"
            | "ovo"
            | "taj"
            | "ta"
            | "to"
            | "onaj"
            | "mu"
            | "joj"
            | "ga"
            | "ih"
            | "nam"
            | "vam"
            | "ја"
            | "ти"
            | "он"
            | "она"
            | "оно"
            | "ми"
            | "ви"
            | "они"
            | "оне"
            | "мој"
            | "моја"
            | "моје"
            | "твој"
            | "твоја"
            | "твоје"
            | "његов"
            | "њен"
            | "наш"
            | "ваш"
            | "њихов"
            | "свој"
            | "своја"
            | "своје"
            | "овај"
            | "ова"
            | "ово"
            | "тај"
            | "та"
            | "то"
            | "онај"
            | "му"
            | "јој"
            | "га"
            | "их"
            | "нам"
            | "вам"
            | "i"
            | "you"
            | "he"
            | "she"
            | "it"
            | "we"
            | "they"
            | "my"
            | "your"
            | "his"
            | "her"
            | "its"
            | "our"
            | "their"
            | "this"
            | "that"
            | "these"
            | "those"
    )
}

fn is_sentence_opening_non_name(clean: &str) -> bool {
    matches!(
        clean,
        "danas"
            | "sada"
            | "sad"
            | "ovde"
            | "ovdje"
            | "tamo"
            | "tada"
            | "tad"
            | "kada"
            | "kad"
            | "ali"
            | "međutim"
            | "medjutim"
            | "ipak"
            | "dakle"
            | "zato"
            | "potom"
            | "zatim"
            | "onda"
            | "juče"
            | "juce"
            | "sutra"
            | "uvek"
            | "uvijek"
            | "nikad"
            | "nikada"
            | "možda"
            | "mozda"
            | "naravno"
            | "zapravo"
            | "inače"
            | "inace"
            | "dok"
            | "ako"
            | "pošto"
            | "posto"
            | "iako"
            | "čim"
            | "cim"
            | "već"
            | "vec"
            | "samo"
            | "čak"
            | "cak"
            | "baš"
            | "bas"
            | "i"
            | "a"
            | "pa"
            | "da"
            | "ne"
            | "ovaj"
            | "ova"
            | "ovo"
            | "taj"
            | "ta"
            | "to"
            | "onaj"
            | "ona"
            | "ono"
            | "on"
            | "mi"
            | "vi"
            | "oni"
            | "one"
            | "ja"
            | "ti"
            | "svi"
            | "sve"
            | "neko"
            | "niko"
            | "šta"
            | "sta"
            | "ko"
            | "kako"
            | "zašto"
            | "zasto"
            | "gde"
            | "gdje"
            | "koliko"
            | "данас"
            | "сада"
            | "сад"
            | "овде"
            | "овдје"
            | "тамо"
            | "тада"
            | "тад"
            | "када"
            | "кад"
            | "али"
            | "међутим"
            | "ипак"
            | "дакле"
            | "зато"
            | "потом"
            | "затим"
            | "онда"
            | "јуче"
            | "сутра"
            | "увек"
            | "увијек"
            | "никад"
            | "никада"
            | "можда"
            | "наравно"
            | "заправо"
            | "иначе"
            | "док"
            | "ако"
            | "пошто"
            | "иако"
            | "чим"
            | "већ"
            | "само"
            | "чак"
            | "баш"
            | "и"
            | "а"
            | "па"
            | "да"
            | "не"
            | "овај"
            | "ова"
            | "ово"
            | "тај"
            | "та"
            | "то"
            | "онај"
            | "она"
            | "оно"
            | "он"
            | "ми"
            | "ви"
            | "они"
            | "оне"
            | "ја"
            | "ти"
            | "сви"
            | "све"
            | "неко"
            | "нико"
            | "шта"
            | "ко"
            | "како"
            | "зашто"
            | "где"
            | "гдје"
            | "колико"
            | "when"
            | "today"
            | "tomorrow"
            | "yesterday"
            | "here"
            | "there"
            | "then"
            | "now"
            | "however"
            | "therefore"
            | "meanwhile"
            | "otherwise"
            | "suddenly"
            | "actually"
            | "maybe"
            | "perhaps"
            | "always"
            | "never"
            | "often"
            | "sometimes"
            | "already"
            | "still"
            | "just"
            | "only"
            | "even"
            | "also"
            | "but"
            | "and"
            | "or"
            | "so"
            | "yet"
            | "because"
            | "although"
            | "though"
            | "while"
            | "since"
            | "until"
            | "unless"
            | "if"
            | "whether"
            | "after"
            | "before"
            | "during"
            | "why"
            | "how"
            | "what"
            | "where"
            | "who"
            | "this"
            | "that"
            | "these"
            | "those"
            | "they"
            | "we"
            | "he"
            | "she"
            | "it"
            | "you"
            | "everyone"
            | "someone"
            | "nobody"
            | "finally"
            | "first"
            | "next"
            | "later"
    )
}

fn is_proper_name_pair(words: &[&str], k: usize, prev_clean: &str) -> bool {
    if k == 0 || k >= words.len() {
        return false;
    }
    if !starts_with_uppercase_alpha(words[k - 1]) || !starts_with_uppercase_alpha(words[k]) {
        return false;
    }
    let is_sentence_start = k == 1 || ends_with_sentence_punct(words[k - 2]);
    !(is_sentence_start && is_sentence_opening_non_name(prev_clean))
}

fn linguistic_split_penalty(words: &[&str], k: usize) -> i64 {
    if k == 0 || k >= words.len() {
        return 0;
    }
    let prev = words[k - 1];
    if ends_with_sentence_punct(prev) || ends_with_clause_punct(prev) {
        return 0;
    }
    let clean = normalize_wrap_token(prev);
    let mut penalty: i64 = 0;
    if is_wrap_negation(&clean) {
        penalty = penalty.max(42);
    }
    if is_wrap_preposition_or_article(&clean) {
        penalty = penalty.max(38);
    }
    if is_proper_name_pair(words, k, &clean) {
        penalty = penalty.max(36);
    }
    if is_wrap_auxiliary_or_enclitic(&clean) {
        penalty = penalty.max(34);
    }
    if is_wrap_conjunction(&clean) {
        penalty = penalty.max(30);
    }
    if is_wrap_pronoun(&clean) {
        penalty = penalty.max(28);
    }
    penalty
}

fn punct_break_bonus(word: &str) -> i64 {
    if ends_with_sentence_punct(word) {
        14
    } else if ends_with_clause_punct(word) {
        10
    } else {
        0
    }
}

fn solve_exact_line_count_dp(
    prefix_chars: &[usize],
    boundary_costs: &[i64],
    total_single_line_chars: usize,
    max_chars: usize,
    line_count: usize,
) -> (i64, Vec<usize>) {
    let n = prefix_chars.len() - 1;
    let l = line_count as i64;
    let total_lines_chars = (total_single_line_chars - (line_count - 1)) as i64;
    let max_chars_i64 = max_chars as i64;

    let line_len = |a: usize, b: usize| -> i64 {
        ((prefix_chars[b] - prefix_chars[a]) + (b - a - 1)) as i64
    };

    let inf_ov = i64::MAX / 4;
    let inf_sc = i64::MAX / 4;

    // dp_ov[r][i], dp_sc[r][i]: optimal (total_overflow, scaled_score) for words[i..n] in r lines.
    let mut dp_ov = vec![vec![inf_ov; n + 1]; line_count + 1];
    let mut dp_sc = vec![vec![inf_sc; n + 1]; line_count + 1];
    let mut next_split = vec![vec![n; n + 1]; line_count + 1];

    // Base case: r = 1 (words[i..n] on a single line)
    for i in 0..n {
        let len = line_len(i, n);
        let ov = (len - max_chars_i64).max(0);
        let balance = (l * len - total_lines_chars).abs();
        dp_ov[1][i] = ov;
        dp_sc[1][i] = ov * 1000 * l + balance;
        next_split[1][i] = n;
    }

    // Transitions: r = 2..=line_count
    for r in 2..=line_count {
        let i_start = if r == line_count { 0 } else { 1 };
        let i_end = if r == line_count { 0 } else { n - r };
        for i in i_start..=i_end {
            let mut best_ov = inf_ov;
            let mut best_sc = inf_sc;
            let mut best_k = i + 1;

            for k in (i + 1)..=(n - (r - 1)) {
                let tail_ov = dp_ov[r - 1][k];
                if tail_ov >= inf_ov {
                    continue;
                }
                let len = line_len(i, k);
                let ov = (len - max_chars_i64).max(0);
                let balance = (l * len - total_lines_chars).abs();
                let cand_ov = ov + tail_ov;
                let cand_sc = ov * 1000 * l + balance + boundary_costs[k] * l + dp_sc[r - 1][k];

                if cand_ov < best_ov || (cand_ov == best_ov && cand_sc < best_sc) {
                    best_ov = cand_ov;
                    best_sc = cand_sc;
                    best_k = k;
                }
            }

            dp_ov[r][i] = best_ov;
            dp_sc[r][i] = best_sc;
            next_split[r][i] = best_k;
        }
    }

    let mut splits = Vec::with_capacity(line_count - 1);
    let mut cur = 0usize;
    for r in (2..=line_count).rev() {
        let k = next_split[r][cur];
        splits.push(k);
        cur = k;
    }

    (dp_ov[line_count][0], splits)
}

/// Balances text into at most `max_lines` (`1..=4`) linguistically-clean lines using
/// exact dynamic programming (minimizing overflow first, then smallest feasible line count,
/// then length balance + linguistic split penalties - punctuation break bonuses).
/// Behaviorally 1:1 identical to TypeScript `wrapBalancedLines`.
pub fn wrap_balanced_lines(text: &str, max_chars_per_line: u32, max_lines: u32) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }

    let single_line = words.join(" ");
    let single_line_chars = single_line.chars().count();
    let max_chars = max_chars_per_line.max(1) as usize;
    let clamped_max_lines = max_lines.clamp(1, 4) as usize;

    if clamped_max_lines <= 1 || single_line_chars <= max_chars || words.len() == 1 {
        return single_line;
    }

    let n = words.len();
    let effective_max_lines = clamped_max_lines.min(n);

    let mut prefix_chars = Vec::with_capacity(n + 1);
    prefix_chars.push(0usize);
    for w in &words {
        let prev = *prefix_chars.last().unwrap_or(&0);
        prefix_chars.push(prev + w.chars().count());
    }

    let mut boundary_costs = vec![0i64; n];
    for k in 1..n {
        boundary_costs[k] = linguistic_split_penalty(&words, k) - punct_break_bonus(words[k - 1]);
    }

    let mut best_overflow = i64::MAX;
    let mut best_splits: Vec<usize> = Vec::new();

    for line_count in 2..=effective_max_lines {
        let (ov, splits) = solve_exact_line_count_dp(
            &prefix_chars,
            &boundary_costs,
            single_line_chars,
            max_chars,
            line_count,
        );
        if ov < best_overflow {
            best_overflow = ov;
            best_splits = splits;
            if ov == 0 {
                break;
            }
        }
    }

    let mut lines = Vec::with_capacity(best_splits.len() + 1);
    let mut start = 0usize;
    for end in best_splits {
        lines.push(words[start..end].join(" "));
        start = end;
    }
    lines.push(words[start..].join(" "));
    lines.join("\n")
}

/// Computes `cue.cps` (rounded to 1 decimal place) and populates `cue.warnings` using
/// the canonical warning IDs (`"critical_cps"`, `"high_cps"`, `"line_too_long"`,
/// `"too_many_lines"`, `"duration_too_short"`, `"duration_too_long"`).
pub fn compute_cue_metrics(cue: &mut SubtitleCue, rules: &SegmentRules) {
    let raw_visible = if !cue.translated_text.trim().is_empty() {
        cue.translated_text.as_str()
    } else {
        cue.source_text.as_str()
    };
    let normalized = raw_visible.replace("\r\n", "\n").replace('\r', "\n");
    let visible = normalized.trim();

    let char_count = visible.chars().filter(|&c| c != '\n').count();
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
    if cps > rules.max_cps + 4.0 {
        warnings.push("critical_cps".to_string());
    } else if cps > rules.max_cps {
        warnings.push("high_cps".to_string());
    }

    let max_chars = rules.max_chars_per_line as usize;
    let lines: Vec<&str> = if visible.is_empty() {
        Vec::new()
    } else {
        visible.split('\n').collect()
    };
    if lines.iter().any(|l| l.chars().count() > max_chars) {
        warnings.push("line_too_long".to_string());
    }
    if lines.len() > rules.max_lines as usize {
        warnings.push("too_many_lines".to_string());
    }

    if duration_ms < rules.min_duration_ms && char_count > 0 {
        warnings.push("duration_too_short".to_string());
    } else if duration_ms > rules.max_duration_ms {
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
        let rules = SegmentRules::default(); // max_cps = 17.0, max_chars_per_line = 42, max_lines = 2
        let mut critical_cue = SubtitleCue {
            id: 1,
            start_ms: 0,
            end_ms: 500, // 0.5s -> duration_too_short & critical_cps
            source_text: "Kratko".to_string(),
            translated_text:
                "Ово је превод који је превише дугачак за једну линију од четрдесет два карактера.\nДруги ред.\nТрећи ред."
                    .to_string(),
            translated_variants: std::collections::BTreeMap::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut critical_cue, &rules);
        assert!(critical_cue.cps > rules.max_cps + 4.0);
        assert!(critical_cue.warnings.contains(&"critical_cps".to_string()));
        assert!(!critical_cue.warnings.contains(&"high_cps".to_string()));
        assert!(critical_cue.warnings.contains(&"line_too_long".to_string()));
        assert!(critical_cue
            .warnings
            .contains(&"too_many_lines".to_string()));
        assert!(critical_cue
            .warnings
            .contains(&"duration_too_short".to_string()));

        // High (non-critical) CPS: 38 chars over 2.0s = 19.0 CPS (17 < 19 <= 21)
        let mut high_cue = SubtitleCue {
            id: 2,
            start_ms: 0,
            end_ms: 2000,
            source_text: "Ovo je malo brži titl sa trideset osam".to_string(),
            translated_text: String::new(),
            translated_variants: std::collections::BTreeMap::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut high_cue, &rules);
        assert_eq!(high_cue.cps, 19.0);
        assert_eq!(high_cue.warnings, vec!["high_cps".to_string()]);

        // Long duration (> 7000ms)
        let mut long_cue = SubtitleCue {
            id: 3,
            start_ms: 0,
            end_ms: 8500,
            source_text: "Dugačak prikaz na ekranu.".to_string(),
            translated_text: String::new(),
            translated_variants: std::collections::BTreeMap::new(),
            cps: 0.0,
            warnings: Vec::new(),
        };
        compute_cue_metrics(&mut long_cue, &rules);
        assert_eq!(long_cue.warnings, vec!["duration_too_long".to_string()]);
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
            // Cue 1: 0..1400ms, followed by 320ms silence (<= 500ms -> bridges to 1720 - 80 = 1640ms)
            word(0, 700, "Prva"),
            word(700, 1400, "rečenica."),
            // Cue 2: 1720..3200ms, followed by 1100ms pause (> 500ms -> holds +350ms to 3550ms)
            word(1720, 2400, "Druga"),
            word(2400, 3200, "rečenica."),
            // Cue 3: 4300..5600ms (final cue -> holds +350ms to 5950ms)
            word(4300, 4900, "Treća"),
            word(4900, 5600, "rečenica."),
        ];
        let rules = SegmentRules::default();
        let cues = segment_words(&words, &rules);

        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].end_ms, 1640);
        assert_eq!(cues[1].start_ms, 1720);
        assert!(
            cues[1].start_ms - cues[0].end_ms >= MIN_INTER_CUE_GAP_MS,
            "expected >= 80ms breathing gap"
        );
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
                translated_variants: std::collections::BTreeMap::new(),
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
                translated_variants: std::collections::BTreeMap::new(),
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

    #[test]
    fn strip_vocal_hesitations_removes_non_lexical_fillers_and_cleans_punctuation() {
        // Standalone hesitations with mixed case/punctuation are stripped, while lexical tokens stay intact.
        let words = vec![
            word(0, 200, "Um,"),
            word(250, 500, "we"),
            word(500, 800, "were,"),
            word(800, 1000, "uh,"),
            word(1000, 1200, "erm..."),
            word(1200, 1400, "ER?"),
            word(1400, 1600, "(hmm)"),
            word(1600, 1800, "mm,"),
            word(1800, 2000, "Uhm,"),
            word(2000, 2400, "going"),
            word(2400, 2800, "home"),
            word(2800, 3200, "umesto"),
            word(3200, 3600, "summer"),
            word(3600, 4000, "error"),
            word(4000, 4400, "uh-huh."),
        ];
        let stripped = strip_vocal_hesitations(&words);
        let tokens: Vec<&str> = stripped.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(
            tokens,
            vec!["we", "were,", "going", "home", "umesto", "summer", "error", "uh-huh."]
        );

        // Orphan standalone commas before/after hesitations and doubled commas are cleaned cleanly.
        let orphan_commas = vec![
            word(0, 200, "We"),
            word(200, 400, "went"),
            word(400, 450, ","),
            word(450, 600, "um"),
            word(600, 650, ","),
            word(650, 900, "home."),
        ];
        let cleaned_orphans: Vec<String> = strip_vocal_hesitations(&orphan_commas)
            .into_iter()
            .map(|w| w.text)
            .collect();
        assert_eq!(cleaned_orphans, vec!["We", "went", "home."]);

        // Doubled trailing comma + leading comma on next surviving word
        let doubled = vec![
            word(0, 200, "Well,,"),
            word(200, 400, "uh,"),
            word(400, 700, ",okay."),
        ];
        let cleaned_doubled: Vec<String> = strip_vocal_hesitations(&doubled)
            .into_iter()
            .map(|w| w.text)
            .collect();
        assert_eq!(cleaned_doubled, vec!["Well,", "okay."]);

        // All-hesitation input yields empty vector
        let only_fillers = vec![word(0, 200, "Um,"), word(250, 500, "uh...")];
        assert!(strip_vocal_hesitations(&only_fillers).is_empty());
    }

    #[test]
    fn strip_vocal_hesitations_propagates_speaker_change_and_aligns_segment_options() {
        let rules = SegmentRules::default();

        // Speaker change marked directly on a stripped hesitation propagates to next surviving word.
        let mut tagged_words = vec![
            word(0, 300, "Zdravo"),
            word(300, 600, "svima"),
            word(650, 850, "um,"),
            word(900, 1200, "dobro"),
            word(1200, 1500, "veče."),
        ];
        tagged_words[2].speaker_change = true;

        let stripped = strip_vocal_hesitations(&tagged_words);
        assert_eq!(stripped.len(), 4);
        assert_eq!(stripped[2].text, "dobro");
        assert!(
            stripped[2].speaker_change,
            "speaker_change from stripped hesitation must propagate to next surviving word"
        );

        let cues_removed = segment_words_with_options(&tagged_words, &rules, &[], true, true);
        assert_eq!(cues_removed.len(), 2);
        assert_eq!(cues_removed[0].source_text, "Zdravo svima");
        assert_eq!(cues_removed[1].source_text, "dobro veče.");

        // Explicit speaker_change_indices pointing AFTER a stripped hesitation must stay aligned.
        let plain_words = vec![
            word(0, 300, "Zdravo"),
            word(300, 500, "uh,"),
            word(550, 850, "svima"),
            word(900, 1200, "dobro"),
            word(1200, 1500, "veče."),
        ];
        let cues_idx_after =
            segment_words_with_options(&plain_words, &rules, &[3], true, true);
        assert_eq!(cues_idx_after.len(), 2);
        assert_eq!(cues_idx_after[0].source_text, "Zdravo svima");
        assert_eq!(cues_idx_after[1].source_text, "dobro veče.");

        // Explicit speaker_change_indices pointing ON the stripped hesitation propagates forward too.
        let cues_idx_on = segment_words_with_options(&plain_words, &rules, &[1], true, true);
        assert_eq!(cues_idx_on.len(), 2);
        assert_eq!(cues_idx_on[0].source_text, "Zdravo");
        assert_eq!(cues_idx_on[1].source_text, "svima dobro veče.");

        // remove_fillers == false preserves the hesitation verbatim.
        let cues_verbatim = segment_words_with_options(&plain_words, &rules, &[3], true, false);
        assert_eq!(cues_verbatim.len(), 2);
        assert_eq!(cues_verbatim[0].source_text, "Zdravo uh, svima");
        assert_eq!(cues_verbatim[1].source_text, "dobro veče.");
    }

    #[test]
    fn eliminates_trailing_orphan_interrogatives_and_rolls_back_unpunctuated_starters() {
        let rules = SegmentRules::default();

        // Workstream 2 plan test #1: "Okay, alright, what" + 520 ms pause + "are we doing now?"
        let en_words = vec![
            word(0, 300, "Okay,"),
            word(300, 650, "alright,"),
            word(650, 900, "what"),
            word(1420, 1620, "are"),
            word(1620, 1800, "we"),
            word(1800, 2100, "doing"),
            word(2100, 2400, "now?"),
        ];
        let en_cues = segment_words(&en_words, &rules);
        assert_eq!(en_cues.len(), 2, "expected clean split: {:?}", en_cues);
        assert_eq!(en_cues[0].source_text, "Okay, alright,");
        assert_eq!(en_cues[1].source_text, "what are we doing now?");
        assert_eq!(en_cues[1].start_ms, 650);
        assert!(en_cues[1].start_ms - en_cues[0].end_ms >= MIN_INTER_CUE_GAP_MS);

        // Serbian Latin & Cyrillic interrogatives ("šta", "шта")
        let sr_lat = vec![
            word(0, 350, "Dobro,"),
            word(350, 500, "u"),
            word(500, 800, "redu,"),
            word(800, 1050, "šta"),
            word(1570, 1850, "ćemo"),
            word(1850, 2100, "sada"),
            word(2100, 2300, "da"),
            word(2300, 2650, "radimo?"),
        ];
        let sr_lat_cues = segment_words(&sr_lat, &rules);
        assert_eq!(sr_lat_cues.len(), 2);
        assert_eq!(sr_lat_cues[0].source_text, "Dobro, u redu,");
        assert_eq!(sr_lat_cues[1].source_text, "šta ćemo sada da radimo?");

        let sr_cyr = vec![
            word(0, 350, "Добро,"),
            word(350, 500, "у"),
            word(500, 800, "реду,"),
            word(800, 1050, "шта"),
            word(1570, 1850, "ћемо"),
            word(1850, 2100, "сада"),
            word(2100, 2300, "да"),
            word(2300, 2650, "радимо?"),
        ];
        let sr_cyr_cues = segment_words(&sr_cyr, &rules);
        assert_eq!(sr_cyr_cues.len(), 2);
        assert_eq!(sr_cyr_cues[0].source_text, "Добро, у реду,");
        assert_eq!(sr_cyr_cues[1].source_text, "шта ћемо сада да радимо?");

        // Workstream 2 plan test #2: General Structural Rollback on non-function starter "danas"
        let rollback_words = vec![
            word(0, 400, "Završili"),
            word(400, 650, "smo"),
            word(650, 900, "to,"),
            word(900, 1200, "danas"),
            word(1720, 2100, "krećemo"),
            word(2100, 2600, "ispočetka."),
        ];
        let rollback_cues = segment_words(&rollback_words, &rules);
        assert_eq!(rollback_cues.len(), 2, "expected structural rollback: {:?}", rollback_cues);
        assert_eq!(rollback_cues[0].source_text, "Završili smo to,");
        assert_eq!(rollback_cues[1].source_text, "danas krećemo ispočetka.");
        assert_eq!(rollback_cues[1].start_ms, 900);
        assert!(rollback_cues[1].start_ms - rollback_cues[0].end_ms >= MIN_INTER_CUE_GAP_MS);

        // Complete closing vocative with terminal punctuation stays intact!
        let vocative_words = vec![
            word(0, 350, "Hvala"),
            word(350, 600, "ti,"),
            word(600, 950, "brate."),
            word(1500, 1850, "Vidimo"),
            word(1850, 2050, "se"),
            word(2050, 2400, "sutra."),
        ];
        let vocative_cues = segment_words(&vocative_words, &rules);
        assert_eq!(vocative_cues.len(), 2);
        assert_eq!(vocative_cues[0].source_text, "Hvala ti, brate.");
        assert_eq!(vocative_cues[1].source_text, "Vidimo se sutra.");

        // Verify expanded interrogative/starter coverage (EN + SR Latin + SR Cyrillic)
        for token in [
            "what", "why", "how", "if", "whether", "who", "where", "when", "šta", "sta", "ko",
            "koga", "kom", "kome", "čemu", "cemu", "zašto", "zasto", "kako", "gde", "gdje", "kuda",
            "odakle", "otkud", "koliko", "zar", "li", "kad", "kada", "dok", "ako", "шта", "ко",
            "кога", "ком", "коме", "чему", "зашто", "како", "где", "гдје", "куда", "одакле",
            "откуд", "колико", "зар", "ли", "кад", "када", "док", "ако",
        ] {
            assert!(
                is_dangling_function_word(token),
                "expected '{token}' to be recognized as dangling function word"
            );
        }
    }

    #[test]
    fn enforces_min_80ms_inter_cue_gap_across_0ms_40ms_and_250ms_boundaries() {
        let rules = SegmentRules::default();
        let words = vec![
            // Cue 1: 0..1000ms -> 0ms gap to Cue 2
            word(0, 500, "Prva"),
            word(500, 1000, "rečenica."),
            // Cue 2: 1000..2000ms -> 40ms gap to Cue 3
            word(1000, 1500, "Druga"),
            word(1500, 2000, "rečenica."),
            // Cue 3: 2040..3000ms -> 250ms gap to Cue 4
            word(2040, 2500, "Treća"),
            word(2500, 3000, "rečenica."),
            // Cue 4: 3250..4200ms
            word(3250, 3700, "Četvrta"),
            word(3700, 4200, "rečenica."),
        ];

        let cues = segment_words(&words, &rules);
        assert_eq!(cues.len(), 4);
        for k in 0..(cues.len() - 1) {
            assert!(
                cues[k].end_ms > cues[k].start_ms,
                "cue {} must have positive duration",
                k + 1
            );
            let gap = cues[k + 1].start_ms.saturating_sub(cues[k].end_ms);
            assert!(
                gap >= MIN_INTER_CUE_GAP_MS,
                "expected >= 80ms gap between cue {} ({}..{}) and cue {} ({}..{}), got {}ms",
                k + 1,
                cues[k].start_ms,
                cues[k].end_ms,
                k + 2,
                cues[k + 1].start_ms,
                cues[k + 1].end_ms,
                gap
            );
        }
        assert!(cues.last().unwrap().end_ms > cues.last().unwrap().start_ms);
    }

    #[test]
    fn enforce_min_inter_cue_gap_handles_short_cues_symmetrically_without_zero_length() {
        let rules = SegmentRules::default();
        let mk = |id: u32, start_ms: u64, end_ms: u64, text: &str| {
            let mut cue = SubtitleCue {
                id,
                start_ms,
                end_ms,
                source_text: text.to_string(),
                translated_text: String::new(),
                translated_variants: std::collections::BTreeMap::new(),
                cps: 0.0,
                warnings: Vec::new(),
            };
            compute_cue_metrics(&mut cue, &rules);
            cue
        };

        // 1. Short Cue 1 (0..260ms, pulling back 80ms would drop below 300ms) -> symmetric -40/+40 split
        let mut sym = vec![mk(1, 0, 260, "Da."), mk(2, 260, 700, "Idemo.")];
        enforce_min_inter_cue_gap(&mut sym, &rules);
        assert_eq!(sym[0].end_ms, 220);
        assert_eq!(sym[1].start_ms, 300);
        assert_eq!(sym[1].start_ms - sym[0].end_ms, MIN_INTER_CUE_GAP_MS);
        assert!(sym[0].end_ms - sym[0].start_ms >= MIN_GAP_SPLIT_DURATION_MS);
        assert!(sym[1].end_ms - sym[1].start_ms >= MIN_GAP_SPLIT_DURATION_MS);

        // 2. Tight Cue 1 (0..180ms, only 20ms slack above 160ms) -> takes 20ms from left, 60ms from right
        let mut asym = vec![mk(1, 0, 180, "Da."), mk(2, 180, 700, "Idemo.")];
        enforce_min_inter_cue_gap(&mut asym, &rules);
        assert_eq!(asym[0].end_ms, 160);
        assert_eq!(asym[1].start_ms, 240);
        assert_eq!(asym[1].start_ms - asym[0].end_ms, MIN_INTER_CUE_GAP_MS);

        // 3. Ultra-short cues (< 160ms each) never shrink to zero-length or invert
        let mut tiny = vec![mk(1, 0, 120, "A."), mk(2, 120, 240, "B.")];
        enforce_min_inter_cue_gap(&mut tiny, &rules);
        assert_eq!(tiny[0].start_ms, 0);
        assert_eq!(tiny[0].end_ms, 120);
        assert_eq!(tiny[1].start_ms, 120);
        assert_eq!(tiny[1].end_ms, 240);
    }

    #[test]
    fn wrap_balanced_lines_guards_negations_prepositions_auxiliaries_and_proper_names() {
        // Negation guard: never leave "ne" at end of Line 1
        let neg = wrap_balanced_lines("Rekao mi je da ne dolazim sutra na posao.", 28, 2);
        let neg_lines: Vec<&str> = neg.lines().collect();
        assert_eq!(neg_lines.len(), 2);
        assert_ne!(
            neg_lines[0].split_whitespace().last().unwrap_or("").to_lowercase(),
            "ne"
        );
        assert_ne!(
            neg_lines[0].split_whitespace().last().unwrap_or("").to_lowercase(),
            "da"
        );

        // Preposition & auxiliary guard: never end Line 1 on "sa", "smo", or "o"
        assert_eq!(
            wrap_balanced_lines("Danas smo razgovarali sa našim kolegama o planu.", 42, 2),
            "Danas smo razgovarali\nsa našim kolegama o planu."
        );

        // Proper-name pair guard keeps "Nikola Tesla" intact on one line
        let proper = wrap_balanced_lines(
            "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
            42,
            2,
        );
        assert!(
            proper.contains("Nikola Tesla"),
            "expected 'Nikola Tesla' intact on one line, got: {proper:?}"
        );

        // Sentence-opening adverb exclusion vs genuine proper-name pair
        assert_eq!(
            wrap_balanced_lines("Međutim Marko radi.", 14, 2),
            "Međutim\nMarko radi."
        );
        assert_eq!(
            wrap_balanced_lines("Aleksandar Tesla radi.", 18, 2),
            "Aleksandar Tesla\nradi."
        );

        // Hard line-width limit (overflow_penalty = 1000/char) wins when unavoidable
        assert_eq!(
            wrap_balanced_lines(
                "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
                30,
                2
            ),
            "Danas Marko dolazi ranije u\nkancelariju da završi izveštaj."
        );
    }

    #[test]
    fn parity_corpus_15_sentences_and_canonical_warnings_match_typescript() {
        let corpus: [(&str, u32, [&str; 4]); 15] = [
            (
                "Rekao mi je da ne dolazim sutra na posao.",
                28,
                [
                    "Rekao mi je da ne dolazim sutra na posao.",
                    "Rekao mi je da ne dolazim\nsutra na posao.",
                    "Rekao mi je da ne dolazim\nsutra na posao.",
                    "Rekao mi je da ne dolazim\nsutra na posao.",
                ],
            ),
            (
                "Danas smo razgovarali sa našim kolegama o planu.",
                42,
                [
                    "Danas smo razgovarali sa našim kolegama o planu.",
                    "Danas smo razgovarali\nsa našim kolegama o planu.",
                    "Danas smo razgovarali\nsa našim kolegama o planu.",
                    "Danas smo razgovarali\nsa našim kolegama o planu.",
                ],
            ),
            (
                "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
                38,
                [
                    "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
                    "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
                    "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
                    "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
                ],
            ),
            (
                "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
                30,
                [
                    "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
                    "Danas Marko dolazi ranije u\nkancelariju da završi izveštaj.",
                    "Danas Marko dolazi\nranije u kancelariju\nda završi izveštaj.",
                    "Danas Marko dolazi\nranije u kancelariju\nda završi izveštaj.",
                ],
            ),
            (
                "Овај пројекат предводи Никола Тесла у главној лабораторији у Београду.",
                34,
                [
                    "Овај пројекат предводи Никола Тесла у главној лабораторији у Београду.",
                    "Овај пројекат предводи Никола Тесла\nу главној лабораторији у Београду.",
                    "Овај пројекат предводи\nНикола Тесла у главној\nлабораторији у Београду.",
                    "Овај пројекат предводи\nНикола Тесла у главној\nлабораторији у Београду.",
                ],
            ),
            (
                "Нисам сигуран да нећемо морати да кренемо раније него што смо планирали.",
                30,
                [
                    "Нисам сигуран да нећемо морати да кренемо раније него што смо планирали.",
                    "Нисам сигуран да нећемо морати\nда кренемо раније него што смо планирали.",
                    "Нисам сигуран да нећемо морати\nда кренемо раније\nнего што смо планирали.",
                    "Нисам сигуран да нећемо морати\nда кренемо раније\nнего што смо планирали.",
                ],
            ),
            (
                "We didn't expect to meet San Francisco engineers at the conference today.",
                30,
                [
                    "We didn't expect to meet San Francisco engineers at the conference today.",
                    "We didn't expect to meet San Francisco\nengineers at the conference today.",
                    "We didn't expect to meet\nSan Francisco engineers\nat the conference today.",
                    "We didn't expect to meet\nSan Francisco engineers\nat the conference today.",
                ],
            ),
            (
                "Today John arrives early, because he wants to review the entire draft before noon.",
                30,
                [
                    "Today John arrives early, because he wants to review the entire draft before noon.",
                    "Today John arrives early, because he wants\nto review the entire draft before noon.",
                    "Today John arrives early,\nbecause he wants to review\nthe entire draft before noon.",
                    "Today John arrives early,\nbecause he wants to review\nthe entire draft before noon.",
                ],
            ),
            (
                "First sentence ends here. Second sentence opens cleanly! Third part wraps onto line three.",
                36,
                [
                    "First sentence ends here. Second sentence opens cleanly! Third part wraps onto line three.",
                    "First sentence ends here. Second sentence opens\ncleanly! Third part wraps onto line three.",
                    "First sentence ends here.\nSecond sentence opens cleanly!\nThird part wraps onto line three.",
                    "First sentence ends here.\nSecond sentence opens cleanly!\nThird part wraps onto line three.",
                ],
            ),
            (
                "Profesor dr. Marko Petrović je objasnio zašto ne smemo da preskočimo ovaj važan korak u analizi podataka.",
                32,
                [
                    "Profesor dr. Marko Petrović je objasnio zašto ne smemo da preskočimo ovaj važan korak u analizi podataka.",
                    "Profesor dr. Marko Petrović je objasnio zašto ne smemo\nda preskočimo ovaj važan korak u analizi podataka.",
                    "Profesor dr. Marko Petrović je objasnio\nzašto ne smemo da preskočimo ovaj\nvažan korak u analizi podataka.",
                    "Profesor dr. Marko Petrović\nje objasnio zašto ne smemo\nda preskočimo ovaj važan\nkorak u analizi podataka.",
                ],
            ),
            (
                "Када смо стигли у Нови Сад, одмах смо отишли на састанак са колегама из централног тима за развој софтвера.",
                32,
                [
                    "Када смо стигли у Нови Сад, одмах смо отишли на састанак са колегама из централног тима за развој софтвера.",
                    "Када смо стигли у Нови Сад, одмах смо отишли на састанак\nса колегама из централног тима за развој софтвера.",
                    "Када смо стигли у Нови Сад, одмах\nсмо отишли на састанак са колегама\nиз централног тима за развој софтвера.",
                    "Када смо стигли у Нови Сад,\nодмах смо отишли на састанак\nса колегама из централног\nтима за развој софтвера.",
                ],
            ),
            (
                "She told us that she can't join the meeting in New York without her manager's final approval.",
                30,
                [
                    "She told us that she can't join the meeting in New York without her manager's final approval.",
                    "She told us that she can't join the meeting\nin New York without her manager's final approval.",
                    "She told us that she can't join\nthe meeting in New York without\nher manager's final approval.",
                    "She told us that she can't\njoin the meeting in New York\nwithout her manager's\nfinal approval.",
                ],
            ),
            (
                "Međutim Marko radi bez prestanka na ovom zadatku.",
                18,
                [
                    "Međutim Marko radi bez prestanka na ovom zadatku.",
                    "Međutim Marko radi\nbez prestanka na ovom zadatku.",
                    "Međutim Marko\nradi bez prestanka\nna ovom zadatku.",
                    "Međutim Marko\nradi bez prestanka\nna ovom zadatku.",
                ],
            ),
            (
                "Aleksandar Tesla radi svakog dana u laboratoriji.",
                20,
                [
                    "Aleksandar Tesla radi svakog dana u laboratoriji.",
                    "Aleksandar Tesla radi\nsvakog dana u laboratoriji.",
                    "Aleksandar Tesla\nradi svakog dana\nu laboratoriji.",
                    "Aleksandar Tesla\nradi svakog dana\nu laboratoriji.",
                ],
            ),
            (
                "Kratak titl u jednom redu.",
                42,
                [
                    "Kratak titl u jednom redu.",
                    "Kratak titl u jednom redu.",
                    "Kratak titl u jednom redu.",
                    "Kratak titl u jednom redu.",
                ],
            ),
        ];

        for (idx, (text, max_chars, expected_by_lines)) in corpus.iter().enumerate() {
            for max_lines in 1u32..=4u32 {
                let actual = wrap_balanced_lines(text, *max_chars, max_lines);
                assert_eq!(
                    actual,
                    expected_by_lines[(max_lines - 1) as usize],
                    "Parity mismatch on sentence #{} (max_lines={max_lines})",
                    idx + 1
                );
                assert_eq!(
                    actual.replace('\n', " "),
                    *text,
                    "Wrapped text must preserve all words on sentence #{}",
                    idx + 1
                );
            }
        }

        // Canonical cue warning IDs & thresholds parity matrix (1:1 with subtitleUtils.test.ts)
        let standard_rules = SegmentRules::default();
        let warning_cases: [(u64, u64, &str, f64, &[&str]); 6] = [
            (1000, 4000, "Dobro došli u EasySRT Studio.", 9.7, &[]),
            (
                0,
                2000,
                "Ovo je malo brži titl sa trideset osam",
                19.0,
                &["high_cps"],
            ),
            (
                0,
                500,
                "Ovaj prvi red ima mnogo više od četrdeset dva karaktera u jednom redu titla.\nDrugi red.\nTreći red.",
                192.0,
                &[
                    "critical_cps",
                    "line_too_long",
                    "too_many_lines",
                    "duration_too_short",
                ],
            ),
            (0, 8500, "Dugačak prikaz na ekranu.", 2.9, &["duration_too_long"]),
            (
                0,
                3000,
                "Prvi red.\nDrugi red.\nTreći red.",
                9.7,
                &["too_many_lines"],
            ),
            (0, 200, "   ", 0.0, &[]),
        ];

        for (start_ms, end_ms, source_text, expected_cps, expected_warnings) in warning_cases {
            let mut cue = SubtitleCue {
                id: 1,
                start_ms,
                end_ms,
                source_text: source_text.to_string(),
                translated_text: String::new(),
                translated_variants: std::collections::BTreeMap::new(),
                cps: 0.0,
                warnings: Vec::new(),
            };
            compute_cue_metrics(&mut cue, &standard_rules);
            assert_eq!(cue.cps, expected_cps);
            let expected: Vec<String> = expected_warnings.iter().map(|s| (*s).to_string()).collect();
            assert_eq!(cue.warnings, expected);
        }
    }
}
