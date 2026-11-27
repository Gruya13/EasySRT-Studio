//! Local export-triggered diff engine, structural/linguistic edit classifiers (Level 2A),
//! K-anonymous lowercase n-gram diff extractor (Level 2B), and PII/proper-noun sanitizer (Level 3).
//!
//! All analysis runs locally in memory in milliseconds without external network calls.

use crate::models::{AddressForm, JobOptions, SegmentRules, SubtitleCue, TranslationStyle};
use crate::pipeline::sr_script::to_latin;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

const TIMELINE_TRIM_THRESHOLD_MS: i64 = 40;
const MAX_TRAINING_PAIRS: usize = 25;
const MIN_INFLECTION_PREFIX_CHARS: usize = 4;
const MAX_NGRAM_SPAN_WORDS: usize = 3;
const MAX_NGRAM_TOKEN_CHARS: usize = 24;
const MIN_NGRAM_SINGLE_TOKEN_CHARS: usize = 2;

/// Compact snapshot of an AI-generated cue captured before human editing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiBaselineCue {
    pub id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub source_text: String,
    pub translated_text: String,
}

/// Level 2A zero-text telemetry metrics summarizing edits, readability, linguistic fixes,
/// and segmentation/timing adjustments at export time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDiffMetrics {
    pub total_baseline_cues: u32,
    pub total_final_cues: u32,
    pub unchanged_cues_count: u32,
    pub text_edited_cues_count: u32,
    pub edit_rate_pct: f64,
    pub initial_high_cps_count: u32,
    pub final_high_cps_count: u32,
    pub initial_line_too_long_count: u32,
    pub final_line_too_long_count: u32,
    pub avg_cps_before: f64,
    pub avg_cps_after: f64,
    pub condense_cps_used: bool,
    pub enclitic_reordered_count: u32,
    pub redundant_possessive_removed_count: u32,
    pub passive_calque_removed_count: u32,
    pub conjunction_comma_fixed_count: u32,
    pub missed_filler_deleted_count: u32,
    pub inflection_suffix_fix_count: u32,
    pub boundary_orphan_shifted_count: u32,
    pub split_count: u32,
    pub split_after_comma_count: u32,
    pub split_after_conjunction_count: u32,
    pub avg_pre_split_chars: f64,
    pub avg_pre_split_duration_ms: f64,
    pub merge_count: u32,
    pub avg_merged_short_cue_duration_ms: f64,
    pub avg_merged_short_cue_chars: f64,
    pub avg_merged_gap_ms: f64,
    pub start_trim_count: u32,
    pub avg_start_delta_ms: f64,
    pub end_trim_count: u32,
    pub avg_end_delta_ms: f64,
}

/// Level 2B K-anonymous lowercase lexical/phrasing diff span with unlinkable monthly cohort proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeNgramDiff {
    pub lang_pair: String,
    pub old_span: String,
    pub new_span: String,
    pub cohort_proof: String,
}

/// Level 3 anonymized `(source, initial_ai, final_human)` cue pair for translation improvement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingCuePair {
    pub source_lang: String,
    pub target_lang: String,
    pub style: String,
    pub address_form: String,
    pub source_text: String,
    pub initial_ai_text: String,
    pub final_human_text: String,
}

/// Full local export diff bundle produced for telemetry submission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDiffReport {
    pub metrics: ExportDiffMetrics,
    pub ngram_diffs: Vec<SafeNgramDiff>,
    pub training_pairs: Vec<TrainingCuePair>,
}

/// Captures a lightweight baseline snapshot of AI-generated cues.
pub fn snapshot_baseline_cues(cues: &[SubtitleCue]) -> Vec<AiBaselineCue> {
    cues.iter()
        .map(|cue| AiBaselineCue {
            id: cue.id,
            start_ms: cue.start_ms,
            end_ms: cue.end_ms,
            source_text: cue.source_text.clone(),
            translated_text: cue.translated_text.clone(),
        })
        .collect()
}

/// Counts non-trivial 1:1 aligned modified cues eligible for Level 3 training donation (capped at 25).
pub fn count_modified_training_cues(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
) -> usize {
    let alignment = align_cues(baseline, final_cues);
    alignment
        .one_to_one
        .iter()
        .filter(|&&(b_idx, f_idx)| is_non_trivial_training_edit(&baseline[b_idx], &final_cues[f_idx]))
        .take(MAX_TRAINING_PAIRS)
        .count()
}

/// Computes a 24-hex-char HMAC-SHA256 cohort proof keyed with `monthly_salt` for a specific
/// `(lang_pair, old_span, new_span)` correction so pairs cannot be correlated across exports.
pub fn build_monthly_cohort_proof(
    monthly_salt: &str,
    lang_pair: &str,
    old_span: &str,
    new_span: &str,
) -> String {
    let msg = format!("{lang_pair}|{old_span}->{new_span}");
    let digest = hmac_sha256(monthly_salt.as_bytes(), msg.as_bytes());
    hex::encode(digest)[..24].to_string()
}

/// Analyzes `baseline` vs `final_cues` locally and produces Level 2A metrics, optional Level 2B
/// K-anonymous lowercase n-gram diffs, and optional Level 3 PII-sanitized training pairs.
pub fn analyze_export_diff(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    options: &JobOptions,
    monthly_salt: &str,
    include_ngrams: bool,
    include_training_pairs: bool,
    condense_cps_used: bool,
) -> ExportDiffReport {
    let alignment = align_cues(baseline, final_cues);
    let metrics = compute_metrics(baseline, final_cues, options, &alignment, condense_cps_used);

    let ngram_diffs = if include_ngrams {
        extract_safe_ngram_diffs(baseline, final_cues, options, monthly_salt, &alignment)
    } else {
        Vec::new()
    };

    let training_pairs = if include_training_pairs {
        sanitize_training_cue_pairs(baseline, final_cues, options, &alignment)
    } else {
        Vec::new()
    };

    ExportDiffReport {
        metrics,
        ngram_diffs,
        training_pairs,
    }
}

/// Extracts privacy-filtered `1..=3` word lowercase lexical diffs from 1:1 matched cues.
pub fn extract_safe_ngram_diffs(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    options: &JobOptions,
    monthly_salt: &str,
    alignment: &CueAlignment,
) -> Vec<SafeNgramDiff> {
    let lang_pair = format_lang_pair(options);
    let glossary_terms = collect_glossary_lowercase_terms(&options.glossary);
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();

    for &(b_idx, f_idx) in &alignment.one_to_one {
        let old_raw = visible_baseline_text(&baseline[b_idx]);
        let new_raw = visible_final_text(&final_cues[f_idx]);
        if normalize_lines(&old_raw) == normalize_lines(&new_raw) {
            continue;
        }

        let old_latin = to_latin(&old_raw);
        let new_latin = to_latin(&new_raw);
        let old_tokens = tokenize_preserving_case(&old_latin);
        let new_tokens = tokenize_preserving_case(&new_latin);
        if old_tokens.is_empty() && new_tokens.is_empty() {
            continue;
        }

        let matches = lcs_matching_indices(&old_tokens, &new_tokens);
        let max_len = old_tokens.len().max(new_tokens.len());
        if max_len == 0 || matches.len() * 2 < max_len {
            continue;
        }

        for (old_slice, new_slice) in extract_diff_hunks(&old_tokens, &new_tokens, &matches) {
            if !(1..=MAX_NGRAM_SPAN_WORDS).contains(&old_slice.len())
                || new_slice.len() > MAX_NGRAM_SPAN_WORDS
            {
                continue;
            }
            if !is_safe_ngram_slice(old_slice, &glossary_terms)
                || !is_safe_ngram_slice(new_slice, &glossary_terms)
            {
                continue;
            }

            let old_span = old_slice.join(" ");
            let new_span = new_slice.join(" ");
            if old_span == new_span
                || glossary_terms.contains(&old_span)
                || (!new_span.is_empty() && glossary_terms.contains(&new_span))
            {
                continue;
            }

            if seen.insert((old_span.clone(), new_span.clone())) {
                let cohort_proof =
                    build_monthly_cohort_proof(monthly_salt, &lang_pair, &old_span, &new_span);
                out.push(SafeNgramDiff {
                    lang_pair: lang_pair.clone(),
                    old_span,
                    new_span,
                    cohort_proof,
                });
            }
        }
    }

    out
}

/// Collects and sanitizes up to 25 modified cue pairs with PII stripping and project-wide
/// proper-noun masking (`[NAME]`, `[NUM]`, `[EMAIL]`, `[URL]`, `[PHONE]`, `[PATH]`).
pub fn sanitize_training_cue_pairs(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    options: &JobOptions,
    alignment: &CueAlignment,
) -> Vec<TrainingCuePair> {
    let candidates: Vec<(usize, usize)> = alignment
        .one_to_one
        .iter()
        .copied()
        .filter(|&(b_idx, f_idx)| is_non_trivial_training_edit(&baseline[b_idx], &final_cues[f_idx]))
        .take(MAX_TRAINING_PAIRS)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    let proper_noun_index = build_project_proper_noun_index(baseline, final_cues, &options.glossary);
    let glossary_phrases = collect_glossary_phrases_sorted(&options.glossary);
    let style = match options.translation_style {
        TranslationStyle::Natural => "natural",
        TranslationStyle::Formal => "formal",
    }
    .to_string();
    let address_form = match options.address_form {
        AddressForm::Auto => "auto",
        AddressForm::PoliteVi => "politeVi",
        AddressForm::InformalTi => "informalTi",
    }
    .to_string();

    candidates
        .into_iter()
        .map(|(b_idx, f_idx)| {
            let b = &baseline[b_idx];
            let f = &final_cues[f_idx];
            let source_raw = if !f.source_text.trim().is_empty() {
                f.source_text.as_str()
            } else {
                b.source_text.as_str()
            };
            TrainingCuePair {
                source_lang: options.source_language.clone(),
                target_lang: options.target_language.clone(),
                style: style.clone(),
                address_form: address_form.clone(),
                source_text: sanitize_training_text(
                    source_raw,
                    &glossary_phrases,
                    &proper_noun_index,
                ),
                initial_ai_text: sanitize_training_text(
                    &b.translated_text,
                    &glossary_phrases,
                    &proper_noun_index,
                ),
                final_human_text: sanitize_training_text(
                    &f.translated_text,
                    &glossary_phrases,
                    &proper_noun_index,
                ),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// HMAC-SHA256
// ---------------------------------------------------------------------------

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        normalized_key[..32].copy_from_slice(&hashed);
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= normalized_key[i];
        opad[i] ^= normalized_key[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

// ---------------------------------------------------------------------------
// Interval-Aware Alignment
// ---------------------------------------------------------------------------

/// Alignment groups between baseline cues and final exported cues.
#[derive(Debug, Clone, Default)]
pub struct CueAlignment {
    pub one_to_one: Vec<(usize, usize)>,
    pub splits: Vec<(usize, Vec<usize>)>,
    pub merges: Vec<(Vec<usize>, usize)>,
}

fn cue_overlap_ms(b_start: u64, b_end: u64, f_start: u64, f_end: u64) -> u64 {
    let b_hi = b_end.max(b_start);
    let f_hi = f_end.max(f_start);
    f_hi.min(b_hi).saturating_sub(f_start.max(b_start))
}

fn cue_iou(b_start: u64, b_end: u64, f_start: u64, f_end: u64) -> f64 {
    let b_hi = b_end.max(b_start);
    let f_hi = f_end.max(f_start);
    let overlap = cue_overlap_ms(b_start, b_hi, f_start, f_hi);
    let union = f_hi.max(b_hi).saturating_sub(f_start.min(b_start));
    if union == 0 {
        if b_start == f_start {
            1.0
        } else {
            0.0
        }
    } else {
        overlap as f64 / union as f64
    }
}

fn overlap_ratio(overlap: u64, start: u64, end: u64) -> f64 {
    let dur = end.saturating_sub(start);
    if dur == 0 {
        0.0
    } else {
        overlap as f64 / dur as f64
    }
}

fn align_cues(baseline: &[AiBaselineCue], final_cues: &[SubtitleCue]) -> CueAlignment {
    let b_len = baseline.len();
    let f_len = final_cues.len();
    if b_len == 0 || f_len == 0 {
        return CueAlignment::default();
    }

    let mut b_matched = vec![false; b_len];
    let mut f_matched = vec![false; f_len];
    let mut alignment = CueAlignment::default();

    // Count strong multi-claims so asymmetric splits/merges aren't prematurely locked as 1:1.
    let mut b_strong_f_claims = vec![0usize; b_len];
    let mut f_strong_b_claims = vec![0usize; f_len];
    for (bi, b) in baseline.iter().enumerate() {
        for (fi, f) in final_cues.iter().enumerate() {
            let ov = cue_overlap_ms(b.start_ms, b.end_ms, f.start_ms, f.end_ms);
            if ov == 0 {
                continue;
            }
            if overlap_ratio(ov, f.start_ms, f.end_ms) >= 0.45 {
                b_strong_f_claims[bi] += 1;
            }
            if overlap_ratio(ov, b.start_ms, b.end_ms) >= 0.45 {
                f_strong_b_claims[fi] += 1;
            }
        }
    }

    // Pass 1: Mutual best IoU >= 0.5 without competing split/merge claims.
    let mut best_f_for_b: Vec<Option<(usize, f64)>> = vec![None; b_len];
    let mut best_b_for_f: Vec<Option<(usize, f64)>> = vec![None; f_len];

    for (bi, b) in baseline.iter().enumerate() {
        for (fi, f) in final_cues.iter().enumerate() {
            let iou = cue_iou(b.start_ms, b.end_ms, f.start_ms, f.end_ms);
            if iou >= 0.5 {
                if best_f_for_b[bi].is_none_or(|(_, best)| iou > best) {
                    best_f_for_b[bi] = Some((fi, iou));
                }
                if best_b_for_f[fi].is_none_or(|(_, best)| iou > best) {
                    best_b_for_f[fi] = Some((bi, iou));
                }
            }
        }
    }

    for (bi, maybe_f) in best_f_for_b.iter().enumerate() {
        if let Some((fi, _)) = *maybe_f {
            if b_strong_f_claims[bi] <= 1
                && f_strong_b_claims[fi] <= 1
                && best_b_for_f[fi].is_some_and(|(best_b, _)| best_b == bi)
            {
                b_matched[bi] = true;
                f_matched[fi] = true;
                alignment.one_to_one.push((bi, fi));
            }
        }
    }

    // Pass 2A: 1:N Splits (one unmatched baseline cue claims >= 2 unmatched final cues).
    let mut best_b_by_overlap_for_f: Vec<Option<usize>> = vec![None; f_len];
    for (fi, f) in final_cues.iter().enumerate() {
        if f_matched[fi] {
            continue;
        }
        let mut best: Option<(usize, u64)> = None;
        for (bi, b) in baseline.iter().enumerate() {
            let ov = cue_overlap_ms(b.start_ms, b.end_ms, f.start_ms, f.end_ms);
            if ov > 0 && best.is_none_or(|(_, best_ov)| ov > best_ov) {
                best = Some((bi, ov));
            }
        }
        if let Some((bi, ov)) = best {
            if !b_matched[bi] && overlap_ratio(ov, f.start_ms, f.end_ms) >= 0.35 {
                best_b_by_overlap_for_f[fi] = Some(bi);
            }
        }
    }

    let mut split_groups = Vec::new();
    for (bi, &is_matched) in b_matched.iter().enumerate() {
        if is_matched {
            continue;
        }
        let children: Vec<usize> = (0..f_len)
            .filter(|&fi| !f_matched[fi] && best_b_by_overlap_for_f[fi] == Some(bi))
            .collect();
        if children.len() >= 2 {
            split_groups.push((bi, children));
        }
    }
    for (bi, children) in split_groups {
        b_matched[bi] = true;
        for &fi in &children {
            f_matched[fi] = true;
        }
        alignment.splits.push((bi, children));
    }

    // Pass 2B: N:1 Merges (one unmatched final cue claims >= 2 unmatched baseline cues).
    let mut best_f_by_overlap_for_b: Vec<Option<usize>> = vec![None; b_len];
    for (bi, b) in baseline.iter().enumerate() {
        if b_matched[bi] {
            continue;
        }
        let mut best: Option<(usize, u64)> = None;
        for (fi, f) in final_cues.iter().enumerate() {
            let ov = cue_overlap_ms(b.start_ms, b.end_ms, f.start_ms, f.end_ms);
            if ov > 0 && best.is_none_or(|(_, best_ov)| ov > best_ov) {
                best = Some((fi, ov));
            }
        }
        if let Some((fi, ov)) = best {
            if !f_matched[fi] && overlap_ratio(ov, b.start_ms, b.end_ms) >= 0.35 {
                best_f_by_overlap_for_b[bi] = Some(fi);
            }
        }
    }

    let mut merge_groups = Vec::new();
    for (fi, &is_matched) in f_matched.iter().enumerate() {
        if is_matched {
            continue;
        }
        let parents: Vec<usize> = (0..b_len)
            .filter(|&bi| !b_matched[bi] && best_f_by_overlap_for_b[bi] == Some(fi))
            .collect();
        if parents.len() >= 2 {
            merge_groups.push((parents, fi));
        }
    }
    for (parents, fi) in merge_groups {
        f_matched[fi] = true;
        for &bi in &parents {
            b_matched[bi] = true;
        }
        alignment.merges.push((parents, fi));
    }

    // Pass 3: Unique best overlap fallback for remaining 1:1 cues (e.g. heavy timeline trims).
    let mut rem_best_f_for_b: Vec<Option<(usize, u64)>> = vec![None; b_len];
    let mut rem_best_b_for_f: Vec<Option<(usize, u64)>> = vec![None; f_len];
    for (bi, b) in baseline.iter().enumerate() {
        if b_matched[bi] {
            continue;
        }
        for (fi, f) in final_cues.iter().enumerate() {
            if f_matched[fi] {
                continue;
            }
            let ov = cue_overlap_ms(b.start_ms, b.end_ms, f.start_ms, f.end_ms);
            if ov > 0 {
                if rem_best_f_for_b[bi].is_none_or(|(_, best)| ov > best) {
                    rem_best_f_for_b[bi] = Some((fi, ov));
                }
                if rem_best_b_for_f[fi].is_none_or(|(_, best)| ov > best) {
                    rem_best_b_for_f[fi] = Some((bi, ov));
                }
            }
        }
    }

    for (bi, maybe_f) in rem_best_f_for_b.iter().enumerate() {
        if b_matched[bi] {
            continue;
        }
        if let Some((fi, _)) = *maybe_f {
            if !f_matched[fi] && rem_best_b_for_f[fi].is_some_and(|(best_b, _)| best_b == bi) {
                b_matched[bi] = true;
                f_matched[fi] = true;
                alignment.one_to_one.push((bi, fi));
            }
        }
    }

    alignment.one_to_one.sort_by_key(|&(bi, _)| bi);
    alignment.splits.sort_by_key(|(bi, _)| *bi);
    alignment.merges.sort_by_key(|(_, fi)| *fi);
    alignment
}

// ---------------------------------------------------------------------------
// Level 2A Metrics & Classifiers
// ---------------------------------------------------------------------------

fn visible_baseline_text(cue: &AiBaselineCue) -> String {
    if !cue.translated_text.trim().is_empty() {
        cue.translated_text.clone()
    } else {
        cue.source_text.clone()
    }
}

fn visible_final_text(cue: &SubtitleCue) -> String {
    if !cue.translated_text.trim().is_empty() {
        cue.translated_text.clone()
    } else {
        cue.source_text.clone()
    }
}

fn normalize_lines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string()
}

fn compute_text_metrics(visible: &str, start_ms: u64, end_ms: u64, rules: &SegmentRules) -> (f64, bool, bool, usize) {
    let normalized = normalize_lines(visible);
    let char_count = normalized.chars().filter(|&c| c != '\n').count();
    let duration_ms = end_ms.saturating_sub(start_ms);

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

    let high_cps = cps > rules.max_cps;
    let max_chars = rules.max_chars_per_line as usize;
    let line_too_long = !normalized.is_empty()
        && normalized
            .split('\n')
            .any(|line| line.chars().count() > max_chars);

    (cps, high_cps, line_too_long, char_count)
}

fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

fn compute_metrics(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    options: &JobOptions,
    alignment: &CueAlignment,
    condense_cps_used: bool,
) -> ExportDiffMetrics {
    let rules = &options.segment_rules;

    let mut initial_high_cps_count = 0u32;
    let mut initial_line_too_long_count = 0u32;
    let mut cps_before_sum = 0.0f64;
    for b in baseline {
        let vis = visible_baseline_text(b);
        let (cps, high_cps, line_too_long, _) =
            compute_text_metrics(&vis, b.start_ms, b.end_ms, rules);
        cps_before_sum += cps;
        if high_cps {
            initial_high_cps_count += 1;
        }
        if line_too_long {
            initial_line_too_long_count += 1;
        }
    }

    let mut final_high_cps_count = 0u32;
    let mut final_line_too_long_count = 0u32;
    let mut cps_after_sum = 0.0f64;
    for f in final_cues {
        let vis = visible_final_text(f);
        let (cps, high_cps, line_too_long, _) =
            compute_text_metrics(&vis, f.start_ms, f.end_ms, rules);
        cps_after_sum += cps;
        if high_cps {
            final_high_cps_count += 1;
        }
        if line_too_long {
            final_line_too_long_count += 1;
        }
    }

    let (boundary_orphan_shifted_count, shifted_donor_b_indices) =
        count_boundary_orphan_shifts(baseline, final_cues, &alignment.one_to_one);

    let mut unchanged_cues_count = 0u32;
    let mut text_edited_cues_count = 0u32;
    let mut enclitic_reordered_count = 0u32;
    let mut redundant_possessive_removed_count = 0u32;
    let mut passive_calque_removed_count = 0u32;
    let mut conjunction_comma_fixed_count = 0u32;
    let mut missed_filler_deleted_count = 0u32;
    let mut inflection_suffix_fix_count = 0u32;

    let mut start_trim_count = 0u32;
    let mut start_delta_sum = 0.0f64;
    let mut end_trim_count = 0u32;
    let mut end_delta_sum = 0.0f64;

    for &(b_idx, f_idx) in &alignment.one_to_one {
        let b = &baseline[b_idx];
        let f = &final_cues[f_idx];

        let start_delta = f.start_ms as i64 - b.start_ms as i64;
        if start_delta.abs() >= TIMELINE_TRIM_THRESHOLD_MS {
            start_trim_count += 1;
            start_delta_sum += start_delta as f64;
        }
        let end_delta = f.end_ms as i64 - b.end_ms as i64;
        if end_delta.abs() >= TIMELINE_TRIM_THRESHOLD_MS {
            end_trim_count += 1;
            end_delta_sum += end_delta as f64;
        }

        let old_vis = normalize_lines(&visible_baseline_text(b));
        let new_vis = normalize_lines(&visible_final_text(f));
        if old_vis == new_vis {
            unchanged_cues_count += 1;
            continue;
        }

        text_edited_cues_count += 1;
        let old_latin = to_latin(&old_vis);
        let new_latin = to_latin(&new_vis);

        if is_enclitic_reordered(&old_latin, &new_latin) {
            enclitic_reordered_count += 1;
        }
        if is_redundant_possessive_removed(&old_latin, &new_latin) {
            redundant_possessive_removed_count += 1;
        }
        if is_passive_calque_removed(&old_latin, &new_latin) {
            passive_calque_removed_count += 1;
        }
        if is_conjunction_comma_fixed(&old_latin, &new_latin) {
            conjunction_comma_fixed_count += 1;
        }
        if !shifted_donor_b_indices.contains(&b_idx)
            && is_missed_filler_deleted(&old_latin, &new_latin)
        {
            missed_filler_deleted_count += 1;
        }
        inflection_suffix_fix_count += count_inflection_suffix_fixes(&old_latin, &new_latin);
    }

    // Splits
    let split_count = alignment.splits.len() as u32;
    let mut split_after_comma_count = 0u32;
    let mut split_after_conjunction_count = 0u32;
    let mut pre_split_chars_sum = 0.0f64;
    let mut pre_split_dur_sum = 0.0f64;

    for (b_idx, children) in &alignment.splits {
        let b = &baseline[*b_idx];
        let vis = normalize_lines(&visible_baseline_text(b));
        pre_split_chars_sum += vis.chars().filter(|&c| c != '\n').count() as f64;
        pre_split_dur_sum += b.end_ms.saturating_sub(b.start_ms) as f64;

        for pair in children.windows(2) {
            let left = to_latin(&normalize_lines(&visible_final_text(&final_cues[pair[0]])));
            let right = to_latin(&normalize_lines(&visible_final_text(&final_cues[pair[1]])));
            if left.trim_end().ends_with(',') {
                split_after_comma_count += 1;
            }
            if is_split_at_conjunction(&left, &right) {
                split_after_conjunction_count += 1;
            }
        }
    }

    // Merges
    let merge_count = alignment.merges.len() as u32;
    let mut merged_short_dur_sum = 0.0f64;
    let mut merged_short_chars_sum = 0.0f64;
    let mut merged_gap_sum = 0.0f64;
    let mut merged_gap_samples = 0u32;

    for (parents, _) in &alignment.merges {
        let mut min_dur = u64::MAX;
        let mut min_chars = usize::MAX;
        for &b_idx in parents {
            let b = &baseline[b_idx];
            let dur = b.end_ms.saturating_sub(b.start_ms);
            let chars = normalize_lines(&visible_baseline_text(b))
                .chars()
                .filter(|&c| c != '\n')
                .count();
            min_dur = min_dur.min(dur);
            min_chars = min_chars.min(chars);
        }
        if min_dur != u64::MAX {
            merged_short_dur_sum += min_dur as f64;
        }
        if min_chars != usize::MAX {
            merged_short_chars_sum += min_chars as f64;
        }
        for pair in parents.windows(2) {
            let gap = baseline[pair[1]]
                .start_ms
                .saturating_sub(baseline[pair[0]].end_ms);
            merged_gap_sum += gap as f64;
            merged_gap_samples += 1;
        }
    }

    let total_baseline_cues = baseline.len() as u32;
    let total_final_cues = final_cues.len() as u32;
    let edit_rate_pct = if total_baseline_cues == 0 {
        0.0
    } else {
        round2(
            (total_baseline_cues.saturating_sub(unchanged_cues_count) as f64
                / total_baseline_cues as f64)
                * 100.0,
        )
    };

    ExportDiffMetrics {
        total_baseline_cues,
        total_final_cues,
        unchanged_cues_count,
        text_edited_cues_count,
        edit_rate_pct,
        initial_high_cps_count,
        final_high_cps_count,
        initial_line_too_long_count,
        final_line_too_long_count,
        avg_cps_before: if total_baseline_cues == 0 {
            0.0
        } else {
            round2(cps_before_sum / total_baseline_cues as f64)
        },
        avg_cps_after: if total_final_cues == 0 {
            0.0
        } else {
            round2(cps_after_sum / total_final_cues as f64)
        },
        condense_cps_used,
        enclitic_reordered_count,
        redundant_possessive_removed_count,
        passive_calque_removed_count,
        conjunction_comma_fixed_count,
        missed_filler_deleted_count,
        inflection_suffix_fix_count,
        boundary_orphan_shifted_count,
        split_count,
        split_after_comma_count,
        split_after_conjunction_count,
        avg_pre_split_chars: if split_count == 0 {
            0.0
        } else {
            round2(pre_split_chars_sum / split_count as f64)
        },
        avg_pre_split_duration_ms: if split_count == 0 {
            0.0
        } else {
            round2(pre_split_dur_sum / split_count as f64)
        },
        merge_count,
        avg_merged_short_cue_duration_ms: if merge_count == 0 {
            0.0
        } else {
            round2(merged_short_dur_sum / merge_count as f64)
        },
        avg_merged_short_cue_chars: if merge_count == 0 {
            0.0
        } else {
            round2(merged_short_chars_sum / merge_count as f64)
        },
        avg_merged_gap_ms: if merged_gap_samples == 0 {
            0.0
        } else {
            round2(merged_gap_sum / merged_gap_samples as f64)
        },
        start_trim_count,
        avg_start_delta_ms: if start_trim_count == 0 {
            0.0
        } else {
            round2(start_delta_sum / start_trim_count as f64)
        },
        end_trim_count,
        avg_end_delta_ms: if end_trim_count == 0 {
            0.0
        } else {
            round2(end_delta_sum / end_trim_count as f64)
        },
    }
}

fn is_serbian_enclitic(word: &str) -> bool {
    matches!(
        word,
        "je" | "sam"
            | "si"
            | "smo"
            | "ste"
            | "su"
            | "ću"
            | "ćeš"
            | "će"
            | "ćemo"
            | "ćete"
            | "bih"
            | "bi"
            | "bismo"
            | "biste"
            | "se"
            | "mi"
            | "ti"
            | "mu"
            | "joj"
            | "nam"
            | "vam"
            | "im"
            | "ga"
            | "ju"
            | "ih"
            | "li"
    )
}

fn is_possessive_pronoun(word: &str) -> bool {
    matches!(
        word,
        "moj"
            | "moja"
            | "moje"
            | "moji"
            | "mojih"
            | "mojim"
            | "mojima"
            | "mojeg"
            | "mog"
            | "mom"
            | "mome"
            | "mojem"
            | "moju"
            | "mojom"
            | "tvoj"
            | "tvoja"
            | "tvoje"
            | "tvoji"
            | "tvojih"
            | "tvojim"
            | "tvojima"
            | "tvojeg"
            | "tvog"
            | "tvom"
            | "tvome"
            | "tvojem"
            | "tvoju"
            | "tvojom"
            | "njegov"
            | "njegova"
            | "njegovo"
            | "njegovi"
            | "njegovih"
            | "njegovim"
            | "njegovima"
            | "njegovog"
            | "njegovom"
            | "njegovome"
            | "njegovu"
            | "njen"
            | "njena"
            | "njeno"
            | "njeni"
            | "njenih"
            | "njenim"
            | "njenima"
            | "njenog"
            | "njenom"
            | "njenome"
            | "njenu"
            | "njezin"
            | "njezina"
            | "njezino"
            | "njezinog"
            | "naš"
            | "naša"
            | "naše"
            | "naši"
            | "naših"
            | "našim"
            | "našima"
            | "našeg"
            | "našem"
            | "našoj"
            | "našu"
            | "našom"
            | "vaš"
            | "vaša"
            | "vaše"
            | "vaši"
            | "vaših"
            | "vašim"
            | "vašima"
            | "vašeg"
            | "vašem"
            | "vašoj"
            | "vašu"
            | "vašom"
            | "njihov"
            | "njihova"
            | "njihovo"
            | "njihovi"
            | "njihovih"
            | "njihovim"
            | "njihovima"
            | "njihovog"
            | "njihovom"
            | "njihovome"
            | "njihovu"
    )
}

fn is_conjunction_word(word: &str) -> bool {
    matches!(
        word,
        "i" | "a"
            | "ali"
            | "pa"
            | "te"
            | "ni"
            | "niti"
            | "ili"
            | "jer"
            | "da"
            | "kad"
            | "kada"
            | "dok"
            | "ako"
            | "mada"
            | "iako"
            | "pošto"
            | "zato"
            | "nego"
            | "već"
            | "and"
            | "but"
            | "or"
            | "so"
            | "because"
            | "when"
            | "while"
            | "if"
    )
}

fn tokenize_lower_words(text: &str) -> Vec<String> {
    tokenize_preserving_case(text)
        .into_iter()
        .map(|t| t.to_lowercase())
        .collect()
}

fn is_enclitic_reordered(old_latin: &str, new_latin: &str) -> bool {
    let old_toks = tokenize_lower_words(old_latin);
    let new_toks = tokenize_lower_words(new_latin);
    if old_toks.len() < 2 || old_toks.len() != new_toks.len() || old_toks == new_toks {
        return false;
    }

    let mut old_sorted = old_toks.clone();
    let mut new_sorted = new_toks.clone();
    old_sorted.sort();
    new_sorted.sort();
    if old_sorted != new_sorted {
        return false;
    }

    old_toks
        .iter()
        .zip(new_toks.iter())
        .any(|(a, b)| a != b && (is_serbian_enclitic(a) || is_serbian_enclitic(b)))
}

fn is_redundant_possessive_removed(old_latin: &str, new_latin: &str) -> bool {
    let old_toks = tokenize_lower_words(old_latin);
    let new_toks = tokenize_lower_words(new_latin);
    let old_poss = old_toks
        .iter()
        .filter(|w| is_possessive_pronoun(w.as_str()))
        .count();
    let new_poss = new_toks
        .iter()
        .filter(|w| is_possessive_pronoun(w.as_str()))
        .count();
    old_poss > new_poss
}

fn is_passive_calque_removed(old_latin: &str, new_latin: &str) -> bool {
    let old_toks = tokenize_lower_words(old_latin);
    let new_toks = tokenize_lower_words(new_latin);
    let has_od_strane = |toks: &[String]| {
        toks.windows(2)
            .any(|w| w[0] == "od" && w[1] == "strane")
    };
    if has_od_strane(&old_toks) && !has_od_strane(&new_toks) {
        return true;
    }
    let has_biva_passive = |toks: &[String]| {
        toks.windows(2).any(|w| {
            (w[0] == "biva" || w[0] == "bivaju")
                && (w[1].ends_with("an")
                    || w[1].ends_with("en")
                    || w[1].ends_with("jen")
                    || w[1].ends_with("ven")
                    || w[1].ends_with("ano")
                    || w[1].ends_with("eno")
                    || w[1].ends_with("jeno"))
        })
    };
    has_biva_passive(&old_toks) && !has_biva_passive(&new_toks)
}

fn conjunction_comma_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[.!?]\s+|(?:—|-)\s*)(tako da|međutim|zapravo|ustvari|dakle|ali|pa|zato|i|a),\s+")
            .expect("valid conjunction comma regex")
    })
}

fn is_conjunction_comma_fixed(old_latin: &str, new_latin: &str) -> bool {
    let re = conjunction_comma_regex();
    let old_matches = re.find_iter(old_latin).count();
    if old_matches == 0 {
        return false;
    }
    let new_matches = re.find_iter(new_latin).count();
    if new_matches >= old_matches {
        return false;
    }
    // Verify the connector word itself remained in `new_latin` (i.e. comma was stripped, not whole clause deleted).
    for cap in re.captures_iter(old_latin) {
        if let Some(conn) = cap.get(1) {
            let conn_lower = conn.as_str().to_lowercase();
            let has_conn_no_comma = new_latin
                .to_lowercase()
                .contains(&format!("{conn_lower} "));
            if has_conn_no_comma {
                return true;
            }
        }
    }
    false
}

fn is_missed_filler_deleted(old_latin: &str, new_latin: &str) -> bool {
    let old_toks = tokenize_lower_words(old_latin);
    let new_toks = tokenize_lower_words(new_latin);
    if old_toks.len() != new_toks.len() + 1 || new_toks.is_empty() {
        return false;
    }
    // Check `new_toks` is an exact ordered subsequence of `old_toks` with 1 omitted token.
    let mut ni = 0usize;
    let mut skipped: Option<&str> = None;
    for old_w in &old_toks {
        if ni < new_toks.len() && old_w == &new_toks[ni] {
            ni += 1;
        } else if skipped.is_none() {
            skipped = Some(old_w.as_str());
        } else {
            return false;
        }
    }
    if ni != new_toks.len() {
        return false;
    }
    skipped.is_some_and(|w| !is_possessive_pronoun(w))
}

fn common_char_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(ca, cb)| ca == cb).count()
}

fn count_inflection_suffix_fixes(old_latin: &str, new_latin: &str) -> u32 {
    let old_toks = tokenize_preserving_case(old_latin);
    let new_toks = tokenize_preserving_case(new_latin);
    if old_toks.is_empty() || new_toks.is_empty() {
        return 0;
    }

    let matches = lcs_matching_indices(&old_toks, &new_toks);
    let mut count = 0u32;
    for (old_slice, new_slice) in extract_diff_hunks(&old_toks, &new_toks, &matches) {
        if old_slice.len() == 1 && new_slice.len() == 1 {
            let a = &old_slice[0];
            let b = &new_slice[0];
            let all_lower = !a.chars().any(|c| c.is_uppercase())
                && !b.chars().any(|c| c.is_uppercase())
                && a.chars().all(|c| c.is_alphabetic())
                && b.chars().all(|c| c.is_alphabetic());
            if all_lower && a != b && common_char_prefix_len(a, b) >= MIN_INFLECTION_PREFIX_CHARS {
                count += 1;
            }
        }
    }
    count
}

fn count_boundary_orphan_shifts(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    one_to_one: &[(usize, usize)],
) -> (u32, HashSet<usize>) {
    let mut count = 0u32;
    let mut donor_b_indices = HashSet::new();
    for pair in one_to_one.windows(2) {
        let (b0_idx, f0_idx) = pair[0];
        let (b1_idx, f1_idx) = pair[1];
        if b1_idx != b0_idx + 1 || f1_idx != f0_idx + 1 {
            continue;
        }
        let b0 = tokenize_lower_words(&to_latin(&visible_baseline_text(&baseline[b0_idx])));
        let b1 = tokenize_lower_words(&to_latin(&visible_baseline_text(&baseline[b1_idx])));
        let f0 = tokenize_lower_words(&to_latin(&visible_final_text(&final_cues[f0_idx])));
        let f1 = tokenize_lower_words(&to_latin(&visible_final_text(&final_cues[f1_idx])));

        if b0.is_empty() || b1.is_empty() || f0.is_empty() || f1.is_empty() {
            continue;
        }

        // Last token of B0 moved to start of F1
        let shifted_right = b0.len() == f0.len() + 1
            && f1.len() == b1.len() + 1
            && b0[..f0.len()] == f0[..]
            && b0.last() == f1.first()
            && b1[..] == f1[1..];

        // First token of B1 moved to end of F0
        let shifted_left = f0.len() == b0.len() + 1
            && b1.len() == f1.len() + 1
            && f0[..b0.len()] == b0[..]
            && f0.last() == b1.first()
            && b1[1..] == f1[..];

        if shifted_right {
            count += 1;
            donor_b_indices.insert(b0_idx);
        } else if shifted_left {
            count += 1;
            donor_b_indices.insert(b1_idx);
        }
    }
    (count, donor_b_indices)
}

fn is_split_at_conjunction(left_latin: &str, right_latin: &str) -> bool {
    let left_toks = tokenize_lower_words(left_latin);
    let right_toks = tokenize_lower_words(right_latin);
    left_toks
        .last()
        .is_some_and(|w| is_conjunction_word(w.as_str()))
        || right_toks
            .first()
            .is_some_and(|w| is_conjunction_word(w.as_str()))
}

// ---------------------------------------------------------------------------
// Level 2B N-Gram Tokenization, LCS & Hard Privacy Filter
// ---------------------------------------------------------------------------

fn format_lang_pair(options: &JobOptions) -> String {
    let src = options.source_language.trim().to_lowercase();
    let dst = options.target_language.trim().to_lowercase();
    let src = if src.is_empty() { "auto" } else { src.as_str() };
    let dst = if dst.is_empty() { "sr" } else { dst.as_str() };
    format!("{src}-{dst}")
}

fn strip_outer_punctuation(tok: &str) -> &str {
    const OUTER_PUNCT: &[char] = &[
        '.', ',', '!', '?', ';', ':', '"', '(', ')', '[', ']', '{', '}', '«', '»', '„', '“', '”',
        '…', '—', '–', '-', '\'', '’',
    ];
    tok.trim_matches(|c: char| OUTER_PUNCT.contains(&c))
}

fn tokenize_preserving_case(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(strip_outer_punctuation)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn lcs_matching_indices(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut pairs = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

fn extract_diff_hunks<'a>(
    old_tokens: &'a [String],
    new_tokens: &'a [String],
    matches: &[(usize, usize)],
) -> Vec<(&'a [String], &'a [String])> {
    let mut hunks = Vec::new();
    let mut prev_old = 0usize;
    let mut prev_new = 0usize;

    for &(oi, ni) in matches {
        if prev_old < oi || prev_new < ni {
            hunks.push((&old_tokens[prev_old..oi], &new_tokens[prev_new..ni]));
        }
        prev_old = oi + 1;
        prev_new = ni + 1;
    }

    if prev_old < old_tokens.len() || prev_new < new_tokens.len() {
        hunks.push((&old_tokens[prev_old..], &new_tokens[prev_new..]));
    }
    hunks
}

fn collect_glossary_lowercase_terms(glossary: &BTreeMap<String, String>) -> HashSet<String> {
    let mut terms = HashSet::new();
    for (k, v) in glossary {
        for raw in [k, v] {
            let norm = to_latin(raw).trim().to_lowercase();
            if !norm.is_empty() {
                terms.insert(norm.clone());
                for tok in tokenize_lower_words(&norm) {
                    if tok.chars().count() >= 2 {
                        terms.insert(tok);
                    }
                }
            }
        }
    }
    terms
}

fn is_safe_ngram_slice(slice: &[String], glossary_terms: &HashSet<String>) -> bool {
    if slice.is_empty() {
        return true;
    }

    let single_word = slice.len() == 1;
    for tok in slice {
        let char_len = tok.chars().count();
        if char_len > MAX_NGRAM_TOKEN_CHARS {
            return false;
        }
        if single_word && char_len < MIN_NGRAM_SINGLE_TOKEN_CHARS {
            return false;
        }
        let lower = tok.to_lowercase();
        if lower.contains(".com") || glossary_terms.contains(&lower) {
            return false;
        }
        if tok.chars().any(|c| {
            c.is_uppercase()
                || c.is_numeric()
                || matches!(c, '@' | '/' | '\\')
                || !(c.is_alphabetic() || matches!(c, '-' | '\'' | '’'))
        }) {
            return false;
        }
        if !tok.chars().any(|c| c.is_alphabetic()) {
            return false;
        }
    }

    let joined = slice.join(" ");
    !glossary_terms.contains(&joined)
}

// ---------------------------------------------------------------------------
// Level 3 PII & Project-Wide Proper Noun Sanitizer (V05-M2)
// ---------------------------------------------------------------------------

fn is_non_trivial_training_edit(baseline: &AiBaselineCue, final_cue: &SubtitleCue) -> bool {
    let source = if !final_cue.source_text.trim().is_empty() {
        final_cue.source_text.trim()
    } else {
        baseline.source_text.trim()
    };
    let ai = normalize_lines(&baseline.translated_text);
    let human = normalize_lines(&final_cue.translated_text);
    !source.is_empty() && !ai.is_empty() && !human.is_empty() && ai != human
}

fn is_ignored_capitalized_pronoun_or_abbrev(token: &str) -> bool {
    matches!(
        token,
        "I" | "I'm"
            | "I've"
            | "I'll"
            | "I'd"
            | "I’m"
            | "I’ve"
            | "I’ll"
            | "I’d"
            | "Vi"
            | "Vas"
            | "Vam"
            | "Vama"
            | "Vaš"
            | "Vaša"
            | "Vaše"
            | "Vaši"
            | "Vašeg"
            | "Vašem"
            | "Vašim"
            | "Vaših"
            | "Vašoj"
            | "Vašu"
            | "Vašom"
            | "Ви"
            | "Вас"
            | "Вам"
            | "Вама"
            | "Ваш"
            | "Ваша"
            | "Ваше"
            | "Ваши"
            | "Вашег"
            | "Вашем"
            | "Вашим"
            | "Ваших"
            | "Вашој"
            | "Вашу"
            | "Вашом"
            | "OK"
            | "TV"
    )
}

fn add_capitalized_variants_to_index(raw_token: &str, index: &mut HashSet<String>) {
    let cleaned = strip_outer_punctuation(raw_token);
    if cleaned.chars().count() < 2 {
        return;
    }
    let first_upper = cleaned.chars().next().is_some_and(|c| c.is_uppercase());
    if !first_upper || is_ignored_capitalized_pronoun_or_abbrev(cleaned) {
        return;
    }

    // Strip English possessive `'s` / `’s` so `Marko's` indexes `Marko` too.
    let base = cleaned
        .strip_suffix("'s")
        .or_else(|| cleaned.strip_suffix("’s"))
        .unwrap_or(cleaned);

    if base.chars().count() >= 2 && !is_ignored_capitalized_pronoun_or_abbrev(base) {
        index.insert(base.to_string());
        let latin = to_latin(base);
        if !latin.is_empty() {
            index.insert(latin);
        }
    }
}

fn scan_mid_sentence_capitalized_tokens(text: &str, index: &mut HashSet<String>) {
    // Split on sentence boundaries (`!`, `?`, `\n`, or `.` followed by whitespace/end).
    let mut sentence_start = true;
    for raw in text.split_whitespace() {
        let stripped = strip_outer_punctuation(raw);
        let is_pure_dash = raw == "—" || raw == "-" || raw == "–";
        if is_pure_dash {
            continue;
        }

        if !sentence_start && !stripped.is_empty() {
            add_capitalized_variants_to_index(raw, index);
        }

        // Check if this token ends a sentence (next token will be sentence-initial).
        let trimmed_end = raw.trim_end_matches(['"', '\'', '’', '”', ')', ']']);
        sentence_start = trimmed_end.ends_with('.')
            || trimmed_end.ends_with('!')
            || trimmed_end.ends_with('?')
            || trimmed_end.ends_with('…');
    }
}

fn build_project_proper_noun_index(
    baseline: &[AiBaselineCue],
    final_cues: &[SubtitleCue],
    glossary: &BTreeMap<String, String>,
) -> HashSet<String> {
    let mut index = HashSet::new();

    for b in baseline {
        for line in b.source_text.lines().chain(b.translated_text.lines()) {
            scan_mid_sentence_capitalized_tokens(line, &mut index);
        }
    }
    for f in final_cues {
        for line in f.source_text.lines().chain(f.translated_text.lines()) {
            scan_mid_sentence_capitalized_tokens(line, &mut index);
        }
    }
    for (k, v) in glossary {
        for raw in [k, v] {
            for tok in raw.split_whitespace() {
                add_capitalized_variants_to_index(tok, &mut index);
            }
        }
    }

    index
}

fn collect_glossary_phrases_sorted(glossary: &BTreeMap<String, String>) -> Vec<String> {
    let mut set = HashSet::new();
    for (k, v) in glossary {
        for raw in [k, v] {
            let trimmed = raw.trim();
            if trimmed.chars().count() >= 2 {
                set.insert(trimmed.to_string());
            }
        }
    }
    let mut list: Vec<String> = set.into_iter().collect();
    list.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    list
}

struct TrainingSanitizerRegexes {
    sk_key: Regex,
    hf_token: Regex,
    gemini_key: Regex,
    gh_token: Regex,
    github_pat: Regex,
    bearer: Regex,
    api_key_kv: Regex,
    url: Regex,
    email: Regex,
    win_path: Regex,
    unix_path: Regex,
    phone_intl: Regex,
    phone_grouped: Regex,
    multi_digit: Regex,
}

fn training_sanitizer_regexes() -> &'static TrainingSanitizerRegexes {
    static REGEXES: OnceLock<TrainingSanitizerRegexes> = OnceLock::new();
    REGEXES.get_or_init(|| TrainingSanitizerRegexes {
        sk_key: Regex::new(r"\bsk-[A-Za-z0-9_\-]{8,}").expect("valid sk_key regex"),
        hf_token: Regex::new(r"\bhf_[A-Za-z0-9]{10,}").expect("valid hf_token regex"),
        gemini_key: Regex::new(r"\bAIza[0-9A-Za-z_\-]{20,}").expect("valid gemini_key regex"),
        gh_token: Regex::new(r"\bgh[pousr]_[A-Za-z0-9]+").expect("valid gh_token regex"),
        github_pat: Regex::new(r"\bgithub_pat_[A-Za-z0-9_]+").expect("valid github_pat regex"),
        bearer: Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._\-]+").expect("valid bearer regex"),
        api_key_kv: Regex::new(
            r#"(?i)(\b(?:x[_-])?api[_-]?key["']?\s*[:=]\s*["']?)[^\s"',}&;\]\)]+"#,
        )
        .expect("valid api_key_kv regex"),
        url: Regex::new(r"(?i)(?:https?://|www\.)[^\s<>]+").expect("valid url regex"),
        email: Regex::new(r"(?i)\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b")
            .expect("valid email regex"),
        win_path: Regex::new(r#"(?i)(?:[A-Za-z]:\\|\\\\)[^\r\n\t"'<>|]+(?:\\[^\r\n\t"'<>|\s]+)*"#)
            .expect("valid win_path regex"),
        unix_path: Regex::new(
            r#"(?:~/(?:[^\s"'<>]+)|(?:/(?:home|Users|var|tmp|etc|opt|mnt|srv)/[^\s"'<>]+))"#,
        )
        .expect("valid unix_path regex"),
        phone_intl: Regex::new(r"\+\d[\d\s\-().]{6,}\d").expect("valid phone_intl regex"),
        phone_grouped: Regex::new(r"\b(?:\(\d{2,4}\)|\d{2,4})[\s\-]\d{2,4}[\s\-]\d{2,4}(?:[\s\-]\d{1,4})?\b")
            .expect("valid phone_grouped regex"),
        multi_digit: Regex::new(r"\b\d[\d.,]*\d\b").expect("valid multi_digit regex"),
    })
}

fn is_placeholder_token(tok: &str) -> bool {
    matches!(
        tok,
        "[NAME]" | "[NUM]" | "[EMAIL]" | "[URL]" | "[PHONE]" | "[PATH]" | "[REDACTED]"
    )
}

fn replace_glossary_phrases_word_bounded(text: &str, phrases: &[String]) -> String {
    let mut out = text.to_string();
    for phrase in phrases {
        let escaped = regex::escape(phrase);
        let pattern = format!(r"(?i)(^|[^\p{{L}}\p{{N}}_]){escaped}([^\p{{L}}\p{{N}}_]|$)");
        if let Ok(re) = Regex::new(&pattern) {
            // Loop handles adjacent back-to-back occurrences sharing a boundary delimiter.
            loop {
                let next = re.replace_all(&out, "${1}[NAME]${2}").into_owned();
                if next == out {
                    break;
                }
                out = next;
            }
        }
    }
    out
}

fn mask_indexed_proper_nouns(text: &str, proper_noun_index: &HashSet<String>) -> String {
    if proper_noun_index.is_empty() {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch == '[' {
            // Preserve existing `[NAME]` / `[NUM]` / `[EMAIL]` / `[URL]` / `[PHONE]` / `[PATH]` / `[REDACTED]` spans.
            let rest = &text[start..];
            if let Some(close_rel) = rest.find(']') {
                let candidate = &rest[..=close_rel];
                if is_placeholder_token(candidate) {
                    out.push_str(candidate);
                    for _ in 0..candidate.chars().count() {
                        chars.next();
                    }
                    continue;
                }
            }
        }

        if ch.is_alphabetic() {
            let mut end = start + ch.len_utf8();
            chars.next();
            while let Some(&(idx, next_ch)) = chars.peek() {
                if next_ch.is_alphabetic()
                    || ((next_ch == '\'' || next_ch == '’')
                        && text[idx + next_ch.len_utf8()..].starts_with('s'))
                {
                    end = idx + next_ch.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            let (base, possessive_suffix) = if let Some(b) = word.strip_suffix("'s") {
                (b, "'s")
            } else if let Some(b) = word.strip_suffix("’s") {
                (b, "’s")
            } else {
                (word, "")
            };
            let latin = to_latin(base);
            if proper_noun_index.contains(base) || proper_noun_index.contains(&latin) {
                out.push_str("[NAME]");
                if !possessive_suffix.is_empty() {
                    out.push_str(possessive_suffix);
                }
            } else {
                out.push_str(word);
            }
        } else {
            out.push(ch);
            chars.next();
        }
    }

    out
}

fn sanitize_training_text(
    raw: &str,
    glossary_phrases: &[String],
    proper_noun_index: &HashSet<String>,
) -> String {
    let rx = training_sanitizer_regexes();
    let s = rx.sk_key.replace_all(raw, "[REDACTED]");
    let s = rx.hf_token.replace_all(&s, "[REDACTED]");
    let s = rx.gemini_key.replace_all(&s, "[REDACTED]");
    let s = rx.gh_token.replace_all(&s, "[REDACTED]");
    let s = rx.github_pat.replace_all(&s, "[REDACTED]");
    let s = rx.bearer.replace_all(&s, "Bearer [REDACTED]");
    let s = rx.api_key_kv.replace_all(&s, "${1}[REDACTED]");
    let s = rx.url.replace_all(&s, "[URL]");
    let s = rx.email.replace_all(&s, "[EMAIL]");
    let s = rx.win_path.replace_all(&s, "[PATH]");
    let s = rx.unix_path.replace_all(&s, "[PATH]");
    let s = rx.phone_intl.replace_all(&s, "[PHONE]");
    let s = rx.phone_grouped.replace_all(&s, "[PHONE]");
    let s = replace_glossary_phrases_word_bounded(&s, glossary_phrases);
    let s = mask_indexed_proper_nouns(&s, proper_noun_index);
    let s = rx.multi_digit.replace_all(&s, "[NUM]");
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SerbianScript;

    fn make_cue(id: u32, start_ms: u64, end_ms: u64, src: &str, dst: &str) -> SubtitleCue {
        SubtitleCue {
            id,
            start_ms,
            end_ms,
            source_text: src.to_string(),
            translated_text: dst.to_string(),
            translated_variants: BTreeMap::new(),
            cps: 12.0,
            warnings: Vec::new(),
        }
    }

    fn make_baseline(id: u32, start_ms: u64, end_ms: u64, src: &str, dst: &str) -> AiBaselineCue {
        AiBaselineCue {
            id,
            start_ms,
            end_ms,
            source_text: src.to_string(),
            translated_text: dst.to_string(),
        }
    }

    #[test]
    fn snapshot_baseline_cues_preserves_fields() {
        let cues = vec![make_cue(1, 100, 1500, "Hello world", "Zdravo svete")];
        let snap = snapshot_baseline_cues(&cues);
        assert_eq!(
            snap,
            vec![AiBaselineCue {
                id: 1,
                start_ms: 100,
                end_ms: 1500,
                source_text: "Hello world".to_string(),
                translated_text: "Zdravo svete".to_string(),
            }]
        );
    }

    #[test]
    fn camel_case_serialization_matches_schema() {
        let report = analyze_export_diff(
            &[make_baseline(1, 0, 2000, "Hi", "Ćao")],
            &[make_cue(1, 0, 2000, "Hi", "Zdravo")],
            &JobOptions::default(),
            "salt-2026-11",
            true,
            true,
            false,
        );
        let val = serde_json::to_value(&report).unwrap();
        assert!(val.get("metrics").unwrap().get("totalBaselineCues").is_some());
        assert!(val.get("metrics").unwrap().get("encliticReorderedCount").is_some());
        assert!(val.get("metrics").unwrap().get("avgMergedShortCueDurationMs").is_some());
        assert!(val.get("ngramDiffs").is_some());
        assert!(val.get("trainingPairs").is_some());
    }

    #[test]
    fn monthly_cohort_proof_is_deterministic_and_unlinkable() {
        let p1 = build_monthly_cohort_proof("salt-a", "en-sr", "tijekom", "tokom");
        let p2 = build_monthly_cohort_proof("salt-a", "en-sr", "tijekom", "tokom");
        let p3 = build_monthly_cohort_proof("salt-a", "en-sr", "glazba", "muzika");
        let p4 = build_monthly_cohort_proof("salt-b", "en-sr", "tijekom", "tokom");

        assert_eq!(p1.len(), 24);
        assert!(p1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        assert_eq!(p1, p2);
        assert_ne!(p1, p3);
        assert_ne!(p1, p4);
    }

    #[test]
    fn classifies_serbian_linguistic_fixes_and_timeline_trims() {
        let baseline = vec![
            // 1. Enclitic reordered
            make_baseline(1, 0, 2000, "He told me", "On rekao je meni"),
            // 2. Redundant possessive removed
            make_baseline(2, 2100, 4000, "I saw my friend", "Video sam mog prijatelja"),
            // 3. Passive calque removed
            make_baseline(3, 4100, 6500, "Written by him", "Napisano je od strane njega"),
            // 4. Conjunction comma fixed
            make_baseline(4, 6600, 8600, "So, we left", "Tako da, otišli smo"),
            // 5. Missed filler deleted
            make_baseline(5, 8700, 10700, "Well actually yes", "Pa zapravo da"),
            // 6. Inflection suffix fix + start/end trims (>= 40ms)
            make_baseline(6, 10800, 13000, "Worst days", "Od najstranije dana"),
            // 7 & 8. Boundary orphan shifted (trailing token moved to next cue)
            make_baseline(7, 13100, 15000, "First part and", "Prvi deo i"),
            make_baseline(8, 15100, 17000, "second part", "drugi deo"),
        ];

        let final_cues = vec![
            make_cue(1, 0, 2000, "He told me", "On je meni rekao"),
            make_cue(2, 2100, 4000, "I saw my friend", "Video sam prijatelja"),
            make_cue(3, 4100, 6500, "Written by him", "On je to napisao"),
            make_cue(4, 6600, 8600, "So, we left", "Tako da otišli smo"),
            make_cue(5, 8700, 10700, "Well actually yes", "Pa da"),
            make_cue(6, 10900, 12850, "Worst days", "Od najstrašnijih dana"),
            make_cue(7, 13100, 15000, "First part and", "Prvi deo"),
            make_cue(8, 15100, 17000, "second part", "i drugi deo"),
        ];

        let report = analyze_export_diff(
            &baseline,
            &final_cues,
            &JobOptions::default(),
            "salt",
            false,
            false,
            true,
        );
        let m = &report.metrics;
        assert_eq!(m.total_baseline_cues, 8);
        assert_eq!(m.total_final_cues, 8);
        assert_eq!(m.unchanged_cues_count, 0);
        assert_eq!(m.text_edited_cues_count, 8);
        assert_eq!(m.edit_rate_pct, 100.0);
        assert!(m.condense_cps_used);
        assert_eq!(m.enclitic_reordered_count, 1);
        assert_eq!(m.redundant_possessive_removed_count, 1);
        assert_eq!(m.passive_calque_removed_count, 1);
        assert_eq!(m.conjunction_comma_fixed_count, 1);
        assert_eq!(m.missed_filler_deleted_count, 1);
        assert_eq!(m.inflection_suffix_fix_count, 1);
        assert_eq!(m.boundary_orphan_shifted_count, 1);
        assert_eq!(m.start_trim_count, 1);
        assert_eq!(m.avg_start_delta_ms, 100.0);
        assert_eq!(m.end_trim_count, 1);
        assert_eq!(m.avg_end_delta_ms, -150.0);
    }

    #[test]
    fn detects_asymmetric_splits_and_merges_and_cps_reduction() {
        let mut opts = JobOptions::default();
        opts.segment_rules.max_cps = 17.0;
        opts.segment_rules.max_chars_per_line = 25;

        let baseline = vec![
            // Asymmetric 1:2 split (3.5s -> 1.0s + 2.5s) with comma + long line + high CPS
            make_baseline(
                1,
                0,
                3500,
                "Long sentence, and more words here",
                "Ovo je veoma dugačka rečenica, ali idemo dalje sada odmah.",
            ),
            // N:1 merge (two short cues with 200ms gap)
            make_baseline(2, 4000, 4600, "Wait", "Čekaj"),
            make_baseline(3, 4800, 5600, "a second.", "sekund."),
        ];

        let final_cues = vec![
            make_cue(1, 0, 1000, "Long sentence,", "Veoma duga rečenica,"),
            make_cue(2, 1000, 3500, "and more words here", "ali idemo dalje."),
            make_cue(3, 4000, 5600, "Wait a second.", "Čekaj sekund."),
        ];

        let report = analyze_export_diff(&baseline, &final_cues, &opts, "salt", false, false, false);
        let m = &report.metrics;
        assert_eq!(m.split_count, 1);
        assert_eq!(m.split_after_comma_count, 1);
        assert_eq!(m.split_after_conjunction_count, 1);
        assert_eq!(m.avg_pre_split_duration_ms, 3500.0);
        assert!(m.avg_pre_split_chars > 50.0);

        assert_eq!(m.merge_count, 1);
        assert_eq!(m.avg_merged_short_cue_duration_ms, 600.0);
        assert_eq!(m.avg_merged_short_cue_chars, 5.0);
        assert_eq!(m.avg_merged_gap_ms, 200.0);
        assert_eq!(m.initial_line_too_long_count, 1);
        assert_eq!(m.final_line_too_long_count, 0);
    }

    #[test]
    fn extract_safe_ngram_diffs_enforces_strict_privacy_barriers() {
        let mut opts = JobOptions {
            source_language: "en".to_string(),
            target_language: "sr".to_string(),
            script: SerbianScript::Cyrillic,
            ..JobOptions::default()
        };
        opts.glossary.insert("AcmeCorp".to_string(), "acmecorp".to_string());

        let baseline = vec![
            // Safe lowercase Cyrillic -> normalized to Latin ("tijekom" -> "tokom")
            make_baseline(1, 0, 2000, "During the meeting", "Током тијеком састанка у граду"),
            // Safe 1-word filler deletion ("zapravo")
            make_baseline(2, 2100, 4000, "We actually went home", "Mi smo zapravo otišli kući"),
            // REJECTED: uppercase proper noun ("Marko" -> "Petar")
            make_baseline(3, 4100, 6000, "Talk to Marko today", "Razgovaraj sa Marko danas ujutru"),
            // REJECTED: digits ("15" -> "20")
            make_baseline(4, 6100, 8000, "At 15 hours today", "U 15 sati danas popodne"),
            // REJECTED: email / .com / symbols
            make_baseline(5, 8100, 10000, "Mail me", "Piši na test@example.com danas"),
            // REJECTED: glossary term ("acmecorp")
            make_baseline(6, 10100, 12000, "Work at AcmeCorp", "Radim u acmecorp firmi stalno"),
            // REJECTED: < 50% unchanged tokens (full rewrite)
            make_baseline(7, 12100, 14000, "Full rewrite", "potpuno drugačija rečenica ovde"),
            // REJECTED: single char token ("u" -> "na")
            make_baseline(8, 14100, 16000, "In the room", "Nalazi se u velikoj sobi sada"),
        ];

        let final_cues = vec![
            make_cue(1, 0, 2000, "During the meeting", "Током током састанка у граду"),
            make_cue(2, 2100, 4000, "We actually went home", "Mi smo otišli kući"),
            make_cue(3, 4100, 6000, "Talk to Marko today", "Razgovaraj sa Petar danas ujutru"),
            make_cue(4, 6100, 8000, "At 15 hours today", "U 20 sati danas popodne"),
            make_cue(5, 8100, 10000, "Mail me", "Piši na info@example.com danas"),
            make_cue(6, 10100, 12000, "Work at AcmeCorp", "Radim u drugoj firmi stalno"),
            make_cue(7, 12100, 14000, "Full rewrite", "sasvim novi prevod"),
            make_cue(8, 14100, 16000, "In the room", "Nalazi se na velikoj sobi sada"),
        ];

        let report = analyze_export_diff(&baseline, &final_cues, &opts, "2026-11-salt", true, false, false);
        assert_eq!(report.ngram_diffs.len(), 2);

        assert_eq!(report.ngram_diffs[0].lang_pair, "en-sr");
        assert_eq!(report.ngram_diffs[0].old_span, "tijekom");
        assert_eq!(report.ngram_diffs[0].new_span, "tokom");
        assert_eq!(report.ngram_diffs[0].cohort_proof.len(), 24);

        assert_eq!(report.ngram_diffs[1].old_span, "zapravo");
        assert_eq!(report.ngram_diffs[1].new_span, "");
    }

    #[test]
    fn training_sanitizer_masks_pii_and_sentence_initial_proper_nouns_v05_m2() {
        let mut opts = JobOptions {
            source_language: "en".to_string(),
            target_language: "sr".to_string(),
            translation_style: TranslationStyle::Natural,
            address_form: AddressForm::InformalTi,
            ..JobOptions::default()
        };
        opts.glossary.insert("OpenCloud".to_string(), "OpenCloud".to_string());

        let baseline = vec![
            // Cue 1 introduces "Marko" and "Petrović" mid-sentence (never edited itself)
            make_baseline(
                1,
                0,
                2000,
                "I met Marko Petrović yesterday.",
                "Sreo sam Marka Petrovića juče.",
            ),
            // Cue 2 has "Marko" at index 0 (sentence start!) + glossary + email + phone + path + key + multi-digit number
            make_baseline(
                2,
                2100,
                5000,
                "Marko works at OpenCloud on 2026 project. Call +381 64 123 4567 or mail marko@company.rs using /home/marko/doc.txt and sk-secret123456789.",
                "Marko radi u OpenCloud na projektu 2026. Pozovi +381 64 123 4567 ili piši marko@company.rs.",
            ),
        ];

        let final_cues = vec![
            make_cue(
                1,
                0,
                2000,
                "I met Marko Petrović yesterday.",
                "Sreo sam Marka Petrovića juče.",
            ),
            make_cue(
                2,
                2100,
                5000,
                "Marko works at OpenCloud on 2026 project. Call +381 64 123 4567 or mail marko@company.rs using /home/marko/doc.txt and sk-secret123456789.",
                "Marko danas radi u OpenCloud na projektu 2026. Pozovite +381 64 123 4567 ili pišite marko@company.rs.",
            ),
        ];

        let count = count_modified_training_cues(&baseline, &final_cues);
        assert_eq!(count, 1);

        let report = analyze_export_diff(&baseline, &final_cues, &opts, "salt", false, true, false);
        assert_eq!(report.training_pairs.len(), 1);
        let pair = &report.training_pairs[0];

        assert_eq!(pair.style, "natural");
        assert_eq!(pair.address_form, "informalTi");
        // Sentence-initial "Marko" must be masked as [NAME] in all three fields!
        assert!(pair.source_text.starts_with("[NAME] works at [NAME] on [NUM] project."));
        assert!(pair.source_text.contains("[PHONE]"));
        assert!(pair.source_text.contains("[EMAIL]"));
        assert!(pair.source_text.contains("[PATH]"));
        assert!(pair.source_text.contains("[REDACTED]"));
        assert!(!pair.source_text.contains("Marko"));
        assert!(!pair.source_text.contains("OpenCloud"));
        assert!(!pair.source_text.contains("2026"));
        assert!(!pair.source_text.contains("sk-secret"));

        assert!(pair.initial_ai_text.starts_with("[NAME] radi u [NAME]"));
        assert!(pair.final_human_text.starts_with("[NAME] danas radi u [NAME]"));
        assert!(!pair.final_human_text.contains("Marko"));
        assert!(!pair.final_human_text.contains("marko@company.rs"));
    }

    #[test]
    fn training_pairs_capped_at_25_and_english_i_is_preserved() {
        let mut baseline = Vec::new();
        let mut final_cues = Vec::new();
        for i in 0..30u32 {
            let start = (i as u64) * 2000;
            let end = start + 1800;
            baseline.push(make_baseline(
                i + 1,
                start,
                end,
                "I think I saw a dog.",
                "Mislim da sam video psa.",
            ));
            final_cues.push(make_cue(
                i + 1,
                start,
                end,
                "I think I saw a dog.",
                "Mislim da sam video kuče.",
            ));
        }

        assert_eq!(count_modified_training_cues(&baseline, &final_cues), 25);
        let report = analyze_export_diff(
            &baseline,
            &final_cues,
            &JobOptions::default(),
            "salt",
            false,
            true,
            false,
        );
        assert_eq!(report.training_pairs.len(), 25);
        assert_eq!(report.training_pairs[0].source_text, "I think I saw a dog.");
    }
}
