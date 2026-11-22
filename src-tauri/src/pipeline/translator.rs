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

use crate::models::{SubtitleCue, SubtitleProject};
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
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
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
struct BatchPromptPayload<'a> {
    context_before: Vec<PromptCueItem<'a>>,
    translate_cues: Vec<PromptCueItem<'a>>,
    context_after: Vec<PromptCueItem<'a>>,
}

#[derive(Debug, Serialize)]
struct PromptCueItem<'a> {
    id: u32,
    text: &'a str,
}

#[derive(Debug, Deserialize)]
struct BatchOutputItem {
    id: u32,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BatchResponseEnvelope {
    Array(Vec<BatchOutputItem>),
    Wrapped {
        #[serde(alias = "cues", alias = "translations", alias = "results")]
        items: Vec<BatchOutputItem>,
    },
}

fn think_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?is)<think>.*?</think>").expect("valid think regex"))
}

/// Strips `<think>...</think>` blocks and markdown code fences from an LLM response,
/// isolating the JSON payload.
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

    // Isolate the outermost JSON array `[...]` if present
    if let (Some(start), Some(end)) = (unfenced.find('['), unfenced.rfind(']')) {
        if start < end {
            return unfenced[start..=end].to_string();
        }
    }

    unfenced.to_string()
}

/// Parses the LLM JSON output and verifies that every expected cue `id` is present and non-empty.
pub fn parse_and_validate_batch_response(
    raw: &str,
    expected_ids: &[u32],
) -> Result<BTreeMap<u32, String>> {
    let cleaned = clean_llm_json_response(raw);
    let envelope: BatchResponseEnvelope = serde_json::from_str(&cleaned).with_context(|| {
        format!(
            "Failed to parse LLM JSON response (cleaned: {})",
            cleaned.chars().take(240).collect::<String>()
        )
    })?;

    let items = match envelope {
        BatchResponseEnvelope::Array(arr) => arr,
        BatchResponseEnvelope::Wrapped { items } => items,
    };

    let mut map = BTreeMap::new();
    for item in items {
        let text = item
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            map.insert(item.id, text);
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

fn build_system_prompt(
    operation: BatchOperation,
    source_language: &str,
    target_language: &str,
    glossary: &BTreeMap<String, String>,
) -> String {
    let mut prompt = match operation {
        BatchOperation::Translate => {
            let target_instructions = if target_language.eq_ignore_ascii_case("sr") {
                "Translate into natural, idiomatic Serbian (strict Ekavian standard: 'mleko', 'devojka', 'vreme', 'gde', 'lepo' — never Ijekavian/Croatian forms). Keep subtitle phrasing concise for comfortable reading speed."
            } else {
                "Translate accurately and concisely into the target language for subtitles."
            };
            format!(
                "You are an expert subtitle translator from '{}' to '{}'. {}\n\
                 STRICT RULES:\n\
                 1. Translate ONLY the items in `translate_cues`. Use `context_before` and `context_after` purely for context so sentences split across cue boundaries flow grammatically.\n\
                 2. NEVER merge, split, add, or drop cues. Return the EXACT same `id`s as in `translate_cues`.\n\
                 3. Output ONLY a valid JSON array of objects: `[{{\"id\": 1, \"text\": \"...\"}}, ...]` with no markdown fences and no commentary.",
                source_language, target_language, target_instructions
            )
        }
        BatchOperation::PolishSerbian => {
            "You are an expert Serbian subtitle editor and proofreader.\n\
             STRICT RULES:\n\
             1. Polish ONLY the items in `translate_cues` (using `context_before` and `context_after` for sentence context).\n\
             2. Fix speech-to-text recognition errors, punctuation, capitalization, and normalize any Ijekavian/Croatian forms into natural Serbian Ekavian standard without changing the speaker's meaning.\n\
             3. NEVER merge, split, add, or drop cues. Return the EXACT same `id`s as in `translate_cues`.\n\
             4. Output ONLY a valid JSON array of objects: `[{\"id\": 1, \"text\": \"...\"}, ...]` with no markdown fences and no commentary."
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

fn build_user_prompt<'a>(
    context_before: &'a [SubtitleCue],
    target_cues: &'a [SubtitleCue],
    context_after: &'a [SubtitleCue],
) -> Result<String> {
    let to_item = |c: &'a SubtitleCue| -> PromptCueItem<'a> {
        PromptCueItem {
            id: c.id,
            text: c.source_text.as_str(),
        }
    };

    let payload = BatchPromptPayload {
        context_before: context_before.iter().map(to_item).collect(),
        translate_cues: target_cues.iter().map(to_item).collect(),
        context_after: context_after.iter().map(to_item).collect(),
    };

    serde_json::to_string(&payload).context("Failed to serialize batch prompt payload")
}

async fn call_chat_completions(
    client: &reqwest::Client,
    endpoint: &LlmEndpointConfig,
    system_prompt: &str,
    user_prompt: &str,
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
    system_prompt: &str,
    all_cues: &[SubtitleCue],
    slice_start: usize,
    slice_end: usize,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<BTreeMap<u32, String>> {
    let mut out = BTreeMap::new();
    let mut queue = vec![(slice_start, slice_end)];

    while let Some((start, end)) = queue.pop() {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        let target_cues = &all_cues[start..end];
        let ctx_before = &all_cues[start.saturating_sub(3)..start];
        let ctx_after = &all_cues[end..(end + 3).min(all_cues.len())];
        let expected_ids: Vec<u32> = target_cues.iter().map(|c| c.id).collect();
        let user_prompt = build_user_prompt(ctx_before, target_cues, ctx_after)?;

        let max_attempts = if target_cues.len() == 1 { 2 } else { 1 };
        let mut last_err = None;
        let mut succeeded = None;

        for _attempt in 0..max_attempts {
            match call_chat_completions(client, endpoint, system_prompt, &user_prompt, cancel_flag)
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
        } else if target_cues.len() > 1 {
            // Subdivide failed batch into two smaller halves to guarantee 1:1 cue ID alignment
            let mid = start + (target_cues.len() / 2);
            queue.push((mid, end));
            queue.push((start, mid));
        } else {
            return Err(last_err.unwrap_or_else(|| {
                anyhow!("Failed to translate cue id {}", target_cues[0].id)
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
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("Failed to initialize translation HTTP client")?;

    let is_serbian_target = operation == BatchOperation::PolishSerbian
        || project.options.target_language.eq_ignore_ascii_case("sr");
    let rules = segmenter::normalize_rules(&project.options.segment_rules);

    for batch_idx in 0..total_batches {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        let start = batch_idx * batch_size;
        let end = ((batch_idx + 1) * batch_size).min(project.cues.len());

        // Resume support: skip already completed batches from a saved checkpoint
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

        let batch_results = translate_slice_with_subdivision(
            &client,
            endpoint,
            &system_prompt,
            &project.cues,
            start,
            end,
            cancel_flag,
        )
        .await?;

        for cue in &mut project.cues[start..end] {
            if let Some(translated) = batch_results.get(&cue.id) {
                let scripted = if is_serbian_target {
                    sr_script::convert_script(translated, project.options.script)
                } else {
                    translated.clone()
                };
                let wrapped = segmenter::wrap_balanced_lines(
                    &scripted,
                    rules.max_chars_per_line,
                    rules.max_lines,
                );
                cue.translated_text = wrapped;
                segmenter::compute_cue_metrics(cue, &rules);
            }
        }

        project.completed_translation_batches = (batch_idx + 1) as u32;
        project.updated_at = chrono::Utc::now().to_rfc3339();
        on_checkpoint(project)?;

        let pct = ((batch_idx + 1) as f64 / total_batches as f64) * 100.0;
        let action_label = match operation {
            BatchOperation::Translate => "Translated",
            BatchOperation::PolishSerbian => "Polished",
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
        assert_eq!(map.get(&1).map(String::as_str), Some("Dobro jutro"));
        assert_eq!(map.get(&2).map(String::as_str), Some("Dobrodošli"));
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
}
