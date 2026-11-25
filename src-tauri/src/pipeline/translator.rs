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

use crate::models::{SegmentRules, SubtitleCue, SubtitleProject, TranslationStyle};
use crate::pipeline::{segmenter, sr_script};
use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;

const MIN_GAP_MS: u64 = 80;
const ORPHAN_SHIFT_MAX_GAP_MS: u64 = 800;
const MAX_TOPIC_CONTEXT_CHARS: usize = 200;
const AWS_IMDS_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptOptions<'a> {
    pub formal_style: bool,
    pub remove_fillers: bool,
    pub topic_context: Option<&'a str>,
}

impl Default for PromptOptions<'_> {
    fn default() -> Self {
        Self {
            formal_style: false,
            remove_fillers: true,
            topic_context: None,
        }
    }
}

fn sanitize_error_preview(raw: &str, max_chars: usize) -> String {
    let bounded: String = raw
        .chars()
        .take(max_chars.saturating_mul(4).max(1024))
        .collect();
    crate::diagnostics::sanitize_diagnostic_text(&bounded)
        .chars()
        .take(max_chars)
        .collect()
}

fn extract_embedded_ipv4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    // IPv4-mapped IPv6 (::ffff:a.b.c.d)
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    // Deprecated IPv4-compatible IPv6 (::a.b.c.d, excluding :: and ::1)
    if s[0] == 0
        && s[1] == 0
        && s[2] == 0
        && s[3] == 0
        && s[4] == 0
        && s[5] == 0
        && (s[6] != 0 || s[7] > 1)
    {
        return Some(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        ));
    }
    None
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_unspecified()
        || ip.octets()[0] == 0
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = extract_embedded_ipv4(ip) {
        return is_blocked_ipv4(v4);
    }
    ip.is_unspecified() || is_ipv6_link_local(ip) || ip == AWS_IMDS_V6 || ip.is_multicast()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedHost<'a> {
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
    Domain(&'a str),
}

fn parse_url_host(url: &reqwest::Url) -> Option<ParsedHost<'_>> {
    let host = url.host_str()?;
    if host.starts_with('[') && host.ends_with(']') {
        let inner = &host[1..host.len() - 1];
        return inner.parse::<Ipv6Addr>().ok().map(ParsedHost::Ipv6);
    }
    if let Ok(v4) = host.parse::<Ipv4Addr>() {
        return Some(ParsedHost::Ipv4(v4));
    }
    Some(ParsedHost::Domain(host))
}

/// Returns `true` strictly when the parsed URL targets a loopback host
/// (`127.0.0.0/8`, `::1`, or exact hostname `"localhost"` — never `"localhost.evil.com"`).
pub fn is_loopback_endpoint(url: &reqwest::Url) -> bool {
    match parse_url_host(url) {
        Some(ParsedHost::Ipv4(v4)) => v4.is_loopback(),
        Some(ParsedHost::Ipv6(v6)) => {
            v6.is_loopback() || extract_embedded_ipv4(v6).is_some_and(|v4| v4.is_loopback())
        }
        Some(ParsedHost::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn is_rfc1918_private_lan(url: &reqwest::Url) -> bool {
    match parse_url_host(url) {
        Some(ParsedHost::Ipv4(v4)) => v4.is_private(),
        Some(ParsedHost::Ipv6(v6)) => extract_embedded_ipv4(v6).is_some_and(|v4| v4.is_private()),
        _ => false,
    }
}

/// Validates a translation / LLM endpoint base URL against `SEC-H2` SSRF & TLS policy:
/// - Rejects control characters, URL credentials (`user:pass@`), query strings (`?`), and fragments (`#`).
/// - Rejects link-local (`169.254.0.0/16`, `fe80::/10`), cloud metadata (`169.254.169.254`, `fd00:ec2::254`),
///   unspecified (`0.0.0.0`, `::`), broadcast, and multicast addresses on every scheme.
/// - Requires `https://` everywhere EXCEPT:
///   1. Loopback hosts (`127.0.0.1`, `::1`, or exact `"localhost"`), and
///   2. RFC1918 private LAN IPv4 addresses (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`)
///      strictly when `has_api_key == false` so Bearer API keys never traverse LAN HTTP in cleartext.
pub fn validate_translation_base_url(raw: &str, has_api_key: bool) -> Result<reqwest::Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Translation endpoint base URL is empty"));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(anyhow!(
            "Translation endpoint base URL contains invalid control characters"
        ));
    }

    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| anyhow!("Invalid translation endpoint base URL: {e}"))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!(
            "Translation endpoint base URL must not contain embedded credentials"
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow!(
            "Translation endpoint base URL must not contain a query string or fragment"
        ));
    }

    let scheme = url.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(anyhow!(
            "Unsupported translation endpoint URL scheme '{scheme}': only https:// (or http:// for loopback / unauthenticated LAN) is allowed"
        ));
    }

    let host = parse_url_host(&url)
        .ok_or_else(|| anyhow!("Translation endpoint base URL must include a valid host"))?;

    match host {
        ParsedHost::Ipv4(v4) if is_blocked_ipv4(v4) => {
            return Err(anyhow!(
                "Translation endpoint host '{v4}' is blocked (link-local, metadata, or unspecified address)"
            ));
        }
        ParsedHost::Ipv6(v6) if is_blocked_ipv6(v6) => {
            return Err(anyhow!(
                "Translation endpoint host '[{v6}]' is blocked (link-local, metadata, or unspecified address)"
            ));
        }
        _ => {}
    }

    if scheme == "http" {
        if is_loopback_endpoint(&url) {
            return Ok(url);
        }
        if is_rfc1918_private_lan(&url) {
            if has_api_key {
                return Err(anyhow!(
                    "Refusing to send an API key over unencrypted http:// LAN endpoint; use https:// or clear the API key for unauthenticated local LAN servers"
                ));
            }
            return Ok(url);
        }
        return Err(anyhow!(
            "Translation endpoint must use https:// (unencrypted http:// is permitted only for loopback or unauthenticated RFC1918 private LAN servers)"
        ));
    }

    Ok(url)
}

fn endpoint_has_api_key(endpoint: &LlmEndpointConfig) -> bool {
    endpoint
        .api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty())
}

fn build_http_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .context("Failed to initialize translation HTTP client")
}

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

fn is_empty_marker_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("[empty]")
}

fn is_stretched_hesitation(token: &str) -> bool {
    let bytes = token.as_bytes();
    if bytes.len() < 2 || !bytes.iter().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    let all_eq = |slice: &[u8], ch: u8| !slice.is_empty() && slice.iter().all(|&b| b == ch);
    if all_eq(bytes, b'm') || all_eq(bytes, b'a') || all_eq(bytes, b'e') || all_eq(bytes, b'o') {
        return true;
    }
    // u+m+, u+h+, u+h+m+, h+m+, a+h+, e+h+, e+r+m*
    let mut i = 0;
    let consume = |idx: &mut usize, ch: u8| -> usize {
        let start = *idx;
        while *idx < bytes.len() && bytes[*idx] == ch {
            *idx += 1;
        }
        *idx - start
    };

    if bytes[0] == b'u' {
        let u = consume(&mut i, b'u');
        let h = consume(&mut i, b'h');
        let m = consume(&mut i, b'm');
        return u > 0 && (h > 0 || m > 0) && i == bytes.len();
    }
    if bytes[0] == b'h' {
        let h = consume(&mut i, b'h');
        let m = consume(&mut i, b'm');
        return h > 0 && m > 0 && i == bytes.len();
    }
    if bytes[0] == b'e' {
        let e = consume(&mut i, b'e');
        let r = consume(&mut i, b'r');
        let m = consume(&mut i, b'm');
        if e > 0 && r > 0 && i == bytes.len() {
            return true;
        }
        i = 0;
        let e2 = consume(&mut i, b'e');
        let h2 = consume(&mut i, b'h');
        return e2 > 0 && h2 > 0 && m == 0 && i == bytes.len();
    }
    if bytes[0] == b'a' {
        let a = consume(&mut i, b'a');
        let h = consume(&mut i, b'h');
        return a > 0 && h > 0 && i == bytes.len();
    }
    false
}

fn is_known_filler_token(token: &str) -> bool {
    matches!(
        token,
        "um" | "uh"
            | "uhm"
            | "erm"
            | "er"
            | "ah"
            | "eh"
            | "hmm"
            | "hm"
            | "mm"
            | "mmm"
            | "mhm"
            | "like"
            | "basically"
            | "actually"
            | "literally"
            | "well"
            | "so"
            | "right"
            | "okay"
            | "ok"
            | "yeah"
            | "yep"
            | "just"
            | "ovaj"
            | "onaj"
            | "ovako"
            | "znači"
            | "znaci"
            | "mislim"
            | "zapravo"
            | "dakle"
            | "pa"
            | "ono"
            | "bre"
            | "eto"
            | "evo"
            | "kao"
            | "znaš"
            | "znas"
            | "znate"
            | "dobro"
            | "suštinski"
            | "sustinski"
    ) || is_stretched_hesitation(token)
}

const KNOWN_FILLER_PHRASES: &[&[&str]] = &[
    &["kako", "se", "zove"],
    &["kako", "da", "kažem"],
    &["kako", "da", "kazem"],
    &["hoću", "da", "kažem"],
    &["hocu", "da", "kazem"],
    &["mislim", "da", "kažem"],
    &["mislim", "da", "kazem"],
    &["na", "neki", "način"],
    &["na", "neki", "nacin"],
    &["u", "suštini"],
    &["u", "sustini"],
    &["da", "kažem"],
    &["da", "kazem"],
    &["da", "kažemo"],
    &["da", "kazemo"],
    &["you", "know"],
    &["i", "mean"],
    &["kind", "of"],
    &["sort", "of"],
    &["you", "see"],
    &["uh", "huh"],
    &["mm", "hmm"],
];

fn is_filler_eligible_source(source: &str) -> bool {
    let raw_words: Vec<&str> = source.split_whitespace().collect();
    if raw_words.len() <= 3 {
        return true;
    }

    let latin_source = sr_script::to_latin(source);
    let tokens: Vec<String> = latin_source
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.len() <= 3 {
        return true;
    }

    let mut i = 0;
    while i < tokens.len() {
        let mut matched_phrase = false;
        for phrase in KNOWN_FILLER_PHRASES {
            if i + phrase.len() <= tokens.len()
                && phrase
                    .iter()
                    .enumerate()
                    .all(|(off, &word)| tokens[i + off] == word)
            {
                i += phrase.len();
                matched_phrase = true;
                break;
            }
        }
        if matched_phrase {
            continue;
        }
        if is_known_filler_token(&tokens[i]) {
            i += 1;
            continue;
        }
        return false;
    }

    true
}

fn insert_validated_cue_item(
    map: &mut BTreeMap<u32, ParsedCueText>,
    id: u32,
    raw_source: Option<String>,
    raw_text: &str,
    remove_fillers: bool,
    source_texts_by_id: &BTreeMap<u32, &str>,
) {
    let normalized = normalize_cue_single_line(raw_text);
    if is_empty_marker_text(&normalized) {
        if remove_fillers
            && source_texts_by_id
                .get(&id)
                .is_some_and(|src| is_filler_eligible_source(src))
        {
            map.insert(
                id,
                ParsedCueText {
                    source: None,
                    text: String::new(),
                },
            );
        }
        return;
    }

    let source = raw_source
        .map(|s| normalize_cue_single_line(&s))
        .filter(|s| !is_empty_marker_text(s));
    map.insert(
        id,
        ParsedCueText {
            source,
            text: normalized,
        },
    );
}

/// Parses the LLM JSON output and verifies that every expected cue `id` is present and valid.
/// Supports both plain string values (`"1": "..."`) and dual-column objects (`"1": {"source": "...", "text": "..."}`).
///
/// Empty strings or `"[EMPTY]"` (case-insensitive) are accepted **only** when `remove_fillers == true`
/// and the corresponding source cue in `source_texts_by_id` is short (`<= 3` words) or consists solely
/// of known filler patterns; substantive cues returning empty text are still rejected.
pub fn parse_and_validate_batch_response(
    raw: &str,
    expected_ids: &[u32],
    remove_fillers: bool,
    source_texts_by_id: &BTreeMap<u32, &str>,
) -> Result<BTreeMap<u32, ParsedCueText>> {
    let cleaned = clean_llm_json_response(raw);
    let envelope: BatchResponseEnvelope = serde_json::from_str(&cleaned).with_context(|| {
        format!(
            "Failed to parse LLM JSON response (cleaned: {})",
            sanitize_error_preview(&cleaned, 240)
        )
    })?;

    let mut map = BTreeMap::new();
    match envelope {
        BatchResponseEnvelope::Array(items) | BatchResponseEnvelope::Wrapped { items } => {
            for item in items {
                insert_validated_cue_item(
                    &mut map,
                    item.id,
                    item.source,
                    &item.text,
                    remove_fillers,
                    source_texts_by_id,
                );
            }
        }
        BatchResponseEnvelope::KeyedMap(kv) => {
            for (k, v) in kv {
                if let Ok(id) = k.trim().parse::<u32>() {
                    match v {
                        KeyedCueValue::Text(t) => {
                            insert_validated_cue_item(
                                &mut map,
                                id,
                                None,
                                &t,
                                remove_fillers,
                                source_texts_by_id,
                            );
                        }
                        KeyedCueValue::Object { source, text } => {
                            insert_validated_cue_item(
                                &mut map,
                                id,
                                source,
                                &text,
                                remove_fillers,
                                source_texts_by_id,
                            );
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

/// Drops filler-emptied cues (empty or `"[EMPTY]"` after filler removal) and re-indexes
/// remaining cues sequentially (`1..=N`).
pub fn prune_empty_filler_cues(cues: &mut Vec<SubtitleCue>) {
    let any_translated = cues
        .iter()
        .any(|c| !is_empty_marker_text(&c.translated_text));

    cues.retain(|cue| {
        if cue.translated_text.trim().eq_ignore_ascii_case("[empty]") {
            return false;
        }
        if is_empty_marker_text(&cue.source_text) && is_empty_marker_text(&cue.translated_text) {
            return false;
        }
        if is_empty_marker_text(&cue.translated_text) {
            if any_translated {
                return false;
            }
            if is_filler_eligible_source(&cue.source_text) {
                return false;
            }
        }
        true
    });

    for (idx, cue) in cues.iter_mut().enumerate() {
        cue.id = (idx + 1) as u32;
    }
}

fn build_response_format(
    endpoint: &LlmEndpointConfig,
    expected_ids: &[u32],
    correct_source_asr: bool,
    operation: BatchOperation,
) -> Option<serde_json::Value> {
    let is_local = reqwest::Url::parse(endpoint.base_url.trim())
        .map(|u| is_loopback_endpoint(&u))
        .unwrap_or(false);
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

fn is_serbian_lang(lang: &str) -> bool {
    lang.trim().eq_ignore_ascii_case("sr")
}

fn sanitize_topic_context(raw: &str) -> Option<String> {
    let cleaned_chars: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned_chars
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let capped: String = collapsed
        .chars()
        .take(MAX_TOPIC_CONTEXT_CHARS)
        .collect::<String>()
        .trim()
        .to_string();
    if capped.is_empty() {
        None
    } else {
        Some(capped)
    }
}

fn build_serbian_style_guidelines(formal_style: bool, remove_fillers: bool) -> String {
    let register_rule = if formal_style {
        "REGISTER & STYLE (Formal broadcast/documentary Serbian — standardni književni stil): \
         Use polished, clear formal Serbian and consistent formal 'Vi' address across \
         `context_before`, `translate_cues`, and `context_after`. Avoid slang while staying \
         genuinely idiomatic Serbian (u duhu srpskog jezika) without stiff English calques."
    } else {
        "REGISTER & STYLE (Natural contemporary spoken Serbian — prirodan govorni srpski): \
         Write fluid, conversational Serbian matching the speaker's tone. Maintain consistent \
         formal ('Vi') vs informal ('ti') address across `context_before`, `translate_cues`, \
         and `context_after`."
    };

    let idioms_rule = if remove_fillers {
        "NATURAL IDIOMS: Map idiomatic expressions into authentic Serbian instead of literal \
         word-for-word translations ('at the end of the day' -> 'na kraju krajeva', \
         'it makes sense' -> 'ima smisla'). Do NOT translate empty spoken fillers like \
         'you know' or 'kind of' when they carry no semantic meaning — omit them cleanly."
    } else {
        "NATURAL IDIOMS & DISCOURSE MARKERS: Map conversational expressions into authentic Serbian \
         instead of literal word-for-word translations ('you know' -> 'znaš' / 'znate', \
         'kind of' -> 'nekako' / 'na neki način', 'at the end of the day' -> 'na kraju krajeva', \
         'it makes sense' -> 'ima smisla')."
    };

    format!(
        "SERBIAN LINGUISTIC & STYLISTIC RULES (u duhu srpskog jezika):\n\
         - STRICT EKAVIAN SERBIAN: Always use standard Serbian Ekavian vocabulary \
         ('mleko', 'devojka', 'vreme', 'gde', 'lepo', 'hiljada', 'muzika', 'nedelja', 'uslov', 'voz', 'hleb') — \
         NEVER Ijekavian or Croatian lexical forms ('mlijeko', 'vrijeme', 'tko', 'krajolik', \
         'tisuća', 'glazba', 'tjedan', 'uvjet', 'vlak', 'kruh'). Use accurate Serbian grammar and cases; \
         never invent non-existent words.\n\
         - MEANING OVER SYNTAX: Translate and express meaning rather than mirroring English sentence syntax — \
         rewrite word order so dialogue sounds naturally spoken in contemporary Serbian without English calques.\n\
         - ACTIVE & REFLEXIVE VOICE: Prefer active or reflexive ('se') constructions over unnatural \
         English-style 'od strane' passives (e.g., 'it was decided by the team' -> 'tim je odlučio' / 'odlučeno je', \
         not 'bilo je odlučeno od strane tima').\n\
         - NO REDUNDANT POSSESSIVES: Eliminate unnecessary English possessives ('moj', 'tvoj', 'njegov', 'njen') \
         where Serbian omits them (body parts, family, personal items, e.g., 'stavio je ruke u džepove'), \
         and use reflexive 'svoj' when referring back to the clause subject.\n\
         - STRICT ENCLITIC PLACEMENT: Place Serbian enclitics ('je', 'sam', 'si', 'smo', 'ste', 'su', \
         'bih', 'bi', 'se', 'mi', 'ti', 'mu', 'joj', 'ga', 'ih') naturally in the second syntactic slot of the clause — \
         never leave them stranded at clause boundaries across cue breaks.\n\
         - {idioms_rule}\n\
         - {register_rule}"
    )
}

fn build_filler_guidelines(serbian_examples: bool, remove_fillers: bool) -> &'static str {
    if remove_fillers {
        if serbian_examples {
            "SPOKEN FILLER & DISFLUENCY REMOVAL:\n\
             - Omit pure conversational fillers, vocal hesitations, false starts, and stutters \
             ('znači', 'ovaj', 'onaj', 'kako se zove', 'mislim', 'you know', 'like', 'I mean', 'sort of', 'basically', 'um', 'uh') \
             whenever they carry zero semantic meaning in context.\n\
             - STRICTLY PRESERVE lexical/semantic uses of these words (e.g., \"Ovaj čovek\", \"Šta to znači?\", \"Mislim da grešiš\", \"I like it\", \"What do you mean?\").\n\
             - If a short cue consists exclusively of spoken fillers with no remaining meaning, keep its cue ID key in the JSON and output \"[EMPTY]\" (or \"\")."
        } else {
            "SPOKEN FILLER & DISFLUENCY REMOVAL:\n\
             - Omit pure conversational fillers, vocal hesitations, false starts, and stutters \
             ('you know', 'like', 'I mean', 'sort of', 'basically', 'um', 'uh') \
             whenever they carry zero semantic meaning in context.\n\
             - STRICTLY PRESERVE lexical/semantic uses of these words (e.g., \"I like it\", \"What do you mean?\").\n\
             - If a short cue consists exclusively of spoken fillers with no remaining meaning, keep its cue ID key in the JSON and output \"[EMPTY]\" (or \"\")."
        }
    } else {
        "DISCOURSE MARKER PRESERVATION:\n\
         - Preserve conversational discourse markers and spoken hesitations faithfully for verbatim readability; do not omit them or output \"[EMPTY]\"."
    }
}

/// Builds the LLM system prompt for a batch operation using full `PromptOptions`
/// (Serbian register style, context-aware spoken filler removal, and optional domain/topic context).
pub fn build_system_prompt_with_options(
    operation: BatchOperation,
    source_language: &str,
    target_language: &str,
    glossary: &BTreeMap<String, String>,
    correct_source_asr: bool,
    opts: &PromptOptions<'_>,
) -> String {
    let target_is_sr = is_serbian_lang(target_language);
    let serbian_examples =
        target_is_sr || is_serbian_lang(source_language) || operation == BatchOperation::PolishSerbian;
    let condense_is_sr = target_is_sr
        || ((target_language.trim().is_empty()
            || target_language.trim().eq_ignore_ascii_case("none"))
            && is_serbian_lang(source_language));

    let mut prompt = match operation {
        BatchOperation::Translate => {
            let target_instructions = if target_is_sr {
                "Translate into natural, idiomatic Serbian (u duhu srpskog jezika, strict Ekavian standard). \
                 If any cue is already in Serbian (e.g., a Serbian host intro before a foreign-language interview), \
                 keep it in Serbian and polish any minor ASR errors into natural Ekavian Serbian. \
                 Keep subtitle phrasing concise for comfortable reading speed."
            } else {
                "Translate accurately, idiomatically, and concisely into the target language for subtitles."
            };
            let mut base = if correct_source_asr {
                format!(
                    "You are an expert subtitle translator from '{}' to '{}' and ASR transcript proofreader. {}\n\
                     STRICT RULES:\n\
                     1. Reconstruct full sentences mentally across adjacent cues (`context_before`, `translate_cues`, `context_after`) so that phrases split across cue boundaries translate into coherent, natural grammar.\n\
                     2. ALSO fix obvious speech-to-text phonetic/homophone errors in the source text (e.g., \"Aesip\" -> \"Aesop\") based on context without altering the speaker's words, and translate using the corrected source meaning.\n\
                     3. Translate ONLY the items in `translate_cues`, preserving each cue's corresponding portion of the sentence and staying within each item's `max_chars` reading-speed budget whenever natural. Never open a new sentence with a single stranded word at the end of a cue when `context_after` carries the rest of that sentence.\n\
                     4. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                     5. Output ONLY a valid JSON object mapping each cue ID string to an object `{{\"1\": {{\"source\": \"<corrected source text>\", \"text\": \"<translated text>\"}}}}` with no markdown fences and no commentary.",
                    source_language, target_language, target_instructions
                )
            } else {
                format!(
                    "You are an expert subtitle translator from '{}' to '{}'. {}\n\
                     STRICT RULES:\n\
                     1. Reconstruct full sentences mentally across adjacent cues (`context_before`, `translate_cues`, `context_after`) so that phrases split across cue boundaries translate into coherent, natural grammar.\n\
                     2. Translate ONLY the items in `translate_cues`, preserving each cue's corresponding portion of the sentence and staying within each item's `max_chars` reading-speed budget whenever natural. Never open a new sentence with a single stranded word at the end of a cue when `context_after` carries the rest of that sentence.\n\
                     3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                     4. Output ONLY a valid JSON object mapping each cue ID string to its translated text: `{{\"1\": \"...\", \"2\": \"...\"}}` with no markdown fences and no commentary.",
                    source_language, target_language, target_instructions
                )
            };
            base.push_str("\n\n");
            base.push_str(build_filler_guidelines(serbian_examples, opts.remove_fillers));
            if target_is_sr {
                base.push_str("\n\n");
                base.push_str(&build_serbian_style_guidelines(
                    opts.formal_style,
                    opts.remove_fillers,
                ));
            }
            base
        }
        BatchOperation::PolishSerbian => {
            format!(
                "You are an expert Serbian subtitle editor and proofreader.\n\
                 STRICT RULES:\n\
                 1. Reconstruct full sentences mentally across `context_before`, `translate_cues`, and `context_after`, and polish ONLY the items in `translate_cues`. Never open a new sentence with a single stranded word at the end of a cue when `context_after` carries the rest of that sentence.\n\
                 2. Fix speech-to-text recognition errors, punctuation, capitalization, and normalize any Ijekavian/Croatian forms into natural Serbian Ekavian standard without changing the speaker's meaning or inventing words, staying within each item's `max_chars` budget when feasible.\n\
                 3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                 4. Output ONLY a valid JSON object mapping each cue ID string to its polished text: `{{\"1\": \"...\", \"2\": \"...\"}}` with no markdown fences and no commentary.\n\n\
                 {}\n\n\
                 {}",
                build_filler_guidelines(true, opts.remove_fillers),
                build_serbian_style_guidelines(opts.formal_style, opts.remove_fillers)
            )
        }
        BatchOperation::CondenseCps => {
            let mut base = "You are an expert subtitle editor specializing in tightening verbose subtitles to comfortable reading speed (CPS).\n\
                 STRICT RULES:\n\
                 1. Rewrite ONLY the cues in `translate_cues` using `context_before` and `context_after` for continuity so every rewritten cue fits cleanly within its `max_chars` budget (shorten by ~15-30% when needed).\n\
                 2. Preserve the core meaning, tone, speaker intent, and target language by cutting filler words and tightening phrasing without inventing facts.\n\
                 3. NEVER merge, split, add, or drop cues. Return the EXACT same `id` keys as in `translate_cues`.\n\
                 4. Output ONLY a valid JSON object mapping each cue ID string to its condensed text: `{\"1\": \"...\", \"2\": \"...\"}` with no markdown fences and no commentary."
                .to_string();
            if condense_is_sr {
                base.push_str("\n\n");
                base.push_str(&build_serbian_style_guidelines(
                    opts.formal_style,
                    opts.remove_fillers,
                ));
            }
            base
        }
    };

    if matches!(
        operation,
        BatchOperation::Translate | BatchOperation::PolishSerbian
    ) {
        if let Some(clean_topic) = opts.topic_context.and_then(sanitize_topic_context) {
            prompt.push_str(&format!(
                "\n\nAUDIO TOPIC & DOMAIN CONTEXT: {clean_topic}\n\
                 Use this domain context to disambiguate technical, specialized, or proper-noun terminology accurately across batches."
            ));
        }
    }

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

/// Builds the LLM system prompt for a batch operation, allowing callers to choose
/// between natural spoken Serbian (`formal_style = false`, default) and formal
/// broadcast/documentary Serbian (`formal_style = true`).
pub fn build_system_prompt_with_style(
    operation: BatchOperation,
    source_language: &str,
    target_language: &str,
    glossary: &BTreeMap<String, String>,
    correct_source_asr: bool,
    formal_style: bool,
) -> String {
    build_system_prompt_with_options(
        operation,
        source_language,
        target_language,
        glossary,
        correct_source_asr,
        &PromptOptions {
            formal_style,
            ..PromptOptions::default()
        },
    )
}

#[cfg(test)]
fn build_system_prompt(
    operation: BatchOperation,
    source_language: &str,
    target_language: &str,
    glossary: &BTreeMap<String, String>,
    correct_source_asr: bool,
) -> String {
    build_system_prompt_with_style(
        operation,
        source_language,
        target_language,
        glossary,
        correct_source_asr,
        false,
    )
}

fn ends_with_boundary_punct(token: &str) -> bool {
    let trimmed = token.trim_end_matches(['"', '\'', '”', '’', '»', ')', ']']);
    trimmed.ends_with(['.', '!', '?', '…', ',', ';', ':', '—', '–'])
}

fn is_abbreviation_or_ordinal(token: &str) -> bool {
    let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\'' | '”' | '’' | '(' | ')' | '[' | ']'));
    let Some(stem) = trimmed.strip_suffix('.') else {
        return false;
    };
    if stem.is_empty() {
        return false;
    }
    if stem.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let lower = sr_script::to_latin(stem).to_lowercase();
    matches!(
        lower.as_str(),
        "mr" | "mrs"
            | "ms"
            | "dr"
            | "prof"
            | "st"
            | "sr"
            | "jr"
            | "vs"
            | "etc"
            | "br"
            | "str"
            | "g"
            | "gđa"
            | "gđica"
            | "tj"
            | "npr"
            | "itd"
            | "sl"
            | "e.g"
            | "i.e"
    )
}

fn starts_with_dialogue_dash(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("– ")
        || trimmed.starts_with("— ")
        || trimmed == "-"
        || trimmed == "–"
        || trimmed == "—"
}

fn shift_trailing_orphan_starters_inner(cues: &mut [SubtitleCue], rules: &SegmentRules) -> bool {
    if cues.len() < 2 {
        return false;
    }

    let rules = segmenter::normalize_rules(rules);
    let max_total_chars = rules
        .max_chars_per_line
        .saturating_mul(rules.max_lines)
        .max(1) as usize;
    let mut changed = false;

    for k in 0..(cues.len() - 1) {
        let gap_ms = cues[k + 1].start_ms.saturating_sub(cues[k].end_ms);
        if gap_ms > ORPHAN_SHIFT_MAX_GAP_MS {
            continue;
        }

        let use_translated_k = !cues[k].translated_text.trim().is_empty();
        let use_translated_next = !cues[k + 1].translated_text.trim().is_empty();
        if use_translated_k != use_translated_next {
            continue;
        }

        let (text_k, text_next) = if use_translated_k {
            (
                cues[k].translated_text.clone(),
                cues[k + 1].translated_text.clone(),
            )
        } else {
            (cues[k].source_text.clone(), cues[k + 1].source_text.clone())
        };

        if starts_with_dialogue_dash(&text_next) {
            continue;
        }

        let words_k: Vec<&str> = text_k.split_whitespace().collect();
        let words_next: Vec<&str> = text_next.split_whitespace().collect();
        if words_k.len() < 2 || words_next.is_empty() {
            continue;
        }

        let penultimate = words_k[words_k.len() - 2];
        let last_word = words_k[words_k.len() - 1];

        if !ends_with_boundary_punct(penultimate) || is_abbreviation_or_ordinal(penultimate) {
            continue;
        }
        if ends_with_boundary_punct(last_word) {
            continue;
        }
        if !last_word.chars().any(|c| c.is_alphanumeric())
            || (last_word.starts_with('[') && last_word.ends_with(']'))
        {
            continue;
        }

        let new_k_flat = words_k[..words_k.len() - 1].join(" ");
        let new_next_flat = format!("{last_word} {}", words_next.join(" "));
        if new_next_flat.chars().count() > max_total_chars {
            continue;
        }

        let wrapped_k =
            segmenter::wrap_balanced_lines(&new_k_flat, rules.max_chars_per_line, rules.max_lines);
        let wrapped_next = segmenter::wrap_balanced_lines(
            &new_next_flat,
            rules.max_chars_per_line,
            rules.max_lines,
        );

        if use_translated_k {
            cues[k].translated_text = wrapped_k;
            cues[k + 1].translated_text = wrapped_next;
        } else {
            cues[k].source_text = wrapped_k;
            cues[k + 1].source_text = wrapped_next;
        }

        // Proportionally nudge a tight/bridged boundary (~120-200 ms) while enforcing >= MIN_GAP_MS
        let dur_k = cues[k].end_ms.saturating_sub(cues[k].start_ms);
        let total_chars_k = text_k.chars().count().max(1) as u64;
        let moved_chars = last_word.chars().count().max(1) as u64;
        let desired_nudge = dur_k
            .saturating_mul(moved_chars)
            .checked_div(total_chars_k)
            .unwrap_or(160)
            .clamp(120, 200);

        let min_k_dur = 300u64.min(dur_k / 2);
        let earliest_e_k = cues[k].start_ms.saturating_add(min_k_dur);
        let target_e_k = cues[k]
            .end_ms
            .saturating_sub(desired_nudge)
            .max(earliest_e_k);
        let min_next_start = target_e_k.saturating_add(MIN_GAP_MS);
        let max_next_start = cues[k + 1].end_ms.saturating_sub(MIN_GAP_MS);

        if min_next_start < cues[k + 1].end_ms && target_e_k > cues[k].start_ms {
            let new_next_start = cues[k + 1]
                .start_ms
                .saturating_sub(desired_nudge)
                .max(min_next_start)
                .min(max_next_start.max(min_next_start));
            let new_e_k = target_e_k.min(new_next_start.saturating_sub(MIN_GAP_MS));
            if new_e_k > cues[k].start_ms && new_next_start < cues[k + 1].end_ms {
                cues[k].end_ms = new_e_k;
                cues[k + 1].start_ms = new_next_start;
            }
        }

        segmenter::compute_cue_metrics(&mut cues[k], &rules);
        segmenter::compute_cue_metrics(&mut cues[k + 1], &rules);
        changed = true;
    }

    changed
}

/// Deterministic post-translation pass that shifts a stranded single unpunctuated word
/// following sentence/clause punctuation (`[. ! ? … , ; : — –]`) from the end of cue `k`
/// onto the start of cue `k + 1` (when inter-cue silence `<= 800 ms` and capacity permits),
/// re-wraps both cues, nudges tight boundaries while preserving `>= 80 ms` gap, and recomputes CPS.
pub fn shift_trailing_orphan_starters(cues: &mut [SubtitleCue], rules: &SegmentRules) {
    let _ = shift_trailing_orphan_starters_inner(cues, rules);
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

    let validated_base =
        validate_translation_base_url(&endpoint.base_url, endpoint_has_api_key(endpoint))?;
    let base = validated_base.as_str().trim_end_matches('/');
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
            sanitize_error_preview(&body_text, 300)
        ));
    }

    let parsed: ChatCompletionResponse =
        serde_json::from_str(&body_text).with_context(|| {
            format!(
                "Failed to deserialize OpenAI chat completions response: {}",
                sanitize_error_preview(&body_text, 240)
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

#[derive(Debug, Deserialize)]
struct TopicClassificationResponse {
    topic: String,
    #[serde(
        default,
        alias = "suggestedStyle",
        alias = "suggested_style",
        alias = "style"
    )]
    suggested_style: Option<String>,
}

/// Classifies a short media transcript snippet into a concise topic/domain summary and
/// recommended Serbian translation register (`Natural` vs `Formal`).
/// Enforces `SEC-H2` URL validation, zero redirects, and a short 15-second timeout.
pub async fn classify_media_topic(
    endpoint: &LlmEndpointConfig,
    snippet: &str,
    detected_language: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(String, TranslationStyle)> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Job cancelled"));
    }

    let normalized_snippet = normalize_cue_single_line(snippet);
    if normalized_snippet.is_empty() {
        return Err(anyhow!("Media topic classification snippet is empty"));
    }
    let capped_snippet: String = normalized_snippet.chars().take(1200).collect();

    let validated_base =
        validate_translation_base_url(&endpoint.base_url, endpoint_has_api_key(endpoint))?;
    let client = build_http_client(Duration::from_secs(15))?;

    let system_prompt = "You classify audio/video transcript snippets for a subtitle translation studio.\n\
         Output ONLY a strict compact JSON object with exactly two keys:\n\
         {\"topic\": \"<concise 3-7 word topic/domain summary>\", \"suggestedStyle\": \"natural\" | \"formal\"}\n\
         RULES:\n\
         - Use \"natural\" for podcasts, interviews, vlogs, dialogue, gaming, comedy, or everyday conversational speech.\n\
         - Use \"formal\" for documentaries, news broadcasts, academic/scientific lectures, legal/medical/corporate presentations, or official speeches.\n\
         - Keep `topic` concise (3-7 words, no line breaks, no markdown fences, no commentary).";

    let user_payload = serde_json::json!({
        "detected_language": detected_language
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown"),
        "snippet": capped_snippet,
    });
    let user_prompt = serde_json::to_string(&user_payload)
        .context("Failed to serialize media topic classification payload")?;

    let response_format = if is_loopback_endpoint(&validated_base) {
        Some(serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "media_topic_classification",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "topic": { "type": "string" },
                        "suggestedStyle": {
                            "type": "string",
                            "enum": ["natural", "formal"]
                        }
                    },
                    "required": ["topic", "suggestedStyle"],
                    "additionalProperties": false
                }
            }
        }))
    } else {
        Some(serde_json::json!({ "type": "json_object" }))
    };

    let raw = call_chat_completions(
        &client,
        endpoint,
        system_prompt,
        &user_prompt,
        response_format,
        cancel_flag,
    )
    .await?;

    let cleaned = clean_llm_json_response(&raw);
    let parsed: TopicClassificationResponse = serde_json::from_str(&cleaned).with_context(|| {
        format!(
            "Failed to parse media topic classification JSON (cleaned: {})",
            sanitize_error_preview(&cleaned, 240)
        )
    })?;

    let topic = sanitize_topic_context(&parsed.topic)
        .ok_or_else(|| anyhow!("Media topic classification returned an empty topic"))?;

    let style = match parsed
        .suggested_style
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        Some(s) if s == "formal" || s.starts_with("formal") || s.starts_with("knjiž") => {
            TranslationStyle::Formal
        }
        _ => TranslationStyle::Natural,
    };

    Ok((topic, style))
}

#[allow(clippy::too_many_arguments)]
async fn translate_slice_with_subdivision(
    client: &reqwest::Client,
    endpoint: &LlmEndpointConfig,
    operation: BatchOperation,
    correct_source_asr: bool,
    remove_fillers: bool,
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

    let allow_filler_empty = remove_fillers && operation != BatchOperation::CondenseCps;
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
        let source_texts_by_id: BTreeMap<u32, &str> = slice_indices
            .iter()
            .map(|&idx| (all_cues[idx].id, raw_for_op(operation, &all_cues[idx])))
            .collect();
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
            .and_then(|raw| {
                parse_and_validate_batch_response(
                    &raw,
                    &expected_ids,
                    allow_filler_empty,
                    &source_texts_by_id,
                )
            }) {
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

/// Runs the ID-locked overlapping context-window batch protocol across `project.cues`
/// with explicit `remove_fillers` and optional domain `topic_context` prompt options.
#[allow(clippy::too_many_arguments)]
pub async fn run_batch_operation_with_options<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    operation: BatchOperation,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    mut on_checkpoint: FCheckpoint,
    mut on_progress: FProgress,
    remove_fillers: bool,
    topic_context: Option<&str>,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    if project.cues.is_empty() {
        return Ok(());
    }

    let _validated_url =
        validate_translation_base_url(&endpoint.base_url, endpoint_has_api_key(endpoint))?;

    let batch_size = endpoint.batch_size.clamp(1, 50) as usize;
    let total_batches = project.cues.len().div_ceil(batch_size);

    let effective_source_lang = if project
        .options
        .source_language
        .trim()
        .eq_ignore_ascii_case("auto")
    {
        project
            .detected_language
            .as_deref()
            .filter(|s| !s.trim().is_empty() && !s.trim().eq_ignore_ascii_case("auto"))
            .unwrap_or(&project.options.source_language)
    } else {
        &project.options.source_language
    };

    let prompt_options = PromptOptions {
        formal_style: project.options.translation_style == TranslationStyle::Formal,
        remove_fillers,
        topic_context,
    };
    let system_prompt = build_system_prompt_with_options(
        operation,
        effective_source_lang,
        &project.options.target_language,
        &project.options.glossary,
        project.options.correct_source_asr,
        &prompt_options,
    );

    let client = build_http_client(Duration::from_secs(120))?;

    let is_serbian_target = operation == BatchOperation::PolishSerbian
        || is_serbian_lang(&project.options.target_language)
        || (operation == BatchOperation::CondenseCps
            && (project.options.target_language.trim().is_empty()
                || project
                    .options
                    .target_language
                    .trim()
                    .eq_ignore_ascii_case("none"))
            && (project.detected_language.as_deref().is_some_and(is_serbian_lang)
                || is_serbian_lang(&project.options.source_language)));
    let rules = segmenter::normalize_rules(&project.options.segment_rules);
    let active_style = project.options.translation_style;

    if operation != BatchOperation::CondenseCps {
        match project.completed_translation_style {
            Some(prev_style) if prev_style != active_style => {
                // Switching styles: preserve previous cached translations and re-translate from batch 0
                for cue in &mut project.cues {
                    if !cue.translated_text.trim().is_empty()
                        && !cue.translated_variants.contains_key(&prev_style)
                    {
                        cue.translated_variants
                            .insert(prev_style, cue.translated_text.clone());
                    }
                }
                project.completed_translation_batches = 0;
                project.completed_translation_style = Some(active_style);
            }
            None if project.completed_translation_batches > 0 => {
                // Legacy pre-v0.4.0 checkpoint resuming without a style tag
                for cue in &mut project.cues {
                    if cue.translated_variants.is_empty() && !cue.translated_text.trim().is_empty()
                    {
                        cue.translated_variants
                            .insert(active_style, cue.translated_text.clone());
                    }
                }
                project.completed_translation_style = Some(active_style);
            }
            None => {
                project.completed_translation_style = Some(active_style);
            }
            Some(_) => {}
        }
    }

    for batch_idx in 0..total_batches {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Job cancelled"));
        }

        let start = batch_idx * batch_size;
        let end = ((batch_idx + 1) * batch_size).min(project.cues.len());

        // Resume support: skip already completed batches from a saved checkpoint ONLY when
        // they were generated for `active_style` (CondenseCps always inspects high-CPS cues).
        if operation != BatchOperation::CondenseCps {
            let already_translated = project.completed_translation_style == Some(active_style)
                && (batch_idx as u32) < project.completed_translation_batches
                && project.cues[start..end].iter().all(|c| {
                    c.translated_variants
                        .get(&active_style)
                        .is_some_and(|v| !v.trim().is_empty())
                });

            if already_translated {
                for cue in &mut project.cues[start..end] {
                    if let Some(cached) = cue.translated_variants.get(&active_style).cloned() {
                        if cue.translated_text != cached {
                            cue.translated_text = cached;
                            segmenter::compute_cue_metrics(cue, &rules);
                        }
                    }
                }
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
            remove_fillers,
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
                if parsed.text.is_empty() && operation != BatchOperation::CondenseCps {
                    cue.translated_text.clear();
                    cue.translated_variants.remove(&active_style);
                    segmenter::compute_cue_metrics(cue, &rules);
                    continue;
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
                    cue.translated_text = wrapped.clone();
                    cue.translated_variants.insert(active_style, wrapped);
                }
                segmenter::compute_cue_metrics(cue, &rules);
            }
        }

        if operation != BatchOperation::CondenseCps {
            project.completed_translation_batches = (batch_idx + 1) as u32;
            project.completed_translation_style = Some(active_style);
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

    if matches!(
        operation,
        BatchOperation::Translate | BatchOperation::PolishSerbian
    ) {
        let mut post_pass_changed = false;
        if remove_fillers {
            let before_len = project.cues.len();
            prune_empty_filler_cues(&mut project.cues);
            if project.cues.len() != before_len {
                post_pass_changed = true;
            }
        }
        if shift_trailing_orphan_starters_inner(&mut project.cues, &rules) {
            for cue in &mut project.cues {
                if !cue.translated_text.trim().is_empty() {
                    cue.translated_variants
                        .insert(active_style, cue.translated_text.clone());
                }
            }
            post_pass_changed = true;
        }
        let timing_snapshot: Vec<(u64, u64)> = project
            .cues
            .iter()
            .map(|c| (c.start_ms, c.end_ms))
            .collect();
        segmenter::enforce_min_inter_cue_gap(&mut project.cues, &rules);
        if project
            .cues
            .iter()
            .zip(timing_snapshot.iter())
            .any(|(c, &(s, e))| c.start_ms != s || c.end_ms != e)
        {
            post_pass_changed = true;
        }
        if post_pass_changed {
            project.updated_at = chrono::Utc::now().to_rfc3339();
            on_checkpoint(project)?;
        }
    }

    Ok(())
}

/// Runs the ID-locked overlapping context-window batch protocol across `project.cues`.
/// Delegates to `run_batch_operation_with_options` using `project.options.remove_fillers`
/// and `project.topic_context`.
pub async fn run_batch_operation<FCheckpoint, FProgress>(
    project: &mut SubtitleProject,
    operation: BatchOperation,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
    on_checkpoint: FCheckpoint,
    on_progress: FProgress,
) -> Result<()>
where
    FCheckpoint: FnMut(&SubtitleProject) -> Result<()>,
    FProgress: FnMut(f64, String),
{
    let remove_fillers = project.options.remove_fillers;
    let topic_context = project.topic_context.clone();
    run_batch_operation_with_options(
        project,
        operation,
        endpoint,
        cancel_flag,
        on_checkpoint,
        on_progress,
        remove_fillers,
        topic_context.as_deref(),
    )
    .await
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
                    translated_variants: BTreeMap::new(),
                    cps: 11.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 2,
                    start_ms: 2000,
                    end_ms: 4000,
                    source_text: "Welcome to Belgrade.".to_string(),
                    translated_text: String::new(),
                    translated_variants: BTreeMap::new(),
                    cps: 10.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 3,
                    start_ms: 4000,
                    end_ms: 6000,
                    source_text: "Love and attention plays an important role in our lives today.".to_string(),
                    translated_text: String::new(),
                    translated_variants: BTreeMap::new(),
                    cps: 15.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 4,
                    start_ms: 6000,
                    end_ms: 8000,
                    source_text: "Thank you for watching!".to_string(),
                    translated_text: String::new(),
                    translated_variants: BTreeMap::new(),
                    cps: 11.5,
                    warnings: vec![],
                },
            ],
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-22T18:00:00Z".to_string(),
        }
    }

    #[test]
    fn cleans_think_tags_and_markdown_fences() {
        let raw = "<think>\nLet me translate cues 1 and 2 carefully.\n</think>\n```json\n[\n  {\"id\": 1, \"text\": \"Dobro jutro\"},\n  {\"id\": 2, \"text\": \"Dobrodošli\"}\n]\n```";
        let map = parse_and_validate_batch_response(raw, &[1, 2], false, &BTreeMap::new()).unwrap();
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
        // Batch 2 succeeded on first try (1 call) => 4 total HTTP calls.
        // Checkpoints: 2 batch checkpoints + 1 post-pass checkpoint after enforcing 80ms gap on 0ms fixture cues.
        assert_eq!(call_counter.load(Ordering::SeqCst), 4);
        assert_eq!(checkpoints_saved, 3);
        assert_eq!(progress_events, 2);
        assert_eq!(project.completed_translation_batches, 2);
        for pair in project.cues.windows(2) {
            assert!(
                pair[1].start_ms.saturating_sub(pair[0].end_ms) >= MIN_GAP_MS,
                "expected >= 80ms inter-cue gap after post-pass"
            );
        }

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
        let map = parse_and_validate_batch_response(raw, &[1, 2], false, &BTreeMap::new()).unwrap();
        assert_eq!(
            map.get(&1).map(|v| v.text.as_str()),
            Some("Zdravo [SMEH] svima.")
        );
        assert_eq!(map.get(&2).map(|v| v.text.as_str()), Some("Kako ste?"));

        let dual_raw = "{\"1\": {\"source\": \"Section 37 of Aesop.\", \"text\": \"Odeljak 37 Ezopa.\"}, \"2\": \"Kako ste?\"}";
        let dual_map =
            parse_and_validate_batch_response(dual_raw, &[1, 2], false, &BTreeMap::new()).unwrap();
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
                    translated_variants: BTreeMap::new(),
                    cps: 16.0,
                    warnings: vec![],
                },
                SubtitleCue {
                    id: 2,
                    start_ms: 4000,
                    end_ms: 6500,
                    source_text: "Welcome to Bellgrade!".to_string(),
                    translated_text: String::new(),
                    translated_variants: BTreeMap::new(),
                    cps: 8.0,
                    warnings: vec![],
                },
            ],
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
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

        // 1 batch checkpoint + 1 post-pass checkpoint after enforcing >= 80ms gap on 0ms fixture boundary
        assert_eq!(checkpoints, 2);
        assert_eq!(project.completed_translation_batches, 1);
        assert!(
            project.cues[1].start_ms.saturating_sub(project.cues[0].end_ms) >= MIN_GAP_MS,
            "expected >= 80ms inter-cue gap after post-pass"
        );

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

    fn assert_has_serbian_stylistic_rules(
        prompt: &str,
        formal_style: bool,
        remove_fillers: bool,
    ) {
        assert!(
            prompt.contains("u duhu srpskog jezika"),
            "missing idiomatic Serbian header in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("MEANING OVER SYNTAX") && prompt.contains("calques"),
            "missing anti-calque rule in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("od strane") && prompt.contains("tim je odlučio"),
            "missing active/reflexive voice rule in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("moj") && prompt.contains("svoj"),
            "missing redundant possessive / reflexive svoj rule in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("STRICT ENCLITIC PLACEMENT")
                && prompt.contains("'je'")
                && prompt.contains("'sam'")
                && prompt.contains("'bih'")
                && prompt.contains("'se'")
                && prompt.contains("second syntactic slot"),
            "missing enclitic placement rule in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("na kraju krajeva") && prompt.contains("ima smisla"),
            "missing natural idioms rule in prompt:\n{prompt}"
        );
        if remove_fillers {
            assert!(
                !prompt.contains("'you know' -> 'znaš'"),
                "filler-removal prompt must not instruct translating 'you know' -> 'znaš':\n{prompt}"
            );
        } else {
            assert!(
                prompt.contains("znaš") && prompt.contains("nekako"),
                "verbatim prompt (remove_fillers=false) must retain discourse marker mapping:\n{prompt}"
            );
        }
        assert!(
            prompt.contains("context_before")
                && prompt.contains("translate_cues")
                && prompt.contains("context_after")
                && prompt.contains("'Vi'"),
            "missing register consistency rule in prompt:\n{prompt}"
        );
        assert!(
            prompt.contains("NEVER merge, split, add, or drop cues")
                && prompt.contains("valid JSON object"),
            "missing strict JSON / cue ID preservation rules in prompt:\n{prompt}"
        );

        if formal_style {
            assert!(
                prompt.contains("Formal broadcast/documentary Serbian")
                    && prompt.contains("standardni književni stil"),
                "expected formal register guidance in prompt:\n{prompt}"
            );
            assert!(
                !prompt.contains("prirodan govorni srpski"),
                "formal prompt should not include natural spoken style label:\n{prompt}"
            );
        } else {
            assert!(
                prompt.contains("Natural contemporary spoken Serbian")
                    && prompt.contains("prirodan govorni srpski")
                    && prompt.contains("'ti'"),
                "expected natural spoken register guidance in prompt:\n{prompt}"
            );
            assert!(
                !prompt.contains("standardni književni stil"),
                "natural prompt should not include formal documentary style label:\n{prompt}"
            );
        }
    }

    #[test]
    fn serbian_prompts_include_idiomatic_rules_for_natural_and_formal_styles() {
        let mut glossary = BTreeMap::new();
        glossary.insert("EasySRT".to_string(), "EasySRT".to_string());

        let serbian_cases = [
            (BatchOperation::Translate, "en", "sr", false),
            (BatchOperation::Translate, "en", "sr", true),
            (BatchOperation::PolishSerbian, "sr", "sr", false),
            (BatchOperation::CondenseCps, "en", "sr", false),
            (BatchOperation::CondenseCps, "sr", "none", false),
        ];

        for (op, src, dst, correct_asr) in serbian_cases {
            let default_prompt = build_system_prompt(op, src, dst, &glossary, correct_asr);
            let natural_prompt =
                build_system_prompt_with_style(op, src, dst, &glossary, correct_asr, false);
            let formal_prompt =
                build_system_prompt_with_style(op, src, dst, &glossary, correct_asr, true);
            let verbatim_natural = build_system_prompt_with_options(
                op,
                src,
                dst,
                &glossary,
                correct_asr,
                &PromptOptions {
                    formal_style: false,
                    remove_fillers: false,
                    topic_context: None,
                },
            );

            assert_eq!(
                default_prompt, natural_prompt,
                "build_system_prompt must default to formal_style = false"
            );

            assert_has_serbian_stylistic_rules(&natural_prompt, false, true);
            assert_has_serbian_stylistic_rules(&formal_prompt, true, true);
            assert_has_serbian_stylistic_rules(&verbatim_natural, false, false);

            // Verify glossary terms are preserved in both styles
            assert!(natural_prompt.contains("MANDATORY GLOSSARY TERMS:"));
            assert!(natural_prompt.contains("- EasySRT => EasySRT"));
            assert!(formal_prompt.contains("MANDATORY GLOSSARY TERMS:"));
            assert!(formal_prompt.contains("- EasySRT => EasySRT"));

            if op == BatchOperation::Translate && correct_asr {
                assert!(natural_prompt.contains("\"source\""));
                assert!(natural_prompt.contains("Aesop"));
                assert!(formal_prompt.contains("\"source\""));
                assert!(formal_prompt.contains("Aesop"));
            }
        }
    }

    #[test]
    fn non_serbian_prompts_do_not_inject_serbian_stylistic_rules() {
        let mut glossary = BTreeMap::new();
        glossary.insert("Belgrade".to_string(), "Belgrade".to_string());

        let non_serbian_cases = [
            (BatchOperation::Translate, "sr", "en", false),
            (BatchOperation::Translate, "en", "de", false),
            (BatchOperation::Translate, "en", "de", true),
            (BatchOperation::CondenseCps, "en", "en", false),
            (BatchOperation::CondenseCps, "en", "de", false),
            (BatchOperation::CondenseCps, "en", "none", false),
        ];

        let forbidden_serbian_markers = [
            "u duhu srpskog jezika",
            "Ekavian",
            "od strane",
            "STRICT ENCLITIC PLACEMENT",
            "na kraju krajeva",
            "prirodan govorni srpski",
            "standardni književni stil",
            "svoj",
        ];

        for (op, src, dst, correct_asr) in non_serbian_cases {
            for formal_style in [false, true] {
                let prompt = build_system_prompt_with_style(
                    op,
                    src,
                    dst,
                    &glossary,
                    correct_asr,
                    formal_style,
                );
                for marker in forbidden_serbian_markers {
                    assert!(
                        !prompt.contains(marker),
                        "non-Serbian prompt ({op:?}, {src}->{dst}, formal={formal_style}) unexpectedly contained `{marker}`:\n{prompt}"
                    );
                }
                assert!(prompt.contains("valid JSON object"));
                assert!(prompt.contains("NEVER merge, split, add, or drop cues"));
                assert!(prompt.contains("- Belgrade => Belgrade"));
            }
        }
    }

    #[tokio::test]
    async fn run_batch_operation_passes_translation_style_to_system_prompt() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured_prompts = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let captured_clone = Arc::clone(&captured_prompts);

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
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

                let req_str = String::from_utf8_lossy(&buf[..total_read]);
                let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
                let req_json: serde_json::Value =
                    serde_json::from_str(body_part).unwrap_or_default();
                let sys_prompt = req_json["messages"][0]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                captured_clone.lock().unwrap().push(sys_prompt);

                let resp_json = serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "{\"1\": \"Dobro jutro svima.\"}"
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
            }
        });

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "mock-model".to_string(),
            api_key: None,
            batch_size: 10,
        };
        let cancel = Arc::new(AtomicBool::new(false));

        let mut natural_project = sample_project(SerbianScript::Latin);
        natural_project.cues.truncate(1);
        natural_project.options.translation_style = TranslationStyle::Natural;
        run_batch_operation(
            &mut natural_project,
            BatchOperation::Translate,
            &endpoint,
            &cancel,
            |_| Ok(()),
            |_, _| {},
        )
        .await
        .unwrap();

        let mut formal_project = sample_project(SerbianScript::Latin);
        formal_project.cues.truncate(1);
        formal_project.options.translation_style = TranslationStyle::Formal;
        run_batch_operation(
            &mut formal_project,
            BatchOperation::Translate,
            &endpoint,
            &cancel,
            |_| Ok(()),
            |_, _| {},
        )
        .await
        .unwrap();

        server.await.unwrap();

        let prompts = captured_prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(
            prompts[0].contains("prirodan govorni srpski"),
            "Natural style must send natural Serbian prompt: {}",
            prompts[0]
        );
        assert!(
            !prompts[0].contains("standardni književni stil"),
            "Natural style must not send formal prompt: {}",
            prompts[0]
        );
        assert!(
            prompts[1].contains("standardni književni stil"),
            "Formal style must send formal Serbian prompt: {}",
            prompts[1]
        );
        assert!(
            !prompts[1].contains("prirodan govorni srpski"),
            "Formal style must not send natural prompt: {}",
            prompts[1]
        );
    }

    #[test]
    fn sec_h2_url_policy_and_loopback_detection_matrix() {
        // HTTPS cloud endpoints: allowed with or without API key; NOT loopback
        for has_key in [false, true] {
            let url = validate_translation_base_url("https://api.deepseek.com/v1", has_key)
                .expect("https cloud URL must be allowed");
            assert!(!is_loopback_endpoint(&url));
        }

        // Plaintext HTTP cloud endpoints: always rejected
        for has_key in [false, true] {
            assert!(validate_translation_base_url("http://api.deepseek.com/v1", has_key).is_err());
            assert!(validate_translation_base_url("http://remote.example.com/v1", has_key).is_err());
            assert!(validate_translation_base_url("http://8.8.8.8:8080/v1", has_key).is_err());
        }

        // Loopback endpoints: allowed on http:// with or without API key, detected as loopback
        for raw in [
            "http://127.0.0.1:8080/v1",
            "http://localhost:11434/v1",
            "http://[::1]:8080/v1",
            "https://127.0.0.1:8443/v1",
        ] {
            for has_key in [false, true] {
                let url = validate_translation_base_url(raw, has_key)
                    .unwrap_or_else(|e| panic!("loopback {raw} (has_key={has_key}) failed: {e}"));
                assert!(
                    is_loopback_endpoint(&url),
                    "expected loopback=true for {raw}"
                );
            }
        }

        // Lookalike hostname MUST NOT be treated as loopback!
        let evil_https = validate_translation_base_url("https://localhost.evil.com/v1", true)
            .expect("https domain is allowed");
        assert!(!is_loopback_endpoint(&evil_https));
        assert!(validate_translation_base_url("http://localhost.evil.com/v1", false).is_err());
        assert!(validate_translation_base_url("http://127.0.0.1.evil.com/v1", false).is_err());
        let evil_ep = LlmEndpointConfig {
            base_url: "https://localhost.evil.com/v1".to_string(),
            model: "m".to_string(),
            api_key: Some("k".to_string()),
            batch_size: 10,
        };
        let evil_rf = build_response_format(&evil_ep, &[1], false, BatchOperation::Translate).unwrap();
        assert_eq!(evil_rf["type"], "json_object");

        // RFC1918 private LAN HTTP: allowed ONLY when has_api_key == false; rejected when has_api_key == true
        for lan in [
            "http://192.168.1.50:11434/v1",
            "http://10.0.0.15:8080/v1",
            "http://172.16.0.10:8080/v1",
            "http://172.31.255.4:8080/v1",
            "http://[::ffff:192.168.1.50]:11434/v1",
        ] {
            let url = validate_translation_base_url(lan, false)
                .unwrap_or_else(|e| panic!("unauthenticated LAN {lan} should be allowed: {e}"));
            assert!(!is_loopback_endpoint(&url));
            assert!(
                validate_translation_base_url(lan, true).is_err(),
                "authenticated HTTP LAN {lan} must be rejected to protect Bearer key"
            );
        }
        // HTTPS on private LAN WITH API key is permitted
        assert!(validate_translation_base_url("https://192.168.1.50:11434/v1", true).is_ok());

        // Non-RFC1918 172.x addresses must be rejected on http:// even without key
        assert!(validate_translation_base_url("http://172.15.0.1:8080/v1", false).is_err());
        assert!(validate_translation_base_url("http://172.32.0.1:8080/v1", false).is_err());

        // Link-local, cloud metadata, unspecified, and IPv4-mapped metadata addresses: always rejected
        for blocked in [
            "http://169.254.169.254/latest/meta-data",
            "https://169.254.169.254/v1",
            "http://169.254.1.2:8080/v1",
            "http://[fe80::1]:8080/v1",
            "https://[fe80::1]/v1",
            "http://[fd00:ec2::254]/v1",
            "https://[fd00:ec2::254]/v1",
            "http://[::ffff:169.254.169.254]/v1",
            "https://[::ffff:169.254.169.254]/v1",
            "http://0.0.0.0:8080/v1",
            "https://0.0.0.0/v1",
            "http://[::]:8080/v1",
            "https://[::]/v1",
        ] {
            for has_key in [false, true] {
                assert!(
                    validate_translation_base_url(blocked, has_key).is_err(),
                    "blocked URL {blocked} (has_key={has_key}) must be rejected"
                );
            }
        }

        // Credentials, query strings, fragments, non-HTTP schemes, control chars: always rejected
        for malformed in [
            "https://user:pass@api.deepseek.com/v1",
            "https://user@api.deepseek.com/v1",
            "https://:pass@api.deepseek.com/v1",
            "https://api.deepseek.com/v1?api_key=sk-1234567890",
            "https://api.deepseek.com/v1?",
            "https://api.deepseek.com/v1#fragment",
            "https://api.deepseek.com/v1#",
            "file:///etc/passwd",
            "ftp://api.deepseek.com/v1",
            "https://api.deepseek.com/v1\nX-Injected: 1",
            "",
        ] {
            assert!(
                validate_translation_base_url(malformed, false).is_err(),
                "malformed URL `{malformed}` must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn sec_h2_blocks_redirects_and_sanitizes_http_error_previews() {
        let secret_hits = Arc::new(AtomicUsize::new(0));
        let secret_hits_clone = Arc::clone(&secret_hits);

        // Secondary server that should NEVER receive a redirected request
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let target_task = tokio::spawn(async move {
            if let Ok((mut sock, _)) = target_listener.accept().await {
                secret_hits_clone.fetch_add(1, Ordering::SeqCst);
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                    .await;
            }
        });

        // Primary mock server: request 1 returns 302 redirect; request 2 returns 401 with secret leak in body
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            for step in 0..2 {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let resp = if step == 0 {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{}/v1/chat/completions\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        target_addr.port()
                    )
                } else {
                    let body = "Unauthorized: invalid key sk-supersecret987654321 for Bearer tok_abc123 at /home/alice/config.json";
                    format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                };
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let client = build_http_client(Duration::from_secs(5)).unwrap();
        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "test-model".to_string(),
            api_key: Some("sk-supersecret987654321".to_string()),
            batch_size: 5,
        };
        let cancel = Arc::new(AtomicBool::new(false));

        // 1. 302 redirect must be rejected without following Location
        let redirect_err = call_chat_completions(&client, &endpoint, "sys", "usr", None, &cancel)
            .await
            .expect_err("302 redirect must fail when redirects are disabled");
        assert!(
            redirect_err.to_string().contains("302"),
            "expected HTTP 302 error, got: {redirect_err}"
        );
        assert_eq!(
            secret_hits.load(Ordering::SeqCst),
            0,
            "redirect target must never be contacted"
        );

        // 2. 401 error body preview must redact API keys, Bearer tokens, and usernames
        let auth_err = call_chat_completions(&client, &endpoint, "sys", "usr", None, &cancel)
            .await
            .expect_err("401 response must fail");
        let msg = auth_err.to_string();
        assert!(msg.contains("401"), "expected HTTP 401 in error: {msg}");
        assert!(
            !msg.contains("sk-supersecret987654321"),
            "API key leaked in error preview: {msg}"
        );
        assert!(
            !msg.contains("tok_abc123"),
            "Bearer token leaked in error preview: {msg}"
        );
        assert!(
            !msg.contains("/home/alice/"),
            "username path leaked in error preview: {msg}"
        );
        assert!(msg.contains("[REDACTED_API_KEY]"), "missing key redaction marker: {msg}");
        assert!(msg.contains("Bearer [REDACTED]"), "missing bearer redaction marker: {msg}");
        assert!(msg.contains("/home/<USER>/"), "missing home path redaction marker: {msg}");

        server_task.abort();
        target_task.abort();
    }

    #[test]
    fn filler_removal_prompt_toggle_across_translate_and_polish() {
        let glossary = BTreeMap::new();
        for (op, src, dst) in [
            (BatchOperation::Translate, "en", "sr"),
            (BatchOperation::PolishSerbian, "sr", "sr"),
        ] {
            let remove_on = build_system_prompt_with_options(
                op,
                src,
                dst,
                &glossary,
                false,
                &PromptOptions {
                    formal_style: false,
                    remove_fillers: true,
                    topic_context: None,
                },
            );
            assert!(remove_on.contains("SPOKEN FILLER & DISFLUENCY REMOVAL"));
            assert!(remove_on.contains("'znači'") && remove_on.contains("'ovaj'"));
            assert!(remove_on.contains("'you know'") && remove_on.contains("'um'"));
            assert!(remove_on.contains("Ovaj čovek"));
            assert!(remove_on.contains("Šta to znači?"));
            assert!(remove_on.contains("Mislim da grešiš"));
            assert!(remove_on.contains("\"[EMPTY]\""));
            assert!(remove_on.contains("Never open a new sentence with a single stranded word"));
            assert!(!remove_on.contains("'you know' -> 'znaš'"));
            assert!(!remove_on.contains("DISCOURSE MARKER PRESERVATION"));

            let remove_off = build_system_prompt_with_options(
                op,
                src,
                dst,
                &glossary,
                false,
                &PromptOptions {
                    formal_style: false,
                    remove_fillers: false,
                    topic_context: None,
                },
            );
            assert!(remove_off.contains("DISCOURSE MARKER PRESERVATION"));
            assert!(!remove_off.contains("SPOKEN FILLER & DISFLUENCY REMOVAL"));
            assert!(remove_off.contains("'you know' -> 'znaš'"));
            assert!(remove_off.contains("'kind of' -> 'nekako'"));
        }
    }

    #[test]
    fn filler_only_cues_validated_and_pruned_while_substantive_empty_rejected() {
        let sources = BTreeMap::from([
            (1u32, "Um, you know..."),
            (2u32, "Pa, znači, ovaj, kako se zove, mislim..."),
            (3u32, "Овај, значи, како се зове, мислим..."),
            (4u32, "Good morning everyone, welcome to Belgrade today."),
        ]);

        // 1. remove_fillers == true accepts "" and "[EMPTY]" (case-insensitive) on short/filler cues
        let raw_ok = r#"{"1": "[EMPTY]", "2": "", "3": "[empty]", "4": "Dobro jutro svima, dobrodošli u Beograd danas."}"#;
        let parsed = parse_and_validate_batch_response(raw_ok, &[1, 2, 3, 4], true, &sources)
            .expect("filler-only cues returning empty/[EMPTY] must validate when remove_fillers=true");
        assert_eq!(parsed.get(&1).map(|v| v.text.as_str()), Some(""));
        assert_eq!(parsed.get(&2).map(|v| v.text.as_str()), Some(""));
        assert_eq!(parsed.get(&3).map(|v| v.text.as_str()), Some(""));
        assert_eq!(
            parsed.get(&4).map(|v| v.text.as_str()),
            Some("Dobro jutro svima, dobrodošli u Beograd danas.")
        );

        // 2. Substantive multi-word cue returning "" or "[EMPTY]" MUST STILL BE REJECTED!
        let raw_dropped_substantive = r#"{"1": "Pa dobro.", "4": "[EMPTY]"}"#;
        assert!(
            parse_and_validate_batch_response(raw_dropped_substantive, &[1, 4], true, &sources)
                .is_err(),
            "substantive multi-word cue returning [EMPTY] must be rejected"
        );
        let raw_blank_substantive = r#"{"1": "Pa dobro.", "4": ""}"#;
        assert!(
            parse_and_validate_batch_response(raw_blank_substantive, &[1, 4], true, &sources)
                .is_err(),
            "substantive multi-word cue returning empty string must be rejected"
        );

        // 3. When remove_fillers == false, even short filler cues returning "" / "[EMPTY]" are rejected
        assert!(
            parse_and_validate_batch_response(r#"{"1": "[EMPTY]"}"#, &[1], false, &sources).is_err()
        );
        assert!(parse_and_validate_batch_response(r#"{"1": ""}"#, &[1], false, &sources).is_err());

        // 4. prune_empty_filler_cues drops emptied cues and re-indexes remaining IDs 1..=N
        let mut cues = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "Good morning.".to_string(),
                translated_text: "Dobro jutro.".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 8.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 1580,
                end_ms: 2200,
                source_text: "Um, you know...".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 0.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 3,
                start_ms: 2280,
                end_ms: 2800,
                source_text: "Znači, ovaj...".to_string(),
                translated_text: "[EMPTY]".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 0.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 4,
                start_ms: 2880,
                end_ms: 4500,
                source_text: "Welcome to Belgrade.".to_string(),
                translated_text: "Dobrodošli u Beograd.".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 12.9,
                warnings: vec![],
            },
        ];
        prune_empty_filler_cues(&mut cues);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].id, 1);
        assert_eq!(cues[0].translated_text, "Dobro jutro.");
        assert_eq!(cues[1].id, 2);
        assert_eq!(cues[1].translated_text, "Dobrodošli u Beograd.");

        // Single-cue all-filler project also prunes cleanly to empty vec
        let mut single_filler = vec![SubtitleCue {
            id: 1,
            start_ms: 0,
            end_ms: 900,
            source_text: "Um, you know...".to_string(),
            translated_text: String::new(),
            translated_variants: BTreeMap::new(),
            cps: 0.0,
            warnings: vec![],
        }];
        prune_empty_filler_cues(&mut single_filler);
        assert!(single_filler.is_empty());
    }

    #[tokio::test]
    async fn run_batch_operation_with_options_prunes_filler_cues_and_shifts_orphan_starters() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured_sys_prompt = Arc::new(std::sync::Mutex::new(String::new()));
        let captured_clone = Arc::clone(&captured_sys_prompt);

        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
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
                let req_str = String::from_utf8_lossy(&buf[..total_read]);
                let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
                let req_json: serde_json::Value =
                    serde_json::from_str(body_part).unwrap_or_default();
                *captured_clone.lock().unwrap() = req_json["messages"][0]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let resp_json = serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "{\"1\": \"[EMPTY]\", \"2\": \"Ne znam. Šta\", \"3\": \"radimo ovde?\"}"
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
            }
        });

        let mut project = sample_project(SerbianScript::Latin);
        project.cues = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 800,
                source_text: "Um, you know...".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 10.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 900,
                end_ms: 2500,
                source_text: "I don't know. What".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 11.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 3,
                start_ms: 2500,
                end_ms: 4200,
                source_text: "are we doing here?".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 11.0,
                warnings: vec![],
            },
        ];

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 10,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut checkpoints = 0usize;

        run_batch_operation_with_options(
            &mut project,
            BatchOperation::Translate,
            &endpoint,
            &cancel,
            |_| {
                checkpoints += 1;
                Ok(())
            },
            |_, _| {},
            true,
            Some("Tehnološki podkast o AI"),
        )
        .await
        .unwrap();

        server.await.unwrap();

        let prompt = captured_sys_prompt.lock().unwrap().clone();
        assert!(prompt.contains("AUDIO TOPIC & DOMAIN CONTEXT: Tehnološki podkast o AI"));
        assert!(prompt.contains("SPOKEN FILLER & DISFLUENCY REMOVAL"));

        // Cue 1 pruned, remaining re-indexed to 1..=2, orphan starter "Šta" shifted to cue 2 with >= 80ms gap
        assert_eq!(project.cues.len(), 2);
        assert_eq!(project.cues[0].id, 1);
        assert_eq!(project.cues[0].translated_text, "Ne znam.");
        assert_eq!(project.cues[1].id, 2);
        assert_eq!(project.cues[1].translated_text, "Šta radimo ovde?");
        assert!(
            project.cues[1].start_ms.saturating_sub(project.cues[0].end_ms) >= MIN_GAP_MS,
            "gap must be >= 80ms, got {} -> {}",
            project.cues[0].end_ms,
            project.cues[1].start_ms
        );
        // Batch checkpoint + post-pass checkpoint after pruning/shifting
        assert_eq!(checkpoints, 2);
    }

    #[test]
    fn shifts_trailing_orphan_starters_and_preserves_min_gap_and_metrics() {
        let rules = SegmentRules::default();

        // 1. Bridged sentence-boundary orphan starter: ["Ne znam. Šta", "radimo ovde?"]
        let mut cues = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 2000,
                source_text: "I don't know. What".to_string(),
                translated_text: "Ne znam. Šta".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 6.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 2000,
                end_ms: 4000,
                source_text: "are we doing here?".to_string(),
                translated_text: "radimo ovde?".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 6.0,
                warnings: vec![],
            },
        ];

        shift_trailing_orphan_starters(&mut cues, &rules);
        assert_eq!(cues[0].translated_text, "Ne znam.");
        assert_eq!(cues[1].translated_text, "Šta radimo ovde?");
        assert!(cues[0].end_ms > cues[0].start_ms);
        assert!(cues[1].end_ms > cues[1].start_ms);
        assert!(
            cues[1].start_ms.saturating_sub(cues[0].end_ms) >= MIN_GAP_MS,
            "expected >= 80ms gap, got {}..{} -> {}..{}",
            cues[0].start_ms,
            cues[0].end_ms,
            cues[1].start_ms,
            cues[1].end_ms
        );
        // CPS must have been recomputed to match the shifted texts and updated durations
        let expected_cps_0 = ((8.0 / ((cues[0].end_ms - cues[0].start_ms) as f64 / 1000.0)) * 10.0)
            .round()
            / 10.0;
        let expected_cps_1 = ((16.0 / ((cues[1].end_ms - cues[1].start_ms) as f64 / 1000.0)) * 10.0)
            .round()
            / 10.0;
        assert_eq!(cues[0].cps, expected_cps_0);
        assert_eq!(cues[1].cps, expected_cps_1);

        // 2. Comma clause-boundary orphan starter shifts cleanly
        let mut comma_cues = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1800,
                source_text: "Okay, alright, what".to_string(),
                translated_text: "Dobro, u redu, šta".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 10.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 1950,
                end_ms: 3600,
                source_text: "do we do now?".to_string(),
                translated_text: "ćemo sada?".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 6.0,
                warnings: vec![],
            },
        ];
        shift_trailing_orphan_starters(&mut comma_cues, &rules);
        assert_eq!(comma_cues[0].translated_text, "Dobro, u redu,");
        assert_eq!(comma_cues[1].translated_text, "šta ćemo sada?");
        assert!(comma_cues[1].start_ms.saturating_sub(comma_cues[0].end_ms) >= MIN_GAP_MS);

        // 3. Complete closing vocative with terminal punctuation stays intact
        let mut vocative = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "Thanks, bro.".to_string(),
                translated_text: "Hvala ti, brate.".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 10.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 1600,
                end_ms: 3000,
                source_text: "See you tomorrow.".to_string(),
                translated_text: "Vidimo se sutra.".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 11.0,
                warnings: vec![],
            },
        ];
        shift_trailing_orphan_starters(&mut vocative, &rules);
        assert_eq!(vocative[0].translated_text, "Hvala ti, brate.");
        assert_eq!(vocative[1].translated_text, "Vidimo se sutra.");

        // 4. Abbreviations ("dr. Markom") and >800ms silence pauses stay untouched
        let mut abbrev_and_pause = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "With Dr. Marko".to_string(),
                translated_text: "Pričao sam sa dr. Markom".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 12.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 1600,
                end_ms: 3000,
                source_text: "yesterday. What".to_string(),
                translated_text: "juče. Šta".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 7.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 3,
                start_ms: 4200, // 1200ms silence > 800ms threshold
                end_ms: 6000,
                source_text: "happened next?".to_string(),
                translated_text: "je bilo dalje?".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 8.0,
                warnings: vec![],
            },
        ];
        shift_trailing_orphan_starters(&mut abbrev_and_pause, &rules);
        assert_eq!(abbrev_and_pause[0].translated_text, "Pričao sam sa dr. Markom");
        assert_eq!(abbrev_and_pause[1].translated_text, "juče. Šta");
        assert_eq!(abbrev_and_pause[2].translated_text, "je bilo dalje?");

        // 5. Does NOT shift when prepending would exceed max_chars_per_line * max_lines
        let reels_rules = SegmentRules {
            preset: crate::models::SegmentPreset::Reels,
            max_chars_per_line: 20,
            max_lines: 1,
            min_duration_ms: 600,
            max_duration_ms: 3500,
            max_cps: 18.0,
            split_on_speaker_change: true,
        };
        let mut full_next = vec![
            SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 1500,
                source_text: "Done. Tomorrow".to_string(),
                translated_text: "Gotovo. Sutra".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 9.0,
                warnings: vec![],
            },
            SubtitleCue {
                id: 2,
                start_ms: 1600,
                end_ms: 3200,
                source_text: "we start the project".to_string(),
                translated_text: "krećemo novi projekat".to_string(),
                translated_variants: BTreeMap::new(),
                cps: 13.0,
                warnings: vec![],
            },
        ];
        shift_trailing_orphan_starters(&mut full_next, &reels_rules);
        assert_eq!(full_next[0].translated_text, "Gotovo. Sutra");
        assert_eq!(full_next[1].translated_text, "krećemo novi projekat");
    }

    #[test]
    fn topic_context_injected_and_sanitized_in_translate_and_polish() {
        let glossary = BTreeMap::new();
        let noisy_topic = "  Tehnološki\npodkast\r\to\x07 veštačkoj inteligenciji   ";

        for (op, src, dst) in [
            (BatchOperation::Translate, "en", "sr"),
            (BatchOperation::PolishSerbian, "sr", "sr"),
        ] {
            let prompt = build_system_prompt_with_options(
                op,
                src,
                dst,
                &glossary,
                false,
                &PromptOptions {
                    formal_style: false,
                    remove_fillers: true,
                    topic_context: Some(noisy_topic),
                },
            );
            assert!(
                prompt.contains(
                    "AUDIO TOPIC & DOMAIN CONTEXT: Tehnološki podkast o veštačkoj inteligenciji"
                ),
                "expected sanitized topic context in {op:?} prompt:\n{prompt}"
            );
        }

        // CondenseCps does not inject AUDIO TOPIC & DOMAIN CONTEXT
        let condense_prompt = build_system_prompt_with_options(
            BatchOperation::CondenseCps,
            "en",
            "sr",
            &glossary,
            false,
            &PromptOptions {
                formal_style: false,
                remove_fillers: true,
                topic_context: Some("Medical lecture"),
            },
        );
        assert!(!condense_prompt.contains("AUDIO TOPIC & DOMAIN CONTEXT:"));

        // Long topic is capped at MAX_TOPIC_CONTEXT_CHARS (200) and whitespace-only is ignored
        let huge = "A".repeat(320);
        let sanitized = sanitize_topic_context(&huge).unwrap();
        assert_eq!(sanitized.chars().count(), MAX_TOPIC_CONTEXT_CHARS);
        assert!(sanitize_topic_context("   \n\t\r  ").is_none());
    }

    #[tokio::test]
    async fn classify_media_topic_parses_response_and_enforces_url_policy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured_formats = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let formats_clone = Arc::clone(&captured_formats);

        let server = tokio::spawn(async move {
            let responses = [
                "<think>Podcast style</think>\n```json\n{\"topic\": \"Tehnološki podkast o veštačkoj inteligenciji\", \"suggestedStyle\": \"natural\"}\n```",
                "{\"topic\": \"Istorijski dokumentarac o Nikoli Tesli\", \"suggestedStyle\": \"formal\"}",
            ];
            for content in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 16384];
                let mut total_read = 0usize;
                let mut header_end = None;
                let mut content_len = 0usize;
                loop {
                    let n = sock.read(&mut buf[total_read..]).await.unwrap_or(0);
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
                let req_str = String::from_utf8_lossy(&buf[..total_read]);
                let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
                let req_json: serde_json::Value =
                    serde_json::from_str(body_part).unwrap_or_default();
                formats_clone
                    .lock()
                    .unwrap()
                    .push(req_json["response_format"].clone());

                let resp_json = serde_json::json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": content
                        }
                    }]
                });
                let resp_body = serde_json::to_string(&resp_json).unwrap();
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = sock.write_all(http_resp.as_bytes()).await;
            }
        });

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 25,
        };
        let cancel = Arc::new(AtomicBool::new(false));

        let (topic_nat, style_nat) = classify_media_topic(
            &endpoint,
            "Hey guys, welcome back to the show, today we talk about local LLMs and GPUs!",
            Some("en"),
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(topic_nat, "Tehnološki podkast o veštačkoj inteligenciji");
        assert_eq!(style_nat, TranslationStyle::Natural);

        let (topic_for, style_for) = classify_media_topic(
            &endpoint,
            "In 1884, Nikola Tesla arrived in New York carrying a letter of recommendation.",
            Some("en"),
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(topic_for, "Istorijski dokumentarac o Nikoli Tesli");
        assert_eq!(style_for, TranslationStyle::Formal);

        server.await.unwrap();

        {
            let fmts = captured_formats.lock().unwrap();
            assert_eq!(fmts.len(), 2);
            assert_eq!(fmts[0]["type"], "json_schema");
            assert_eq!(fmts[0]["json_schema"]["strict"], true);
        }

        // Insecure remote HTTP endpoint must fail URL policy BEFORE making a network call
        let bad_ep = LlmEndpointConfig {
            base_url: "http://192.168.1.50:11434/v1".to_string(),
            model: "llama3".to_string(),
            api_key: Some("sk-secret123456".to_string()),
            batch_size: 25,
        };
        assert!(
            classify_media_topic(&bad_ep, "Test snippet", Some("en"), &cancel)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn dual_style_cache_scopes_resume_preserves_both_variants_and_syncs_condense() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let http_calls = Arc::new(AtomicUsize::new(0));
        let http_calls_clone = Arc::clone(&http_calls);

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                http_calls_clone.fetch_add(1, Ordering::SeqCst);
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

                    let req_str = String::from_utf8_lossy(&buf[..total_read]);
                    let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("{}");
                    let req_json: serde_json::Value =
                        serde_json::from_str(body_part).unwrap_or_default();
                    let sys_prompt = req_json["messages"][0]["content"].as_str().unwrap_or("");
                    let user_content = req_json["messages"][1]["content"].as_str().unwrap_or("{}");
                    let payload: serde_json::Value =
                        serde_json::from_str(user_content).unwrap_or_default();
                    let targets = payload["translate_cues"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();

                    let is_condense = sys_prompt.contains("tightening verbose subtitles");
                    let is_formal = sys_prompt.contains("standardni književni stil");

                    let mut map_out = serde_json::Map::new();
                    for t in &targets {
                        let id = t["id"].as_u64().unwrap_or(1);
                        let text = if is_condense {
                            "Sažet formalni titl.".to_string()
                        } else if is_formal {
                            if id == 1 {
                                "Poštovani gledaoci, ovo je veoma opširan formalan uvod u današnju temu."
                                    .to_string()
                            } else {
                                "Srdačan pozdrav iz Beograda.".to_string()
                            }
                        } else if id == 1 {
                            "Ćao svima, idemo dalje.".to_string()
                        } else {
                            "Pozdrav iz Beograda.".to_string()
                        };
                        map_out.insert(id.to_string(), serde_json::Value::String(text));
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
        project.cues.truncate(2);
        project.cues[0].start_ms = 0;
        project.cues[0].end_ms = 1900;
        project.cues[1].start_ms = 2000;
        project.cues[1].end_ms = 4000;

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 2,
        };
        let cancel = Arc::new(AtomicBool::new(false));

        // 1. Translate in Natural style
        project.options.translation_style = TranslationStyle::Natural;
        translate_cues(&mut project, &endpoint, &cancel, |_| Ok(()), |_, _| {})
            .await
            .unwrap();
        assert_eq!(http_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            project.completed_translation_style,
            Some(TranslationStyle::Natural)
        );
        assert_eq!(project.cues[0].translated_text, "Ćao svima, idemo dalje.");
        assert_eq!(
            project.cues[0]
                .translated_variants
                .get(&TranslationStyle::Natural)
                .map(String::as_str),
            Some("Ćao svima, idemo dalje.")
        );

        // 2. Re-running Translate in same Natural style skips completed batch (0 new HTTP calls)
        translate_cues(&mut project, &endpoint, &cancel, |_| Ok(()), |_, _| {})
            .await
            .unwrap();
        assert_eq!(http_calls.load(Ordering::SeqCst), 1);

        // 3. Switching to Formal style resets resume and translates into Formal while keeping Natural!
        project.options.translation_style = TranslationStyle::Formal;
        translate_cues(&mut project, &endpoint, &cancel, |_| Ok(()), |_, _| {})
            .await
            .unwrap();
        assert_eq!(http_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            project.completed_translation_style,
            Some(TranslationStyle::Formal)
        );
        assert_eq!(
            project.cues[0]
                .translated_variants
                .get(&TranslationStyle::Natural)
                .map(String::as_str),
            Some("Ćao svima, idemo dalje.")
        );
        assert!(project.cues[0]
            .translated_variants
            .get(&TranslationStyle::Formal)
            .is_some_and(|s| s.contains("Poštovani gledaoci")));
        assert_eq!(
            project.cues[1]
                .translated_variants
                .get(&TranslationStyle::Formal)
                .map(String::as_str),
            Some("Srdačan pozdrav iz Beograda.")
        );

        // 4. CondenseCps in Formal style updates Formal variant + translated_text without touching Natural!
        condense_cps_cues(&mut project, &endpoint, &cancel, |_| Ok(()), |_, _| {})
            .await
            .unwrap();
        assert_eq!(http_calls.load(Ordering::SeqCst), 3);
        assert_eq!(project.cues[0].translated_text, "Sažet formalni titl.");
        assert_eq!(
            project.cues[0]
                .translated_variants
                .get(&TranslationStyle::Formal)
                .map(String::as_str),
            Some("Sažet formalni titl.")
        );
        assert_eq!(
            project.cues[0]
                .translated_variants
                .get(&TranslationStyle::Natural)
                .map(String::as_str),
            Some("Ćao svima, idemo dalje.")
        );

        server.abort();
    }
}
