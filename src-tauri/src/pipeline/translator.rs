//! Unified OpenAI-compatible subtitle translation and Serbian ASR polishing client.
//!
//! Implements the ID-locked overlapping context-window batch protocol across:
//! - Local `llama-server` (`http://127.0.0.1:<port>/v1`)
//! - DeepSeek API (`https://api.deepseek.com/v1`)
//! - Custom OpenAI-compatible endpoints
//!
//! Automatically strips `<think>...</think>` blocks and markdown code fences, validates
//! 1:1 cue ID preservation, subdivides and retries batches if the LLM drops or merges IDs,
//! normalizes Serbian script (Latin/Cyrillic), wraps balanced lines, updates CPS/warnings,
//! and checkpoints after every batch.

use crate::models::{SegmentRules, SubtitleCue, SubtitleProject};
use crate::pipeline::{segmenter, sr_script};
use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct LlmEndpointConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub batch_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOperation {
    Translate,
    PolishSerbian,
    CondenseCps,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Serialize)]
struct BatchPromptPayload {
    context_before: Vec<PromptCueItem>,
    translate_cues: Vec<PromptCueItem>,
    context_after: Vec<PromptCueItem>,
}

#[derive(Debug, Serialize)]
struct PromptCueItem {
    id: u32,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_chars: Option<u32>,
}

fn normalize_cue_single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCueText {
    pub source: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct BatchOutputItem {
    id: u32,
    #[serde(default)]
    source: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KeyedCueValue {
    Text(String),
    Object {
        #[serde(default)]
        source: Option<String>,
        text: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BatchResponseEnvelope {
    Array(Vec<BatchOutputItem>),
    Wrapped {
        #[serde(alias = "cues", alias = "translations", alias = "results")]
        items: Vec<BatchOutputItem>,
    },
    KeyedMap(BTreeMap<String, KeyedCueValue>),
}

fn think_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<think>.*?</think>").expect("valid think regex"))
}

/// Strips `<think>...</think>` blocks and markdown code fences from an LLM response,
/// isolating the JSON object `{...}` or array `[...]`.
pub fn clean_llm_json_response(raw: &str) -> String {
    let without_think = think_tag_regex().replace_all(raw, "");
    let trimmed = without_think.trim();

    // Strip markdown code fences if present
    let unfenced = if let Some(rest) = trimmed.strip_prefix("```") {
        let after_lang = if let Some(nl_pos) = rest.find('\n') {
            &rest[nl_pos + 1..]
        } else {
            rest
        };
        after_lang
            .trim()
            .strip_suffix("```")
            .unwrap_or(after_lang)
            .trim()
    } else {
        trimmed
    };

    // Isolate the outermost JSON object `{...}` or array `[...]` based on whichever starts first
    let obj_bounds = unfenced
        .find('{')
        .zip(unfenced.rfind('}'))
        .filter(|(s, e)| s < e);
    let arr_bounds = unfenced
        .find('[')
        .zip(unfenced.rfind(']'))
        .filter(|(s, e)| s < e);

    match (obj_bounds, arr_bounds) {
        (Some((os, oe)), Some((as_, ae))) => {
            if os < as_ {
                return unfenced[os..=oe].to_string();
            } else {
                return unfenced[as_..=ae].to_string();
            }
        }
        (Some((os, oe)), None) => return unfenced[os..=oe].to_string(),
        (None, Some((as_, ae))) => return unfenced[as_..=ae].to_string(),
        (None, None) => {}
    }

    unfenced.to_string()
}

/// Parses the LLM JSON output and verifies that every expected cue `id` is present and non-empty.
/// Supports both plain string values (`"1": "..."`) and dual-column objects (`"1": {"source": "...", "text": "..."}`).
pub fn parse_and_validate_batch_response(
    raw: &str,
    expected_ids: &[u32],
) -> Result<BTreeMap<u32, ParsedCueText>> {
    let cleaned = clean_llm_json_response(raw);
    let envelope: BatchResponseEnvelope = serde_json::from_str(&cleaned).with_context(|| {
        format!(
            "Failed to parse LLM JSON response (cleaned: {})",
            cleaned.chars().take(240).collect::<String>()
        )
    })?;

    let mut map = BTreeMap::new();
    match envelope {
        BatchResponseEnvelope::Array(items) | BatchResponseEnvelope::Wrapped { items } => {
            for item in items {
                let text = normalize_cue_single_line(&item.text);
                if !text.is_empty() {
                    let source = item
                        .source
                        .map(|s| normalize_cue_single_line(&s))
                        .filter(|s| !s.is_empty());
                    map.insert(item.id, ParsedCueText { source, text });
                }
            }
        }
        BatchResponseEnvelope::KeyedMap(kv) => {
            for (k, v) in kv {
                if let Ok(id) = k.trim().parse::<u32>() {
                    match v {
                        KeyedCueValue::Text(t) => {
                            let text = normalize_cue_single_line(&t);
                            if !text.is_empty() {
                                map.insert(id, ParsedCueText { source: None, text });
                            }
                        }
                        KeyedCueValue::Object { source, text } => {
                            let text = normalize_cue_single_line(&text);
                            if !text.is_empty() {
                                let source = source
                                    .map(|s| normalize_cue_single_line(&s))
                                    .filter(|s| !s.is_empty());
                                map.insert(id, ParsedCueText { source, text });
                            }
                        }
                    }
                }
            }
        }
    }

    for &expected_id in expected_ids {
        if !map.contains_key(&expected_id) {
            return Err(anyhow!(
                "LLM batch output is missing or empty for cue id {}",
                expected_id
            ));
        }
    }

    Ok(map)
}

fn build_response_format(
    endpoint: &LlmEndpointConfig,
    expected_ids: &[u32],
    correct_source_asr: bool,
    operation: BatchOperation,
) -> Option<serde_json::Value> {
    let is_local =
        endpoint.base_url.contains("127.0.0.1") || endpoint.base_url.contains("localhost");
    if is_local {
        let dual_column = correct_source_asr && operation == BatchOperation::Translate;
        let mut props = serde_json::Map::new();
        let mut req_keys = Vec::with_capacity(expected_ids.len());
        for &id in expected_ids {
            let key = id.to_string();
            let prop_schema = if dual_column {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "source": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["source", "text"],
                    "additionalProperties": false
                })
            } else {
                serde_json::json!({ "type": "string" })
            };
            props.insert(key.clone(), prop_schema);
            req_keys.push(key);
        }
        Some(serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "subtitle_map",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": props,
                    "required": req_keys,
                    "additionalProperties": false
                }
            }
        }))
    } else {
        Some(serde_json::json!({ "type": "json_object" }))
    }
}

fn build_system_prompt(
    operation: BatchOperation,
    source_language: &str,
    target_language: &str,
    glossary: &BTreeMap<String, String>,
    correct_source_asr: bool,
) -> String {
    let mut prompt = match operation {
        BatchOperation::Translate => {
            let target_instructions = if target_language.eq_ignore_ascii_case("sr") {
                "Translate into natural, idiomatic Serbian (strict Ekavian standard: 'mleko', 'devojka', 'vreme', 'gde', 'lepo' — never Ijekavian/Croatian forms like 'mlijeko', 'vrijeme', 'tko', 'krajolik'). Use accurate Serbian grammar and cases, never invent non-existent words, and keep subtitle phrasing concise for comfortable reading speed."
            } else {
                "Translate accurately, idiomatically, and concisely into the target language for subtitles."
            };
            if correct_source_asr {
                format!(
                    "You are an expert subtitle translator from '{}' to '{}' and ASR transcript proofreader. {}\n\
                     STRICT RULES:\n\
                     1. Reconstruct full sentences mentally across adjacent cues (`context_before`, `translate_cues`, `context_after`) so that phrases split across cue boundaries translate into coherent, natural grammar.\n\
                     2. ALSO fix obvious speech-to-text phonetic/homophone errors in the source text (e.g., \"Aesip\" -> \"Aesop\") based on context without altering the speaker's words, and translate using the corrected source meaning.\n\
                     3. Translate ONLY the items in `translate_cues`, preserving each cue's corresponding portion of the sentence and staying within each item's `max_chars` reading-speed budget whenever natural.\n\
                     4. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                     5. Output ONLY a valid JSON object mapping each cue ID string to an object `{{\"1\": {{\"source\": \"<corrected source text>\", \"text\": \"<translated text>\"}}}}` with no markdown fences and no commentary.",
                    source_language, target_language, target_instructions
                )
            } else {
                format!(
                    "You are an expert subtitle translator from '{}' to '{}'. {}\n\
                     STRICT RULES:\n\
                     1. Reconstruct full sentences mentally across adjacent cues (`context_before`, `translate_cues`, `context_after`) so that phrases split across cue boundaries translate into coherent, natural grammar.\n\
                     2. Translate ONLY the items in `translate_cues`, preserving each cue's corresponding portion of the sentence and staying within each item's `max_chars` reading-speed budget whenever natural.\n\
                     3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                     4. Output ONLY a valid JSON object mapping each cue ID string to its translated text: `{{\"1\": \"...\", \"2\": \"...\"}}` with no markdown fences and no commentary.",
                    source_language, target_language, target_instructions
                )
            }
        }
        BatchOperation::PolishSerbian => {
            "You are an expert Serbian subtitle editor and proofreader.\n\
             STRICT RULES:\n\
             1. Reconstruct full sentences mentally across `context_before`, `translate_cues`, and `context_after`, and polish ONLY the items in `translate_cues`.\n\
             2. Fix speech-to-text recognition errors, punctuation, capitalization, and normalize any Ijekavian/Croatian forms into natural Serbian Ekavian standard without changing the speaker's meaning or inventing words, staying within each item's `max_chars` budget when feasible.\n\
             3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
             4. Output ONLY a valid JSON object mapping each cue ID string to its polished text: `{\"1\": \"...\", \"2\": \"...\"}` with no markdown fences and no commentary."
                .to_string()
        }
        BatchOperation::CondenseCps => {
            "You are an expert subtitle editor specializing in tightening verbose subtitles to comfortable reading speed (CPS).\n\
             STRICT RULES:\n\
             1. Rewrite ONLY the cues in `translate_cues` using `context_before` and `context_after` for continuity so every rewritten cue fits cleanly within its `max_chars` budget (shorten by ~15-30% when needed).\n\
             2. Preserve the core meaning, tone, speaker intent, and target language (when Serbian, strictly use natural Ekavian standard — never Ijekavian/Croatian forms) by cutting filler words and tightening phrasing without inventing facts.\n\
             3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
             4. Output ONLY a valid JSON object mapping each cue ID string to its condensed text: `{\"1\": \"...\", \"2\": \"...\"}` with no markdown fences and no commentary."
                .to_string()
        }
    };

    if !glossary.is_empty() {
        prompt.push_str("\n\nMANDATORY GLOSSARY TERMS:\n");
        for (src, dst) in glossary {
            if !src.trim().is_empty() && !dst.trim().is_empty() {
                prompt.push_str(&format!("- {} => {}\n", src.trim(), dst.trim()));
            }
        }
    }

    prompt
}

fn cue_visible_text(cue: &SubtitleCue) -> &str {
    if cue.translated_text.trim().is_empty() {
        &cue.source_text
    } else {
        &cue.translated_text
    }
}

fn cue_max_chars(cue: &SubtitleCue, rules: &SegmentRules) -> u32 {
    let duration_secs = (cue.end_ms.saturating_sub(cue.start_ms) as f64) / 1000.0;
    let cps_budget = (duration_secs * rules.max_cps).floor() as u32;
    let layout_budget = rules.max_chars_per_line.saturating_mul(rules.max_lines).max(12);
    cps_budget.clamp(12, layout_budget)
}

fn cue_needs_condensing(cue: &SubtitleCue, rules: &SegmentRules) -> bool {
    let mut probe = cue.clone();
    segmenter::compute_cue_metrics(&mut probe, rules);
    probe.cps > rules.max_cps
        || probe
            .warnings
            .iter()
            .any(|w| w == "high_cps" || w == "line_too_long")
}

fn raw_for_op(operation: BatchOperation, c: &SubtitleCue) -> &str {
    match operation {
        BatchOperation::CondenseCps => cue_visible_text(c),
        BatchOperation::Translate | BatchOperation::PolishSerbian => &c.source_text,
    }
}

fn build_user_prompt(
    operation: BatchOperation,
    rules: &SegmentRules,
    all_cues: &[SubtitleCue],
    context_before: &[SubtitleCue],
    target_indices: &[usize],
    context_after: &[SubtitleCue],
) -> Result<String> {
    let to_context_item = |c: &SubtitleCue| -> PromptCueItem {
        PromptCueItem {
            id: c.id,
            text: normalize_cue_single_line(raw_for_op(operation, c)),
            max_chars: None,
        }
    };

    let to_target_item = |c: &SubtitleCue| -> PromptCueItem {
        PromptCueItem {
            id: c.id,
            text: normalize_cue_single_line(raw_for_op(operation, c)),
            max_chars: Some(cue_max_chars(c, rules)),
        }
    };

    let payload = BatchPromptPayload {
        context_before: context_before.iter().map(to_context_item).collect(),
        translate_cues: target_indices
            .iter()
            .map(|&idx| to_target_item(&all_cues[idx]))
            .collect(),
        context_after: context_after.iter().map(to_context_item).collect(),
    };

    serde_json::to_string(&payload).context("Failed to serialize batch prompt payload")
}

async fn call_chat_completions(
    client: &reqwest::Client,
    endpoint: &LlmEndpointConfig,
    system_prompt: &str,
    user_prompt: &str,
    response_format: Option<serde_json::Value>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<String> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    let base = endpoint.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(anyhow!("Translation endpoint base URL is empty"));
    }
    let url = format!("{base}/chat/completions");

    let req_body = ChatCompletionRequest {
        model: if endpoint.model.trim().is_empty() {
            "local-model".to_string()
        } else {
            endpoint.model.trim().to_string()
        },
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt.to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt.to_string(),
            },
        ],
        temperature: 0.2,
        response_format,
    };

    let mut req = client.post(&url).json(&req_body);
    if let Some(key) = endpoint.api_key.as_deref() {
        let trimmed_key = key.trim();
        if !trimmed_key.is_empty() {
            req = req.bearer_auth(trimmed_key);
        }
    }

    let resp = tokio::select! {
        res = req.send() => res.with_context(|| format!("HTTP request to {url} failed"))?,
        _ = async {
            while !cancel_flag.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => {
            return Err(anyhow!("Job cancelled"));
        }
    };

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .context("Failed reading chat completions response body")?;

    if !status.is_success() {
        return Err(anyhow!(
            "Chat completions endpoint returned HTTP {}: {}",
            status,
            body_text.chars().take(300).collect::<String>()
        ));
    }

    let parsed: ChatCompletionResponse =
        serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Failed to deserialize OpenAI chat completions response: {}",
                body_text.chars().take(240).collect::<String>()
            )
        })?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| anyhow!("Chat completions response contained no choices"))?;

    Ok(content)
}

#[allow(clippy::too_many_arguments)]
async fn translate_slice_with_subdivision(
    client: &reqwest::Client,
    endpoint: &LlmEndpointConfig,
    operation: BatchOperation,
    correct_source_asr: bool,
    rules: &SegmentRules,
    system_prompt: &str,
    all_cues: &[SubtitleCue],
    target_indices: &[usize],
    cancel_flag: &Arc<AtomicBool>,
) -> Result<BTreeMap<u32, ParsedCueText>> {
    let mut out = BTreeMap::new();
    if target_indices.is_empty() {
        return Ok(out);
    }

    let mut queue = vec![(0usize, target_indices.len())];

    while let Some((q_start, q_end)) = queue.pop() {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        let slice_indices = &target_indices[q_start..q_end];
        let first_idx = slice_indices[0];
        let last_idx = *slice_indices.last().unwrap_or(&first_idx);
        let ctx_before = &all_cues[first_idx.saturating_sub(3)..first_idx];
        let ctx_after = &all_cues[(last_idx + 1)..(last_idx + 4).min(all_cues.len())];
        let expected_ids: Vec<u32> = slice_indices.iter().map(|&idx| all_cues[idx].id).collect();
        let user_prompt = build_user_prompt(
            operation,
            rules,
            all_cues,
            ctx_before,
            slice_indices,
            ctx_after,
        )?;
        let response_format =
            build_response_format(endpoint, &expected_ids, correct_source_asr, operation);

        let max_attempts = if slice_indices.len() == 1 { 2 } else { 1 };
        let mut last_err = None;
        let mut succeeded = None;

        for _attempt in 0..max_attempts {
            match call_chat_completions(
                client,
                endpoint,
                system_prompt,
                &user_prompt,
                response_format.clone(),
                cancel_flag,
            )
            .await
            .and_then(|raw| parse_and_validate_batch_response(&raw, &expected_ids))
            {
                Ok(batch_map) => {
                    succeeded = Some(batch_map);
                    break;
                }
                Err(err) => {
                    if cancel_flag.load(Ordering::SeqCst) {
                        return Err(anyhow!("Job cancelled"));
                    }
                    last_err = Some(err);
                }
            }
        }

        if let Some(batch_map) = succeeded {
            out.extend(batch_map);
        } else if slice_indices.len() > 1 {
            // Subdivide failed batch into two smaller halves to guarantee 1:1 cue ID alignment
            let mid = q_start + (slice_indices.len() / 2);
            queue.push((mid, q_end));
            queue.push((q_start, mid));
        } else {
            return Err(last_err.unwrap_or_else(|| {
                anyhow!("Failed to process cue id {}", all_cues[first_idx].id)
            }));
        }
    }

    Ok(out)
}

/// Runs the ID-locked overlapping context-window batch protocol across `project.cues`.
pub async fn run_batch_operation<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    operation: BatchOperation,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    mut on_checkpoint: FCheckpoint,
    mut on_progress: FProgress,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    if project.cues.is_empty() {
        return Ok(());
    }

    let batch_size = endpoint.batch_size.clamp(1, 50) as usize;
    let total_batches = project.cues.len().div_ceil(batch_size);

    let system_prompt = build_system_prompt(
        operation,
        &project.options.source_language,
        &project.options.target_language,
        &project.options.glossary,
        project.options.correct_source_asr,
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("Failed to initialize translation HTTP client")?;

    let is_serbian_target = operation == BatchOperation::PolishSerbian
        || project.options.target_language.eq_ignore_ascii_case("sr")
        || (operation == BatchOperation::CondenseCps
            && project.options.target_language == "none"
            && (project
                .detected_language
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case("sr"))
                || project.options.source_language.eq_ignore_ascii_case("sr")));
    let rules = segmenter::normalize_rules(&project.options.segment_rules);

    for batch_idx in 0..total_batches {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        let start = batch_idx * batch_size;
        let end = ((batch_idx + 1) * batch_size).min(project.cues.len());

        // Resume support: skip already completed batches from a saved checkpoint
        // (CondenseCps operates on cues that may already have completed_translation_batches set).
        if operation != BatchOperation::CondenseCps {
            let already_translated = (batch_idx as u32) < project.completed_translation_batches
                && project.cues[start..end]
                    .iter()
                    .all(|c| !c.translated_text.trim().is_empty());

            if already_translated {
                let pct = ((batch_idx + 1) as f64 / total_batches as f64) * 100.0;
                on_progress(
                    pct,
                    format!(
                        "Skipped completed batch {}/{} from checkpoint",
                        batch_idx + 1,
                        total_batches
                    ),
                );
                continue;
            }
        }

        let target_indices: Vec<usize> = match operation {
            BatchOperation::CondenseCps => (start..end)
                .filter(|&idx| cue_needs_condensing(&project.cues[idx], &rules))
                .collect(),
            BatchOperation::Translate | BatchOperation::PolishSerbian => (start..end).collect(),
        };

        if target_indices.is_empty() {
            let pct = ((batch_idx + 1) as f64 / total_batches as f64) * 100.0;
            on_progress(
                pct,
                format!(
                    "Skipped batch {}/{} (all cues within CPS limits)",
                    batch_idx + 1,
                    total_batches
                ),
            );
            continue;
        }

        let batch_results = translate_slice_with_subdivision(
            &client,
            endpoint,
            operation,
            project.options.correct_source_asr,
            &rules,
            &system_prompt,
            &project.cues,
            &target_indices,
            cancel_flag,
        )
        .await?;

        for &idx in &target_indices {
            let cue = &mut project.cues[idx];
            if let Some(parsed) = batch_results.get(&cue.id) {
                if operation == BatchOperation::Translate && project.options.correct_source_asr {
                    if let Some(src) = &parsed.source {
                        if !src.trim().is_empty() {
                            cue.source_text = segmenter::wrap_balanced_lines(
                                src,
                                rules.max_chars_per_line,
                                rules.max_lines,
                            );
                        }
                    }
                }
                let scripted = if is_serbian_target {
                    let ekavian = sr_script::normalize_ekavian(&parsed.text);
                    sr_script::convert_script(&ekavian, project.options.script)
                } else {
                    parsed.text.clone()
                };
                let wrapped = segmenter::wrap_balanced_lines(
                    &scripted,
                    rules.max_chars_per_line,
                    rules.max_lines,
                );
                if operation == BatchOperation::CondenseCps
                    && cue.translated_text.trim().is_empty()
                {
                    cue.source_text = wrapped;
                } else {
                    cue.translated_text = wrapped;
                }
                segmenter::compute_cue_metrics(cue, &rules);
            }
        }

        if operation != BatchOperation::CondenseCps {
            project.completed_translation_batches = (batch_idx + 1) as u32;
        }
        project.updated_at = chrono::Utc::now().to_rfc3339();
        on_checkpoint(project)?;

        let pct = ((batch_idx + 1) as f64 / total_batches as f64) * 100.0;
        let action_label = match operation {
            BatchOperation::Translate => "Translated",
            BatchOperation::PolishSerbian => "Polished",
            BatchOperation::CondenseCps => "Condensed",
        };
        on_progress(
            pct,
            format!(
                "{action_label} batch {}/{} ({} cues)",
                batch_idx + 1,
                total_batches,
                end
            ),
        );
    }

    Ok(())
}

/// Translates `project.cues` using the configured LLM endpoint.
pub async fn translate_cues<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    on_checkpoint: FCheckpoint,
    on_progress: FProgress,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    run_batch_operation(
        project,
        BatchOperation::Translate,
        endpoint,
        cancel_flag,
        on_checkpoint,
        on_progress,
    )
    .await
}

/// Polishes Serbian ASR cues in `project.cues` using the configured LLM endpoint.
pub async fn polish_serbian_cues<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    on_checkpoint: FCheckpoint,
    on_progress: FProgress,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    run_batch_operation(
        project,
        BatchOperation::PolishSerbian,
        endpoint,
        cancel_flag,
        on_checkpoint,
        on_progress,
    )
    .await
}

/// Condenses cues that exceed CPS or line-length limits using per-cue `max_chars` budgets.
pub async fn condense_cps_cues<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    on_checkpoint: FCheckpoint,
    on_progress: FProgress,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    run_batch_operation(
        project,
        BatchOperation::CondenseCps,
        endpoint,
        cancel_flag,
        on_checkpoint,
        on_progress,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{JobOptions, JobPhase, SerbianScript};
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_project(script: SerbianScript) -> SubtitleProject {
        SubtitleProject {
            job_id: "test-job".to_string(),
            audio_path: "/tmp/test.mp3".to_string(),
            audio_file_name: "test.mp3".to_string(),
            duration_ms: 8000,
            detected_language: Some("en".to_string()),
            options: JobOptions {
                source_language: "en".to_string(),
                target_language: "sr".to_string(),
                script,
                ..JobOptions::default()
            },
            words: vec![],
            cues: vec![
                SubtitleCue {
                    id: 1,
                    start_ms: 0,
                    end_ms: 2000,
                    source_text: "Good morning everyone.".to_string(),
                    translated_text: String::new(),
                    cps: 11.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 2,
                    start_ms: 2000,
                    end_ms: 4000,
                    source_text: "Welcome to Belgrade.".to_string(),
                    translated_text: String::new(),
                    cps: 10.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 3,
                    start_ms: 4000,
                    end_ms: 6000,
                    source_text: "Love and attention plays an important role in our lives today.".to_string(),
                    translated_text: String::new(),
                    cps: 15.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 4,
                    start_ms: 6000,
                    end_ms: 8000,
                    source_text: "Thank you for watching!".to_string(),
                    translated_text: String::new(),
                    cps: 11.5,
                    warnings: vec![],
                },
            ],
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            error_message: None,
            updated_at: "2026-11-22T18:00:00Z".to_string(),
        }
    }

    #[test]
    fn cleans_think_tags_and_markdown_fences() {
        let raw = "<think>\nLet me translate cues 1 and 2 carefully.\n</think>\n```json\n[\n  {\"id\": 1, \"text\": \"Dobro jutro\"},\n  {\"id\": 2, \"text\": \"Dobrodošli\"}\n]\n```";
        let map = parse_and_validate_batch_response(raw, &[1, 2]).unwrap();
        assert_eq!(map.get(&1).map(|v| v.text.as_str()), Some("Dobro jutro"));
        assert_eq!(map.get(&2).map(|v| v.text.as_str()), Some("Dobrodošli"));
    }

    #[tokio::test]
    async fn recovers_via_subbatch_retry_and_applies_cyrillic_plus_line_wrapping() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let call_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&call_counter);

        let server_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let count = counter_clone.fetch_add(1, Ordering::SeqCst);

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let mut total_read = 0usize;
                    let mut header_end = None;
                    let mut content_len = 0usize;

                    loop {
                        let n = socket.read(&mut buf[total_read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        total_read += n;
                        let s = String::from_utf8_lossy(&buf[..total_read]);
                        if header_end.is_none() {
                            if let Some(pos) = s.find("\r\n\r\n") {
                                header_end = Some(pos + 4);
                                for line in s[..pos].lines() {
                                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                                        content_len = v.trim().parse().unwrap_or(0);
                                    }
                                }
                            }
                        }
                        if let Some(h_end) = header_end {
                            if total_read >= h_end + content_len {
                                break;
                            }
                        }
                    }

                    let req_str = String::from_utf8_lossy(&buf[..total_read]);
                    let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
                    let req_json: serde_json::Value =
                        serde_json::from_str(body_part).unwrap_or_default();
                    let user_content = req_json["messages"][1]["content"]
                        .as_str()
                        .unwrap_or("{}");
                    let prompt_payload: serde_json::Value =
                        serde_json::from_str(user_content).unwrap_or_default();
                    let translate_cues = prompt_payload["translate_cues"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();

                    // On the VERY FIRST call (count == 0, which has 2 cues [1, 2]),
                    // intentionally drop cue #2 to trigger automatic sub-batch subdivision!
                    let llm_content = if count == 0 && translate_cues.len() > 1 {
                        "```json\n[{\"id\": 1, \"text\": \"Dobro jutro svima.\"}]\n```".to_string()
                    } else {
                        let items: Vec<serde_json::Value> = translate_cues
                            .iter()
                            .map(|c| {
                                let id = c["id"].as_u64().unwrap_or(1);
                                let text = match id {
                                    1 => "Dobro jutro svima.",
                                    2 => "Dobrodošli u Beograd.",
                                    3 => "Ljubav i pažnja igraju veoma važnu ulogu u našim životima danas.",
                                    4 => "Hvala vam što ste gledali!",
                                    _ => "Prevod.",
                                };
                                serde_json::json!({ "id": id, "text": text })
                            })
                            .collect();
                        format!(
                            "<think>Translating batch</think>\n```json\n{}\n```",
                            serde_json::to_string(&items).unwrap()
                        )
                    };

                    let resp_json = serde_json::json!({
                        "choices": [
                            {
                                "message": {
                                    "role": "assistant",
                                    "content": llm_content
                                }
                            }
                        ]
                    });
                    let resp_body = serde_json::to_string(&resp_json).unwrap();
                    let http_resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = socket.write_all(http_resp.as_bytes()).await;
                });
            }
        });

        let mut project = sample_project(SerbianScript::Cyrillic);
        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "test-model".to_string(),
            api_key: Some("test-key".to_string()),
            batch_size: 2, // 4 cues -> 2 batches of 2 cues
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut checkpoints_saved = 0usize;
        let mut progress_events = 0usize;

        translate_cues(
            &mut project,
            &endpoint,
            &cancel,
            |_proj| {
                checkpoints_saved += 1;
                Ok(())
            },
            |_pct, _detail| {
                progress_events += 1;
            },
        )
        .await
        .unwrap();

        server_task.abort();

        // Batch 1 failed with 2 cues, subdivided into 2 single-cue calls (1 + 2 = 3 calls),
        // Batch 2 succeeded on first try (1 call) => 4 total HTTP calls
        assert_eq!(call_counter.load(Ordering::SeqCst), 4);
        assert_eq!(checkpoints_saved, 2);
        assert_eq!(progress_events, 2);
        assert_eq!(project.completed_translation_batches, 2);

        // Verify Serbian Cyrillic script conversion and balanced 2-line wrapping
        assert_eq!(project.cues[0].translated_text, "Добро јутрo свима.".replace('o', "о"));
        assert_eq!(project.cues[1].translated_text, "Добродошли у Београд.");
        assert!(
            project.cues[2].translated_text.contains("Љубав и пажња"),
            "got: {}",
            project.cues[2].translated_text
        );
        // Cue 3 is > 42 chars, so it must be wrapped into 2 balanced lines
        assert_eq!(project.cues[2].translated_text.lines().count(), 2);
        assert_eq!(project.cues[3].translated_text, "Хвала вам што сте гледали!");
    }

    #[tokio::test]
    async fn resumes_from_completed_batches_without_retranslating() {
        let mut project = sample_project(SerbianScript::Latin);
        project.cues[0].translated_text = "Dobro jutro svima.".to_string();
        project.cues[1].translated_text = "Dobrodošli u Beograd.".to_string();
        project.cues[2].translated_text = "Ljubav i pažnja.".to_string();
        project.cues[3].translated_text = "Hvala vam!".to_string();
        project.completed_translation_batches = 2;

        // Invalid/unreachable port: if it tries to make an HTTP call, it would fail!
        let endpoint = LlmEndpointConfig {
            base_url: "http://127.0.0.1:1/v1".to_string(),
            model: "test-model".to_string(),
            api_key: None,
            batch_size: 2,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut progress_calls = 0usize;

        translate_cues(
            &mut project,
            &endpoint,
            &cancel,
            |_proj| Ok(()),
            |_pct, _detail| {
                progress_calls += 1;
            },
        )
        .await
        .unwrap();

        assert_eq!(progress_calls, 2);
    }

    #[test]
    fn parses_keyed_json_map_with_brackets_and_builds_response_formats() {
        let raw = "<think>ok</think>```json\n{\"1\": \"Zdravo [SMEH] svima.\", \"2\": \"Kako ste?\"}\n```";
        let map = parse_and_validate_batch_response(raw, &[1, 2]).unwrap();
        assert_eq!(
            map.get(&1).map(|v| v.text.as_str()),
            Some("Zdravo [SMEH] svima.")
        );
        assert_eq!(map.get(&2).map(|v| v.text.as_str()), Some("Kako ste?"));

        let dual_raw = "{\"1\": {\"source\": \"Section 37 of Aesop.\", \"text\": \"Odeljak 37 Ezopa.\"}, \"2\": \"Kako ste?\"}";
        let dual_map = parse_and_validate_batch_response(dual_raw, &[1, 2]).unwrap();
        assert_eq!(
            dual_map.get(&1),
            Some(&ParsedCueText {
                source: Some("Section 37 of Aesop.".to_string()),
                text: "Odeljak 37 Ezopa.".to_string(),
            })
        );
        assert_eq!(
            dual_map.get(&2),
            Some(&ParsedCueText {
                source: None,
                text: "Kako ste?".to_string(),
            })
        );

        let local_ep = LlmEndpointConfig {
            base_url: "http://127.0.0.1:8080/v1".to_string(),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 25,
        };
        let local_rf =
            build_response_format(&local_ep, &[1, 2], false, BatchOperation::Translate).unwrap();
        assert_eq!(local_rf["type"], "json_schema");
        assert_eq!(local_rf["json_schema"]["strict"], true);
        assert_eq!(
            local_rf["json_schema"]["schema"]["required"],
            serde_json::json!(["1", "2"])
        );
        assert_eq!(
            local_rf["json_schema"]["schema"]["properties"]["1"],
            serde_json::json!({ "type": "string" })
        );

        let dual_rf =
            build_response_format(&local_ep, &[1, 2], true, BatchOperation::Translate).unwrap();
        assert_eq!(
            dual_rf["json_schema"]["schema"]["properties"]["1"],
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["source", "text"],
                "additionalProperties": false
            })
        );

        let cloud_ep = LlmEndpointConfig {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-chat".to_string(),
            api_key: Some("k".to_string()),
            batch_size: 25,
        };
        let cloud_rf =
            build_response_format(&cloud_ep, &[1, 2], true, BatchOperation::Translate).unwrap();
        assert_eq!(cloud_rf["type"], "json_object");
    }

    #[tokio::test]
    async fn condenses_high_cps_cues_within_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let call_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&call_counter);
        let captured_max_chars = Arc::new(tokio::sync::Mutex::new(Vec::<(u64, u64)>::new()));
        let captured_clone = Arc::clone(&captured_max_chars);

        let server_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let counter_clone = Arc::clone(&counter_clone);
                let captured_clone = Arc::clone(&captured_clone);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 32768];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let raw_req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let body_part = raw_req.split("\r\n\r\n").nth(1).unwrap_or("{}");
                    let req_json: serde_json::Value =
                        serde_json::from_str(body_part).unwrap_or_default();
                    let user_content = req_json["messages"][1]["content"]
                        .as_str()
                        .unwrap_or("{}");
                    let prompt_payload: serde_json::Value =
                        serde_json::from_str(user_content).unwrap_or_default();
                    let targets = prompt_payload["translate_cues"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();

                    counter_clone.fetch_add(1, Ordering::SeqCst);

                    let mut map_out = serde_json::Map::new();
                    {
                        let mut guard = captured_clone.lock().await;
                        for t in &targets {
                            let id = t["id"].as_u64().unwrap_or(1);
                            let max_c = t["max_chars"].as_u64().unwrap_or(0);
                            guard.push((id, max_c));
                            map_out.insert(
                                id.to_string(),
                                serde_json::Value::String("Kraći tekst.".to_string()),
                            );
                        }
                    }

                    let resp_json = serde_json::json!({
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": serde_json::Value::Object(map_out).to_string()
                            }
                        }]
                    });
                    let resp_body = serde_json::to_string(&resp_json).unwrap();
                    let http_resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = socket.write_all(http_resp.as_bytes()).await;
                });
            }
        });

        let mut project = sample_project(SerbianScript::Latin);
        project.completed_translation_batches = 2;
        // Cue 1 (0..2000ms -> budget 34 chars at 17 CPS): make it long/high-CPS
        project.cues[0].translated_text = "Ovo je izuzetno predugačka rečenica za titl od dve sekunde.".to_string();
        segmenter::compute_cue_metrics(&mut project.cues[0], &project.options.segment_rules);
        // Cue 2 (1200..3000ms -> 1.8s -> 32 chars): keep it short and compliant
        project.cues[1].translated_text = "Kratak titl.".to_string();
        segmenter::compute_cue_metrics(&mut project.cues[1], &project.options.segment_rules);
        // Batch 2 (cues 3 & 4): keep both short & compliant -> whole batch must be skipped!
        project.cues[2].translated_text = "U redu.".to_string();
        segmenter::compute_cue_metrics(&mut project.cues[2], &project.options.segment_rules);
        project.cues[3].translated_text = "Hvala.".to_string();
        segmenter::compute_cue_metrics(&mut project.cues[3], &project.options.segment_rules);

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "test-model".to_string(),
            api_key: Some("test-key".to_string()),
            batch_size: 2,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut checkpoints = 0usize;

        condense_cps_cues(
            &mut project,
            &endpoint,
            &cancel,
            |_| {
                checkpoints += 1;
                Ok(())
            },
            |_, _| {},
        )
        .await
        .unwrap();

        server_task.abort();

        // Only Batch 1 should hit the HTTP server, and ONLY Cue 1 should appear in translate_cues
        assert_eq!(call_counter.load(Ordering::SeqCst), 1);
        assert_eq!(checkpoints, 1);
        let sent = captured_max_chars.lock().await;
        assert_eq!(*sent, vec![(1, 34)]);
        assert_eq!(project.cues[0].translated_text, "Kraći tekst.");
        assert!(project.cues[0].cps <= project.options.segment_rules.max_cps);
        assert_eq!(project.cues[1].translated_text, "Kratak titl.");
        assert_eq!(project.completed_translation_batches, 2);
    }

    #[tokio::test]
    async fn translates_and_corrects_source_asr_in_single_pass() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured_requests = Arc::new(tokio::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let captured_clone = Arc::clone(&captured_requests);

        let server_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let captured_clone = Arc::clone(&captured_clone);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 32768];
                    let mut total_read = 0usize;
                    let mut header_end = None;
                    let mut content_len = 0usize;

                    loop {
                        let n = socket.read(&mut buf[total_read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        total_read += n;
                        let s = String::from_utf8_lossy(&buf[..total_read]);
                        if header_end.is_none() {
                            if let Some(pos) = s.find("\r\n\r\n") {
                                header_end = Some(pos + 4);
                                for line in s[..pos].lines() {
                                    if let Some(v) =
                                        line.to_ascii_lowercase().strip_prefix("content-length:")
                                    {
                                        content_len = v.trim().parse().unwrap_or(0);
                                    }
                                }
                            }
                        }
                        if let Some(h_end) = header_end {
                            if total_read >= h_end + content_len {
                                break;
                            }
                        }
                    }

                    let raw_req = String::from_utf8_lossy(&buf[..total_read]).to_string();
                    let body_part = raw_req.split("\r\n\r\n").nth(1).unwrap_or("{}");
                    let req_json: serde_json::Value =
                        serde_json::from_str(body_part).unwrap_or_default();
                    {
                        let mut guard = captured_clone.lock().await;
                        guard.push(req_json.clone());
                    }

                    let llm_content = serde_json::json!({
                        "1": {
                            "source": "Section 37 of the Aesop for Children, and this is a classic fable.",
                            "text": "Odeljak 37 Ezopovih basni za decu, a ovo je klasična basna."
                        },
                        "2": {
                            "source": "Welcome to Belgrade!",
                            "text": "Dobrodošli u Beograd!"
                        }
                    })
                    .to_string();

                    let resp_json = serde_json::json!({
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": llm_content
                            }
                        }]
                    });
                    let resp_body = serde_json::to_string(&resp_json).unwrap();
                    let http_resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = socket.write_all(http_resp.as_bytes()).await;
                });
            }
        });

        let mut project = SubtitleProject {
            job_id: "dual-asr-job".to_string(),
            audio_path: "/tmp/fable.mp3".to_string(),
            audio_file_name: "fable.mp3".to_string(),
            duration_ms: 8000,
            detected_language: Some("en".to_string()),
            options: JobOptions {
                source_language: "en".to_string(),
                target_language: "sr".to_string(),
                script: SerbianScript::Latin,
                correct_source_asr: true,
                ..JobOptions::default()
            },
            words: vec![],
            cues: vec![
                SubtitleCue {
                    id: 1,
                    start_ms: 0,
                    end_ms: 4000,
                    source_text: "Section 37 of the Aesip for Children, and this is a classic fable."
                        .to_string(),
                    translated_text: String::new(),
                    cps: 16.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 2,
                    start_ms: 4000,
                    end_ms: 6500,
                    source_text: "Welcome to Bellgrade!".to_string(),
                    translated_text: String::new(),
                    cps: 8.0,
                    warnings: vec![],
                },
            ],
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            error_message: None,
            updated_at: "2026-11-22T22:00:00Z".to_string(),
        };

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 25,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut checkpoints = 0usize;

        translate_cues(
            &mut project,
            &endpoint,
            &cancel,
            |_| {
                checkpoints += 1;
                Ok(())
            },
            |_, _| {},
        )
        .await
        .unwrap();

        server_task.abort();

        assert_eq!(checkpoints, 1);
        assert_eq!(project.completed_translation_batches, 1);

        // Verify local json_schema enforced {"source": string, "text": string} per cue ID
        let reqs = captured_requests.lock().await;
        assert_eq!(reqs.len(), 1);
        let sys_prompt = reqs[0]["messages"][0]["content"].as_str().unwrap_or("");
        assert!(sys_prompt.contains("Aesop"));
        assert!(sys_prompt.contains("\"source\""));
        assert_eq!(
            reqs[0]["response_format"]["json_schema"]["schema"]["properties"]["1"],
            serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["source", "text"],
                "additionalProperties": false
            })
        );

        // Verify both source_text (with balanced line wrapping) and translated_text were updated
        assert!(project.cues[0].source_text.contains("Aesop"));
        assert!(!project.cues[0].source_text.contains("Aesip"));
        assert_eq!(project.cues[0].source_text.lines().count(), 2);
        assert!(project.cues[0].translated_text.contains("Ezopovih"));
        assert_eq!(project.cues[0].translated_text.lines().count(), 2);
        assert_eq!(project.cues[1].source_text, "Welcome to Belgrade!");
        assert_eq!(project.cues[1].translated_text, "Dobrodošli u Beograd!");
    }
}
