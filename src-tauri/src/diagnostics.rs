//! Diagnostics collection, rolling file logging (`<data_dir>/logs/app.log`),
//! secret/PII sanitization, Rust panic hook, and bug/crash report submission.

use crate::hardware;
use crate::models::{
    AppConfig, BugReportRequest, CatalogManifest, CrashReportRequest, CustomModelEntry,
    DiagnosticsPreview, GpuInfo, ReportSubmitResult, StoragePaths,
};
use crate::{setup_manager, storage};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use tauri::AppHandle;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

/// Compile-time default telemetry worker base URL (`easysrt-telemetry` Cloudflare Worker).
/// Can be overridden at runtime via `EASYSRT_TELEMETRY_URL` (set to `""` or `"off"` to disable
/// remote uploads and fall back to saving sanitized reports locally).
pub const DEFAULT_TELEMETRY_URL: &str = "https://easysrt-telemetry.ig-systems.rs";

pub const MAX_LOG_FILE_BYTES: u64 = 2 * 1024 * 1024;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = concat!("EasySRT-Studio/", env!("CARGO_PKG_VERSION"));
const MAX_DIAGNOSTICS_BYTES: usize = 32 * 1024;
const MAX_STACK_BYTES: usize = 16 * 1024;
const MAX_SUBPROCESS_REPORTS_PER_SESSION: usize = 10;

static APP_DATA_DIR: OnceLock<RwLock<PathBuf>> = OnceLock::new();
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
static PANIC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static SUBPROCESS_REPORT_DEDUPE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerBugReportPayload {
    pub app_version: String,
    pub title: String,
    pub description: String,
    pub steps: Option<String>,
    pub contact: Option<String>,
    pub platform: Option<String>,
    pub gpu: Option<String>,
    pub diagnostics: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCrashReportPayload {
    pub app_version: String,
    pub crash_kind: String,
    pub fingerprint: String,
    pub message: String,
    pub stack_trace: Option<String>,
    pub source: Option<String>,
    pub platform: Option<String>,
    pub gpu: Option<String>,
    pub diagnostics: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerReportResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    deduplicated: bool,
    #[serde(default)]
    issue_number: Option<u64>,
    #[serde(default)]
    issue_url: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn normalize_endpoint_override(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("disabled")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Pure testable telemetry endpoint resolver:
/// 1. Runtime env `EASYSRT_TELEMETRY_URL` (if set, even empty/`off` to explicitly disable)
/// 2. Compile-time `option_env!("EASYSRT_TELEMETRY_URL")`
/// 3. `default_url` (`DEFAULT_TELEMETRY_URL`)
pub fn resolve_telemetry_endpoint_from(
    runtime_env: Option<&str>,
    compile_env: Option<&str>,
    default_url: &str,
) -> Option<String> {
    if let Some(val) = runtime_env {
        return normalize_endpoint_override(val);
    }
    if let Some(val) = compile_env {
        return normalize_endpoint_override(val);
    }
    normalize_endpoint_override(default_url)
}

/// Resolve telemetry worker base URL from:
/// 1. Runtime env `EASYSRT_TELEMETRY_URL`
/// 2. Compile-time `option_env!("EASYSRT_TELEMETRY_URL")`
/// 3. `DEFAULT_TELEMETRY_URL`
///
/// Returns `None` when explicitly disabled or unconfigured.
pub fn resolve_telemetry_endpoint() -> Option<String> {
    let runtime_env = std::env::var("EASYSRT_TELEMETRY_URL").ok();
    resolve_telemetry_endpoint_from(
        runtime_env.as_deref(),
        option_env!("EASYSRT_TELEMETRY_URL"),
        DEFAULT_TELEMETRY_URL,
    )
}

/// Record the active application `data_dir` so panic hooks and subprocess reporters
/// write logs/crash files into the exact directory used by the running app.
pub fn set_app_data_dir(data_dir: PathBuf) {
    let lock = APP_DATA_DIR.get_or_init(|| RwLock::new(data_dir.clone()));
    if let Ok(mut guard) = lock.write() {
        *guard = data_dir;
    }
}

fn current_app_data_dir() -> PathBuf {
    if let Some(lock) = APP_DATA_DIR.get() {
        if let Ok(guard) = lock.read() {
            return guard.clone();
        }
    }
    storage::default_installed_root_dir().join("data")
}

fn current_config_path_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .parent()
        .map(|root| root.join("config.json"))
        .unwrap_or_else(|| storage::default_installed_root_dir().join("config.json"))
}

// ---------------------------------------------------------------------------
// Rolling Log File Writer (<data_dir>/logs/app.log -> app.log.1 at > 2 MB)
// ---------------------------------------------------------------------------

struct RotatingFileSink {
    log_path: PathBuf,
    rotated_path: PathBuf,
    file: Option<File>,
    written_bytes: u64,
    max_bytes: u64,
}

impl RotatingFileSink {
    fn new(log_dir: &Path, max_bytes: u64) -> Self {
        let _ = fs::create_dir_all(log_dir);
        let log_path = log_dir.join("app.log");
        let rotated_path = log_dir.join("app.log.1");

        let mut existing_len = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        if existing_len > max_bytes {
            let _ = fs::remove_file(&rotated_path);
            let _ = fs::rename(&log_path, &rotated_path);
            existing_len = 0;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();

        Self {
            log_path,
            rotated_path,
            file,
            written_bytes: existing_len,
            max_bytes,
        }
    }

    fn rotate(&mut self) {
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
        let _ = fs::remove_file(&self.rotated_path);
        let _ = fs::rename(&self.log_path, &self.rotated_path);
        self.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.log_path)
            .ok();
        self.written_bytes = 0;
    }

    fn write_bytes(&mut self, buf: &[u8]) {
        if self.written_bytes.saturating_add(buf.len() as u64) > self.max_bytes
            && self.written_bytes > 0
        {
            self.rotate();
        }
        if self.file.is_none() {
            if let Some(parent) = self.log_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
                .ok();
        }
        if let Some(ref mut f) = self.file {
            if f.write_all(buf).is_ok() {
                self.written_bytes = self.written_bytes.saturating_add(buf.len() as u64);
            }
        }
    }

    fn flush_sink(&mut self) {
        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
    }
}

#[derive(Clone)]
struct TeeMakeWriter {
    sink: Arc<Mutex<RotatingFileSink>>,
}

struct TeeWriter {
    sink: Arc<Mutex<RotatingFileSink>>,
}

impl<'a> MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            sink: Arc::clone(&self.sink),
        }
    }
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        let mut sink = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        sink.write_bytes(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        let mut sink = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        sink.flush_sink();
        Ok(())
    }
}

/// Initialize global `tracing_subscriber` writing to both stderr and `<data_dir>/logs/app.log`
/// with automatic 2 MB rotation to `app.log.1`.
pub fn init_logging(data_dir: &Path) {
    set_app_data_dir(data_dir.to_path_buf());
    let logs_dir = data_dir.join("logs");
    let sink = Arc::new(Mutex::new(RotatingFileSink::new(
        &logs_dir,
        MAX_LOG_FILE_BYTES,
    )));
    let make_writer = TeeMakeWriter { sink };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(make_writer)
        .try_init();
}

// ---------------------------------------------------------------------------
// Sanitization & Crash Fingerprinting
// ---------------------------------------------------------------------------

struct SanitizerRegexes {
    url_userinfo: Regex,
    sk_key: Regex,
    hf_token: Regex,
    gemini_key: Regex,
    gh_token: Regex,
    github_pat: Regex,
    bearer: Regex,
    api_key_kv: Regex,
    obf_cred: Regex,
    win_user_escaped: Regex,
    win_user_path: Regex,
    linux_home_path: Regex,
    macos_users_path: Regex,
}

fn sanitizer_regexes() -> &'static SanitizerRegexes {
    static REGEXES: OnceLock<SanitizerRegexes> = OnceLock::new();
    REGEXES.get_or_init(|| SanitizerRegexes {
        url_userinfo: Regex::new(
            r#"(?i)\b([A-Za-z][A-Za-z0-9+.\-]{0,15}://)[^/\s\\?#"'<>]{1,512}@([A-Za-z0-9\[])"#,
        )
        .expect("valid url_userinfo regex"),
        sk_key: Regex::new(r"\bsk-[A-Za-z0-9_\-]{8,}").expect("valid sk_key regex"),
        hf_token: Regex::new(r"\bhf_[A-Za-z0-9]{10,}").expect("valid hf_token regex"),
        gemini_key: Regex::new(r"\bAIza[0-9A-Za-z_\-]{20,}").expect("valid gemini_key regex"),
        gh_token: Regex::new(r"\bgh[pousr]_[A-Za-z0-9]+").expect("valid gh_token regex"),
        github_pat: Regex::new(r"\bgithub_pat_[A-Za-z0-9_]+").expect("valid github_pat regex"),
        bearer: Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._\-]+").expect("valid bearer regex"),
        api_key_kv: Regex::new(
            r#"(?i)(\b(?:x[_-])?api[_-]?key["']?\s*[:=]\s*["']?)(?:\[REDACTED(?:_[A-Z_]+)?\]|[^\s"',}&;\]\)]+)"#,
        )
        .expect("valid api_key_kv regex"),
        obf_cred: Regex::new(r"obf1:[0-9a-fA-F]{8,}").expect("valid obf_cred regex"),
        win_user_escaped: Regex::new(r#"(?i)([A-Za-z]):\\\\Users\\\\[^\\/\r\n\t"']+\\\\"#)
            .expect("valid win_user_escaped regex"),
        win_user_path: Regex::new(r#"(?i)([A-Za-z]):[\\/]Users[\\/][^\\/\r\n\t"']+[\\/]"#)
            .expect("valid win_user_path regex"),
        linux_home_path: Regex::new(r#"/home/[^/\s"'<>:;,\)\]]+/"#)
            .expect("valid linux_home_path regex"),
        macos_users_path: Regex::new(r#"(?P<prefix>^|[^A-Za-z0-9_:])/Users/[^\\/\s"'<>:;,\)\]]+"#)
            .expect("valid macos_users_path regex"),
    })
}

/// Redact API keys, auth tokens, URL credentials, and OS usernames from diagnostic text
/// before saving or uploading.
pub fn sanitize_diagnostic_text(raw: &str) -> String {
    let rx = sanitizer_regexes();
    let s = rx.url_userinfo.replace_all(raw, "${1}<redacted>@${2}");
    let s = rx.sk_key.replace_all(&s, "[REDACTED_API_KEY]");
    let s = rx.hf_token.replace_all(&s, "[REDACTED_HF_TOKEN]");
    let s = rx.gemini_key.replace_all(&s, "[REDACTED_API_KEY]");
    let s = rx.gh_token.replace_all(&s, "[REDACTED_TOKEN]");
    let s = rx.github_pat.replace_all(&s, "[REDACTED_TOKEN]");
    let s = rx.bearer.replace_all(&s, "Bearer [REDACTED]");
    let s = rx.api_key_kv.replace_all(&s, "${1}[REDACTED]");
    let s = rx.obf_cred.replace_all(&s, "[REDACTED_CREDENTIAL]");
    let s = rx
        .win_user_escaped
        .replace_all(&s, r"${1}:\\Users\\<USER>\\");
    let s = rx.win_user_path.replace_all(&s, r"${1}:\Users\<USER>\");
    let s = rx.linux_home_path.replace_all(&s, "/home/<USER>/");
    let s = rx
        .macos_users_path
        .replace_all(&s, "${prefix}/Users/<USER>");
    s.into_owned()
}

/// Alias for [`sanitize_diagnostic_text`] for callers using `diagnostics::sanitize_text`.
pub fn sanitize_text(raw: &str) -> String {
    sanitize_diagnostic_text(raw)
}

/// Compute a deterministic 16-character lowercase hex SHA-256 fingerprint for crash deduplication.
pub fn compute_crash_fingerprint(
    crash_kind: &str,
    app_version: &str,
    message: &str,
    stack: &str,
) -> String {
    let clean_kind = crash_kind.trim().to_ascii_lowercase();
    let clean_ver = app_version.trim();
    let clean_msg = sanitize_diagnostic_text(message);
    let first_msg_line = clean_msg.lines().next().unwrap_or("").trim();
    let clean_stack = sanitize_diagnostic_text(stack);
    let stack_head: String = clean_stack
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");

    let mut hasher = Sha256::new();
    hasher.update(clean_kind.as_bytes());
    hasher.update(b"\n");
    hasher.update(clean_ver.as_bytes());
    hasher.update(b"\n");
    hasher.update(first_msg_line.as_bytes());
    hasher.update(b"\n");
    hasher.update(stack_head.as_bytes());

    let digest = hex::encode(hasher.finalize());
    digest[..16].to_string()
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        raw.to_string()
    } else {
        raw.chars().take(max_chars).collect()
    }
}

fn truncate_utf8_bytes(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].to_string()
}

// ---------------------------------------------------------------------------
// Diagnostics Collection (Never reads subtitle checkpoints or .credentials.json)
// ---------------------------------------------------------------------------

fn read_tail_lines(path: &Path, max_lines: usize) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = raw.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    let joined = lines[start..].join("\n").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Read and sanitize the last ~150 lines from `<data_dir>/logs/app.log` and
/// `<data_dir>/setup-bootstrap.log`. Never touches `checkpoints/` or `.credentials.json`.
pub fn collect_sanitized_log_excerpt(data_dir: &Path) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(app_tail) = read_tail_lines(&data_dir.join("logs").join("app.log"), 120) {
        sections.push(format!("=== app.log ===\n{app_tail}"));
    }

    if let Some(setup_tail) = read_tail_lines(&data_dir.join("setup-bootstrap.log"), 30) {
        sections.push(format!("=== setup-bootstrap.log ===\n{setup_tail}"));
    }

    if sections.is_empty() {
        return "(no log entries yet)".to_string();
    }

    let sanitized = sanitize_diagnostic_text(&sections.join("\n\n"));
    truncate_utf8_bytes(&sanitized, 24 * 1024)
}

fn detect_os_info() -> String {
    let base = format!(
        "{} / {} ({})",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::FAMILY
    );

    #[cfg(target_os = "linux")]
    if let Ok(os_release) = fs::read_to_string("/etc/os-release") {
        for line in os_release.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                let pretty = val.trim().trim_matches('"').trim();
                if !pretty.is_empty() {
                    return sanitize_diagnostic_text(&format!("{base} — {pretty}"));
                }
            }
        }
    }

    base
}

fn format_gpu_summary(gpu: &GpuInfo) -> String {
    if !gpu.has_nvidia {
        return "No NVIDIA GPU detected (CPU mode)".to_string();
    }

    let name = gpu.gpu_name.as_deref().unwrap_or("NVIDIA GPU");
    let driver = gpu.driver_version.as_deref().unwrap_or("unknown");
    let cuda_state = if gpu.cuda_supported {
        "supported"
    } else {
        "unsupported"
    };
    let note = gpu
        .cuda_status_note
        .as_deref()
        .map(|n| format!(" — {n}"))
        .unwrap_or_default();

    sanitize_diagnostic_text(&format!(
        "{name} ({} MB VRAM, driver {driver}, CUDA 12.4: {cuda_state}, tier {:?}{note})",
        gpu.vram_mb, gpu.recommended_tier
    ))
}

/// Pure/testable diagnostics preview builder from explicit paths, catalog, GPU info, and platform.
pub fn collect_diagnostics_preview_for_paths(
    paths: &StoragePaths,
    catalog: &CatalogManifest,
    gpu: &GpuInfo,
    custom_models: &[CustomModelEntry],
    platform: &str,
) -> DiagnosticsPreview {
    let status = setup_manager::compute_setup_status(
        catalog,
        gpu.clone(),
        paths.clone(),
        custom_models,
        platform,
    );

    let mut installed_components: Vec<String> = status
        .components
        .iter()
        .filter(|c| c.installed)
        .map(|c| {
            let mb = c.size_bytes as f64 / (1024.0 * 1024.0);
            format!("{} ({:.1} MB)", c.id, mb)
        })
        .collect();

    if !custom_models.is_empty() {
        installed_components.push(format!("custom-models ({} imported)", custom_models.len()));
    }

    DiagnosticsPreview {
        app_version: APP_VERSION.to_string(),
        platform: platform.to_string(),
        os_info: detect_os_info(),
        gpu_summary: format_gpu_summary(gpu),
        installed_components,
        log_excerpt: collect_sanitized_log_excerpt(Path::new(&paths.data_dir)),
        telemetry_endpoint: resolve_telemetry_endpoint(),
    }
}

fn format_diagnostics_markdown(preview: &DiagnosticsPreview) -> String {
    let components = if preview.installed_components.is_empty() {
        "- (none)".to_string()
    } else {
        preview
            .installed_components
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let md = format!(
        "## System Diagnostics\n\
         - **App Version**: {}\n\
         - **Platform**: {}\n\
         - **OS**: {}\n\
         - **GPU**: {}\n\n\
         ### Installed Components\n{}\n\n\
         ### Recent Logs\n```text\n{}\n```\n",
        preview.app_version,
        preview.platform,
        preview.os_info,
        preview.gpu_summary,
        components,
        preview.log_excerpt,
    );

    let sanitized = sanitize_diagnostic_text(&md);
    truncate_utf8_bytes(&sanitized, MAX_DIAGNOSTICS_BYTES)
}

fn collect_diagnostics_preview_sync(app: &AppHandle) -> Result<(StoragePaths, AppConfig, DiagnosticsPreview)> {
    let config = storage::load_config(app)?;
    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    set_app_data_dir(PathBuf::from(&paths.data_dir));
    let catalog = setup_manager::load_catalog(app)?;
    let gpu = hardware::detect_gpu();
    let preview = collect_diagnostics_preview_for_paths(
        &paths,
        &catalog,
        &gpu,
        &config.custom_models,
        setup_manager::current_platform_tag(),
    );
    Ok((paths, config, preview))
}

/// Collect a sanitized diagnostics snapshot for preview in the Bug Report modal.
pub async fn collect_diagnostics_preview(app: &AppHandle) -> Result<DiagnosticsPreview> {
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let (_paths, _config, preview) = collect_diagnostics_preview_sync(&app_clone)?;
        Ok(preview)
    })
    .await
    .context("Diagnostics preview task failed")?
}

// ---------------------------------------------------------------------------
// Local File Persistence Helpers
// ---------------------------------------------------------------------------

fn unique_report_path(dir: &Path, prefix: &str, ext: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create diagnostics directory: {}", dir.display()))?;
    let ts = Utc::now().format("%Y%m%d-%H%M%S");
    let candidate = dir.join(format!("{prefix}-{ts}.{ext}"));
    if !candidate.exists() {
        return Ok(candidate);
    }
    let short_id = &Uuid::new_v4().to_string()[..8];
    Ok(dir.join(format!("{prefix}-{ts}-{short_id}.{ext}")))
}

/// Save a sanitized Markdown diagnostics bundle to `<data_dir>/reports/diagnostics-<ts>.md`.
pub fn save_diagnostics_local_to_data_dir(
    data_dir: &Path,
    preview: &DiagnosticsPreview,
) -> Result<String> {
    let reports_dir = data_dir.join("reports");
    let out_path = unique_report_path(&reports_dir, "diagnostics", "md")?;
    let header = format!(
        "# EasySRT Studio Diagnostics Report\n\
         Generated at: {}\n\n",
        Utc::now().to_rfc3339()
    );
    let body = format!("{header}{}", format_diagnostics_markdown(preview));
    fs::write(&out_path, sanitize_diagnostic_text(&body)).with_context(|| {
        format!(
            "Failed to write local diagnostics report: {}",
            out_path.display()
        )
    })?;
    Ok(out_path.to_string_lossy().into_owned())
}

/// Save full sanitized Markdown diagnostics locally and return the written path.
pub fn save_diagnostics_local(app: &AppHandle) -> Result<String> {
    let (paths, _config, preview) = collect_diagnostics_preview_sync(app)?;
    save_diagnostics_local_to_data_dir(Path::new(&paths.data_dir), &preview)
}

fn save_bug_report_local(data_dir: &Path, payload: &WorkerBugReportPayload) -> Result<String> {
    let reports_dir = data_dir.join("reports");
    let out_path = unique_report_path(&reports_dir, "bug-report", "md")?;

    let mut md = format!(
        "# EasySRT Studio Bug Report (v{})\n\
         Generated at: {}\n\n\
         ## Title\n{}\n\n\
         ## Description\n{}\n",
        payload.app_version,
        Utc::now().to_rfc3339(),
        payload.title,
        payload.description,
    );

    if let Some(ref steps) = payload.steps {
        md.push_str(&format!("\n## Steps to Reproduce\n{steps}\n"));
    }
    if let Some(ref contact) = payload.contact {
        md.push_str(&format!("\n## Contact\n{contact}\n"));
    }
    if let Some(ref platform) = payload.platform {
        md.push_str(&format!("\n- **Platform**: {platform}\n"));
    }
    if let Some(ref gpu) = payload.gpu {
        md.push_str(&format!("- **GPU**: {gpu}\n"));
    }
    if let Some(ref diag) = payload.diagnostics {
        md.push_str(&format!("\n{diag}\n"));
    }

    fs::write(&out_path, sanitize_diagnostic_text(&md)).with_context(|| {
        format!(
            "Failed to save local bug report: {}",
            out_path.display()
        )
    })?;
    Ok(out_path.to_string_lossy().into_owned())
}

fn write_crash_report_json(
    data_dir: &Path,
    prefix: &str,
    payload: &WorkerCrashReportPayload,
) -> Result<PathBuf> {
    let crashes_dir = data_dir.join("crashes");
    fs::create_dir_all(&crashes_dir).with_context(|| {
        format!(
            "Failed to create crashes directory: {}",
            crashes_dir.display()
        )
    })?;
    let file_path = crashes_dir.join(format!("{prefix}-{}.json", Uuid::new_v4()));
    let json_bytes =
        serde_json::to_vec_pretty(payload).context("Failed to serialize crash report JSON")?;
    fs::write(&file_path, json_bytes).with_context(|| {
        format!(
            "Failed to write crash report file: {}",
            file_path.display()
        )
    })?;
    Ok(file_path)
}

fn move_crash_file_to_sent(data_dir: &Path, src_path: &Path) -> PathBuf {
    let sent_dir = data_dir.join("crashes").join("sent");
    let _ = fs::create_dir_all(&sent_dir);
    let file_name = src_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| format!("sent-{}.json", Uuid::new_v4()).into());
    let dst_path = sent_dir.join(file_name);

    if fs::rename(src_path, &dst_path).is_ok() {
        return dst_path;
    }
    if fs::copy(src_path, &dst_path).is_ok() {
        let _ = fs::remove_file(src_path);
        return dst_path;
    }
    src_path.to_path_buf()
}

// ---------------------------------------------------------------------------
// Worker HTTP Transport
// ---------------------------------------------------------------------------

async fn post_report_json<T: Serialize>(
    url: &str,
    payload: &T,
    timeout: Duration,
) -> std::result::Result<WorkerReportResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("Failed to initialize HTTP client: {e}"))?;

    let resp = client
        .post(url)
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("Network error sending report: {e}"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    if !status.is_success() {
        let detail = serde_json::from_str::<WorkerReportResponse>(&body_text)
            .ok()
            .and_then(|r| r.error)
            .unwrap_or_else(|| truncate_chars(body_text.trim(), 240));
        return Err(if detail.is_empty() {
            format!("Telemetry endpoint returned HTTP {status}")
        } else {
            format!("Telemetry endpoint returned HTTP {status}: {detail}")
        });
    }

    let parsed: WorkerReportResponse = serde_json::from_str(&body_text)
        .map_err(|e| format!("Invalid JSON response from telemetry endpoint: {e}"))?;
    if !parsed.ok {
        return Err(parsed
            .error
            .unwrap_or_else(|| "Telemetry endpoint reported ok=false".to_string()));
    }

    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Public Report Submission Commands
// ---------------------------------------------------------------------------

/// Build a sanitized bug report, submit it to `${endpoint}/v1/report/bug` when configured,
/// and gracefully fall back to saving `<data_dir>/reports/bug-report-<ts>.md` locally when
/// offline or when the telemetry endpoint is unconfigured.
pub async fn submit_bug_report(
    app: &AppHandle,
    req: BugReportRequest,
) -> Result<ReportSubmitResult> {
    let app_clone = app.clone();
    let (paths, _config, preview) =
        tokio::task::spawn_blocking(move || collect_diagnostics_preview_sync(&app_clone))
            .await
            .context("Failed collecting diagnostics for bug report")??;

    let clean_title = truncate_chars(sanitize_diagnostic_text(&req.title).trim(), 180);
    let clean_description = truncate_chars(sanitize_diagnostic_text(&req.description).trim(), 8000);
    if clean_title.is_empty() {
        return Err(anyhow!("Bug report title cannot be empty"));
    }
    if clean_description.is_empty() {
        return Err(anyhow!("Bug report description cannot be empty"));
    }

    let clean_steps = req
        .steps
        .as_deref()
        .map(|s| truncate_chars(sanitize_diagnostic_text(s).trim(), 4000))
        .filter(|s| !s.is_empty());
    let clean_contact = req
        .contact
        .as_deref()
        .map(|s| truncate_chars(sanitize_diagnostic_text(s).trim(), 200))
        .filter(|s| !s.is_empty());

    let diagnostics = if req.include_diagnostics {
        Some(format_diagnostics_markdown(&preview))
    } else {
        None
    };

    let payload = WorkerBugReportPayload {
        app_version: APP_VERSION.to_string(),
        title: clean_title,
        description: clean_description,
        steps: clean_steps,
        contact: clean_contact,
        platform: Some(preview.platform.clone()),
        gpu: Some(preview.gpu_summary.clone()),
        diagnostics,
    };

    let data_dir = PathBuf::from(&paths.data_dir);

    if let Some(endpoint) = resolve_telemetry_endpoint() {
        let url = format!("{endpoint}/v1/report/bug");
        match post_report_json(&url, &payload, Duration::from_secs(15)).await {
            Ok(resp) => {
                return Ok(ReportSubmitResult {
                    sent: true,
                    deduplicated: resp.deduplicated,
                    issue_number: resp.issue_number,
                    issue_url: resp.issue_url,
                    saved_local_path: None,
                    error: None,
                });
            }
            Err(err_msg) => {
                tracing::warn!("Bug report upload failed ({err_msg}); saving locally instead");
                let local_path = save_bug_report_local(&data_dir, &payload)?;
                return Ok(ReportSubmitResult {
                    sent: false,
                    deduplicated: false,
                    issue_number: None,
                    issue_url: None,
                    saved_local_path: Some(local_path),
                    error: Some(sanitize_diagnostic_text(&err_msg)),
                });
            }
        }
    }

    let local_path = save_bug_report_local(&data_dir, &payload)?;
    Ok(ReportSubmitResult {
        sent: false,
        deduplicated: false,
        issue_number: None,
        issue_url: None,
        saved_local_path: Some(local_path),
        error: None,
    })
}

fn build_crash_payload(
    req: &CrashReportRequest,
    platform: Option<String>,
    gpu: Option<String>,
    diagnostics: Option<String>,
) -> WorkerCrashReportPayload {
    let clean_kind = {
        let k = sanitize_diagnostic_text(&req.crash_kind);
        let t = k.trim();
        if t.is_empty() {
            "unknown_crash".to_string()
        } else {
            truncate_chars(t, 64)
        }
    };

    let clean_msg = {
        let m = sanitize_diagnostic_text(&req.message);
        let t = m.trim();
        if t.is_empty() {
            "Unknown crash".to_string()
        } else {
            truncate_chars(t, 2000)
        }
    };

    let clean_stack = req
        .stack_trace
        .as_deref()
        .map(|s| truncate_utf8_bytes(sanitize_diagnostic_text(s).trim(), MAX_STACK_BYTES))
        .filter(|s| !s.is_empty());

    let clean_source = req
        .source
        .as_deref()
        .map(|s| truncate_chars(sanitize_diagnostic_text(s).trim(), 240))
        .filter(|s| !s.is_empty());

    let fingerprint = compute_crash_fingerprint(
        &clean_kind,
        APP_VERSION,
        &clean_msg,
        clean_stack.as_deref().unwrap_or(""),
    );

    WorkerCrashReportPayload {
        app_version: APP_VERSION.to_string(),
        crash_kind: clean_kind,
        fingerprint,
        message: clean_msg,
        stack_trace: clean_stack,
        source: clean_source,
        platform,
        gpu,
        diagnostics: diagnostics.map(|d| truncate_utf8_bytes(&sanitize_diagnostic_text(&d), MAX_DIAGNOSTICS_BYTES)),
    }
}

/// Persist a sanitized crash report under `<data_dir>/crashes/`, and automatically submit it
/// to `${endpoint}/v1/report/crash` when `config.auto_send_crash_reports` is enabled and the
/// telemetry endpoint is configured.
pub async fn submit_crash_report(
    app: &AppHandle,
    req: CrashReportRequest,
) -> Result<ReportSubmitResult> {
    let app_clone = app.clone();
    let (paths, config, preview) =
        tokio::task::spawn_blocking(move || collect_diagnostics_preview_sync(&app_clone))
            .await
            .context("Failed collecting diagnostics for crash report")??;

    let payload = build_crash_payload(
        &req,
        Some(preview.platform.clone()),
        Some(preview.gpu_summary.clone()),
        Some(format_diagnostics_markdown(&preview)),
    );

    let data_dir = PathBuf::from(&paths.data_dir);
    let prefix = if config.auto_send_crash_reports {
        "pending"
    } else {
        "local"
    };
    let saved_path = write_crash_report_json(&data_dir, prefix, &payload)?;

    if config.auto_send_crash_reports {
        if let Some(endpoint) = resolve_telemetry_endpoint() {
            let url = format!("{endpoint}/v1/report/crash");
            match post_report_json(&url, &payload, Duration::from_secs(10)).await {
                Ok(resp) => {
                    let sent_path = move_crash_file_to_sent(&data_dir, &saved_path);
                    return Ok(ReportSubmitResult {
                        sent: true,
                        deduplicated: resp.deduplicated,
                        issue_number: resp.issue_number,
                        issue_url: resp.issue_url,
                        saved_local_path: Some(sent_path.to_string_lossy().into_owned()),
                        error: None,
                    });
                }
                Err(err_msg) => {
                    tracing::warn!("Crash report upload failed ({err_msg}); kept pending on disk");
                    return Ok(ReportSubmitResult {
                        sent: false,
                        deduplicated: false,
                        issue_number: None,
                        issue_url: None,
                        saved_local_path: Some(saved_path.to_string_lossy().into_owned()),
                        error: Some(sanitize_diagnostic_text(&err_msg)),
                    });
                }
            }
        }
    }

    Ok(ReportSubmitResult {
        sent: false,
        deduplicated: false,
        issue_number: None,
        issue_url: None,
        saved_local_path: Some(saved_path.to_string_lossy().into_owned()),
        error: None,
    })
}

fn sanitize_crash_payload(mut payload: WorkerCrashReportPayload) -> WorkerCrashReportPayload {
    payload.crash_kind = sanitize_diagnostic_text(&payload.crash_kind);
    payload.message = truncate_chars(&sanitize_diagnostic_text(&payload.message), 2000);
    payload.stack_trace = payload
        .stack_trace
        .map(|s| truncate_utf8_bytes(&sanitize_diagnostic_text(&s), MAX_STACK_BYTES));
    payload.source = payload
        .source
        .map(|s| truncate_chars(&sanitize_diagnostic_text(&s), 240));
    payload.gpu = payload.gpu.map(|g| sanitize_diagnostic_text(&g));
    payload.diagnostics = payload
        .diagnostics
        .map(|d| truncate_utf8_bytes(&sanitize_diagnostic_text(&d), MAX_DIAGNOSTICS_BYTES));
    payload
}

/// Flush any `crashes/pending-*.json` reports left from previous runs when
/// `config.auto_send_crash_reports` is true and a telemetry endpoint is configured.
pub async fn flush_pending_crash_reports(app: &AppHandle) -> Result<u32> {
    let config = storage::load_config(app)?;
    if !config.auto_send_crash_reports {
        return Ok(0);
    }
    let Some(endpoint) = resolve_telemetry_endpoint() else {
        return Ok(0);
    };

    let paths = storage::resolve_storage_paths(app, config.models_dir_override.as_deref())?;
    set_app_data_dir(PathBuf::from(&paths.data_dir));
    let data_dir = PathBuf::from(&paths.data_dir);
    let crashes_dir = data_dir.join("crashes");
    if !crashes_dir.is_dir() {
        return Ok(0);
    }

    let mut pending_files: Vec<PathBuf> = fs::read_dir(&crashes_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("pending-") && n.ends_with(".json"))
                    .unwrap_or(false)
        })
        .collect();
    pending_files.sort();
    pending_files.truncate(20);

    let url = format!("{endpoint}/v1/report/crash");
    let mut sent_count = 0u32;

    for file_path in pending_files {
        let Ok(raw) = fs::read_to_string(&file_path) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<WorkerCrashReportPayload>(&raw) else {
            tracing::warn!(
                "Skipping unreadable pending crash report: {}",
                file_path.display()
            );
            continue;
        };
        let clean_payload = sanitize_crash_payload(parsed);

        match post_report_json(&url, &clean_payload, Duration::from_secs(10)).await {
            Ok(_) => {
                let _ = move_crash_file_to_sent(&data_dir, &file_path);
                sent_count += 1;
            }
            Err(err_msg) => {
                tracing::warn!(
                    "Stopping pending crash flush after network/HTTP error: {err_msg}"
                );
                break;
            }
        }
    }

    Ok(sent_count)
}

// ---------------------------------------------------------------------------
// Rust Panic Hook
// ---------------------------------------------------------------------------

fn panic_payload_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown Rust panic payload".to_string()
    }
}

/// Install a process-wide panic hook that writes a sanitized pending crash report under
/// `<data_dir>/crashes/pending-<uuid>.json` and attempts a bounded <= 3s upload when
/// `auto_send_crash_reports` is enabled and a telemetry endpoint is configured.
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if !PANIC_IN_PROGRESS.swap(true, Ordering::SeqCst) {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let raw_msg = panic_payload_message(panic_info);
                let location = panic_info
                    .location()
                    .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
                    .unwrap_or_else(|| "unknown location".to_string());
                let backtrace = std::backtrace::Backtrace::force_capture().to_string();

                let data_dir = current_app_data_dir();
                let config_path = current_config_path_from_data_dir(&data_dir);
                let auto_send = storage::load_config_from_path(&config_path)
                    .map(|c| c.auto_send_crash_reports)
                    .unwrap_or(true);

                let log_excerpt = collect_sanitized_log_excerpt(&data_dir);
                let req = CrashReportRequest {
                    crash_kind: "rust_panic".to_string(),
                    message: raw_msg,
                    stack_trace: Some(backtrace),
                    source: Some(location),
                };
                let payload = build_crash_payload(
                    &req,
                    Some(setup_manager::current_platform_tag().to_string()),
                    None,
                    Some(format!(
                        "## Panic Context\n- **OS**: {}\n\n### Recent Logs\n```text\n{}\n```",
                        detect_os_info(),
                        log_excerpt
                    )),
                );

                tracing::error!(
                    target: "easysrt::panic",
                    fingerprint = %payload.fingerprint,
                    source = %payload.source.as_deref().unwrap_or("unknown"),
                    "Unhandled Rust panic: {}",
                    payload.message
                );

                let prefix = if auto_send { "pending" } else { "local" };
                let saved_path = write_crash_report_json(&data_dir, prefix, &payload).ok();

                if auto_send {
                    if let (Some(endpoint), Some(path)) = (resolve_telemetry_endpoint(), saved_path)
                    {
                        let url = format!("{endpoint}/v1/report/crash");
                        let (tx, rx) = std::sync::mpsc::channel::<()>();
                        let data_dir_bg = data_dir.clone();
                        std::thread::spawn(move || {
                            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                            {
                                let res = rt.block_on(post_report_json(
                                    &url,
                                    &payload,
                                    Duration::from_secs(3),
                                ));
                                if res.is_ok() {
                                    let _ = move_crash_file_to_sent(&data_dir_bg, &path);
                                }
                            }
                            let _ = tx.send(());
                        });
                        let _ = rx.recv_timeout(Duration::from_secs(3));
                    }
                }
            }));
            PANIC_IN_PROGRESS.store(false, Ordering::SeqCst);
        }

        prev_hook(panic_info);
    }));
}

// ---------------------------------------------------------------------------
// Background Subprocess Failure Auto-Reporting
// ---------------------------------------------------------------------------

/// Write a sanitized `subprocess_crash` pending report into `data_dir` (testable core helper).
pub fn write_subprocess_failure_report_to_data_dir(
    data_dir: &Path,
    tool: &str,
    detail: &str,
    auto_send: bool,
) -> Result<(PathBuf, WorkerCrashReportPayload)> {
    let clean_tool = truncate_chars(sanitize_diagnostic_text(tool).trim(), 64);
    let clean_detail = truncate_utf8_bytes(sanitize_diagnostic_text(detail).trim(), MAX_STACK_BYTES);
    let headline = clean_detail
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Subprocess failed fatally");

    let req = CrashReportRequest {
        crash_kind: "subprocess_crash".to_string(),
        message: format!("{clean_tool}: {headline}"),
        stack_trace: Some(clean_detail),
        source: Some(clean_tool),
    };

    let log_excerpt = collect_sanitized_log_excerpt(data_dir);
    let payload = build_crash_payload(
        &req,
        Some(setup_manager::current_platform_tag().to_string()),
        None,
        Some(format!(
            "## Subprocess Failure Context\n- **OS**: {}\n\n### Recent Logs\n```text\n{}\n```",
            detect_os_info(),
            log_excerpt
        )),
    );

    let prefix = if auto_send { "pending" } else { "local" };
    let path = write_crash_report_json(data_dir, prefix, &payload)?;
    Ok((path, payload))
}

/// Log a fatal, non-cancelled subprocess failure via `tracing::error!` and persist/submit a
/// deduplicated `subprocess_crash` report in the background.
pub fn record_subprocess_failure_async(tool: &str, detail: &str) {
    let clean_tool = sanitize_diagnostic_text(tool);
    let clean_detail = sanitize_diagnostic_text(detail);

    tracing::error!(
        target: "easysrt::subprocess",
        tool = %clean_tool,
        "Fatal subprocess failure: {}",
        clean_detail
    );

    // Avoid writing mock test crash files into the user's real app data directory during `cargo test`
    if cfg!(test) {
        return;
    }

    let fp = compute_crash_fingerprint("subprocess_crash", APP_VERSION, &clean_tool, &clean_detail);
    {
        let set_lock = SUBPROCESS_REPORT_DEDUPE.get_or_init(|| Mutex::new(HashSet::new()));
        let mut set = set_lock.lock().unwrap_or_else(|e| e.into_inner());
        if set.len() >= MAX_SUBPROCESS_REPORTS_PER_SESSION || !set.insert(fp) {
            return;
        }
    }

    let data_dir = current_app_data_dir();
    let config_path = current_config_path_from_data_dir(&data_dir);

    let task = async move {
        let auto_send = storage::load_config_from_path(&config_path)
            .map(|c| c.auto_send_crash_reports)
            .unwrap_or(true);

        let Ok((saved_path, payload)) = write_subprocess_failure_report_to_data_dir(
            &data_dir,
            &clean_tool,
            &clean_detail,
            auto_send,
        ) else {
            return;
        };

        if auto_send {
            if let Some(endpoint) = resolve_telemetry_endpoint() {
                let url = format!("{endpoint}/v1/report/crash");
                if post_report_json(&url, &payload, Duration::from_secs(10))
                    .await
                    .is_ok()
                {
                    let _ = move_crash_file_to_sent(&data_dir, &saved_path);
                }
            }
        }
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(task);
    } else {
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(task);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{StorageMode, VramTier};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "easysrt_diag_test_{}_{}",
            prefix,
            Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitizer_redacts_secrets_tokens_and_os_usernames() {
        let fake_gemini = ["AIza", "SyTestDummyKey0123456789abcdefghijk"].concat();
        assert_eq!(fake_gemini.len(), 39);
        let raw = concat!(
            "DeepSeek key: sk-1234567890abcdef_XYZ\n",
            "HF token: hf_AbCdEfGhIjKlMnOpQrStUvWx1234\n",
            "Gemini key: __FAKE_GEMINI_KEY__\n",
            "GitHub classic: ghp_abc123XYZ987654\n",
            "GitHub OAuth: gho_OAuthToken987654\n",
            "GitHub fine-grained: github_pat_11AAA_bbbCCC999\n",
            "Header: Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig\n",
            "Header2: X-API-Key: header_secret_value_77\n",
            "Query: https://api.example.com/v1?x-api-key=query_secret_88&model=pro\n",
            "Json: {\"x-api-key\": \"json_secret_99\", \"ok\": true}\n",
            "Config: api_key=\"super_secret_value_42\", api-key: another_secret, apiKey=third_secret\n",
            "Cred: obf1:deadbeef0123456789\n",
            "UrlCred1: https://alice:s3cr3tPass@api.example.com/v1/chat\n",
            "UrlCred2: socks5://proxyUser:proxyPass@192.168.1.5:1080\n",
            "UrlClean: https://example.com/help?contact=support@example.com\n",
            "Win1: C:\\Users\\Milica\\AppData\\Local\\com.easysrt.studio\\logs\\app.log\n",
            "Win2: C:/Users/Marko/Videos/clip.mp4\n",
            "WinJson: \"C:\\\\Users\\\\Nikola\\\\Desktop\\\\test.srt\"\n",
            "Linux: /home/alice/Projects/EasySRT Studio/audio.wav\n",
            "macOS1: /Users/elena/Movies/interview.mov\n",
            "macOS2: HOME=/Users/stefan\n",
        )
        .replace("__FAKE_GEMINI_KEY__", &fake_gemini);

        let sanitized = sanitize_diagnostic_text(&raw);

        assert!(!sanitized.contains("sk-1234567890abcdef_XYZ"));
        assert!(!sanitized.contains("hf_AbCdEfGhIjKlMnOpQrStUvWx1234"));
        assert!(!sanitized.contains(&fake_gemini));
        assert!(sanitized.contains("Gemini key: [REDACTED_API_KEY]"));
        assert!(!sanitized.contains("ghp_abc123XYZ987654"));
        assert!(!sanitized.contains("gho_OAuthToken987654"));
        assert!(!sanitized.contains("github_pat_11AAA_bbbCCC999"));
        assert!(!sanitized.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(!sanitized.contains("header_secret_value_77"));
        assert!(!sanitized.contains("query_secret_88"));
        assert!(!sanitized.contains("json_secret_99"));
        assert!(!sanitized.contains("super_secret_value_42"));
        assert!(!sanitized.contains("another_secret"));
        assert!(!sanitized.contains("third_secret"));
        assert!(!sanitized.contains("deadbeef0123456789"));
        assert!(!sanitized.contains("alice:s3cr3tPass"));
        assert!(!sanitized.contains("proxyUser:proxyPass"));
        assert!(!sanitized.contains("Milica"));
        assert!(!sanitized.contains("Marko"));
        assert!(!sanitized.contains("Nikola"));
        assert!(!sanitized.contains("/home/alice/"));
        assert!(!sanitized.contains("/Users/elena/"));
        assert!(!sanitized.contains("/Users/stefan"));

        assert!(sanitized.contains("[REDACTED_API_KEY]"));
        assert!(sanitized.contains("[REDACTED_HF_TOKEN]"));
        assert!(sanitized.contains("[REDACTED_TOKEN]"));
        assert!(sanitized.contains("Bearer [REDACTED]"));
        assert!(sanitized.contains("X-API-Key: [REDACTED]"));
        assert!(sanitized.contains("https://api.example.com/v1?x-api-key=[REDACTED]&model=pro"));
        assert!(sanitized.contains(r#"{"x-api-key": "[REDACTED]", "ok": true}"#));
        assert!(sanitized.contains("https://<redacted>@api.example.com/v1/chat"));
        assert!(sanitized.contains("socks5://<redacted>@192.168.1.5:1080"));
        assert!(sanitized.contains("https://example.com/help?contact=support@example.com"));
        assert!(sanitized.contains(r"C:\Users\<USER>\AppData"));
        assert!(sanitized.contains(r"C:\Users\<USER>\Videos"));
        assert!(sanitized.contains(r"C:\\Users\\<USER>\\Desktop"));
        assert!(sanitized.contains("/home/<USER>/Projects/EasySRT Studio/audio.wav"));
        assert!(sanitized.contains("/Users/<USER>/Movies/interview.mov"));
        assert!(sanitized.contains("HOME=/Users/<USER>"));

        // Alias parity and idempotency
        assert_eq!(sanitize_text(&raw), sanitized);
        assert_eq!(sanitize_diagnostic_text(&sanitized), sanitized);
    }

    #[test]
    fn crash_fingerprint_is_deterministic_16_hex_and_normalizes_user_paths() {
        let fp1 = compute_crash_fingerprint(
            "rust_panic",
            "0.3.0",
            "Failed reading C:\\Users\\Alice\\AppData\\Local\\file.bin",
            "  0: easysrt::pipeline::run\n  1: main\n",
        );
        let fp2 = compute_crash_fingerprint(
            "rust_panic",
            "0.3.0",
            "Failed reading C:\\Users\\Bob\\AppData\\Local\\file.bin",
            "  0: easysrt::pipeline::run\n  1: main\n",
        );
        let fp_other = compute_crash_fingerprint(
            "react_error",
            "0.3.0",
            "Cannot read properties of undefined",
            "at App.tsx:12",
        );

        assert_eq!(fp1.len(), 16);
        assert!(fp1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(fp1, fp2, "identical crashes across different usernames must share fingerprint");
        assert_ne!(fp1, fp_other);
    }

    #[test]
    fn rotating_log_sink_rotates_to_app_log_1_when_exceeding_limit() {
        let dir = make_temp_dir("rotate");
        let logs_dir = dir.join("logs");

        let mut sink = RotatingFileSink::new(&logs_dir, 64);
        sink.write_bytes(&[b'A'; 40]);
        sink.write_bytes(&[b'B'; 40]); // triggers rotation because 40 + 40 > 64
        sink.flush_sink();

        let current = fs::read_to_string(logs_dir.join("app.log")).unwrap();
        let rotated = fs::read_to_string(logs_dir.join("app.log.1")).unwrap();
        assert_eq!(rotated, "A".repeat(40));
        assert_eq!(current, "B".repeat(40));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_excerpt_and_local_diagnostics_save_sanitize_and_exclude_private_files() {
        let dir = make_temp_dir("local_save");
        let paths =
            storage::resolve_storage_paths_from_root(&dir, StorageMode::Installed, None).unwrap();
        let data_dir = Path::new(&paths.data_dir);

        // Create logs containing sensitive data
        let logs_dir = data_dir.join("logs");
        fs::create_dir_all(&logs_dir).unwrap();
        fs::write(
            logs_dir.join("app.log"),
            "INFO started by /home/alice/bin/app with token sk-supersecret123456\n",
        )
        .unwrap();
        fs::write(
            data_dir.join("setup-bootstrap.log"),
            "[EasySRT Setup] Installed in C:\\Users\\Petar\\AppData\\Local\\com.easysrt.studio\n",
        )
        .unwrap();

        // Create private files that MUST NEVER be read into diagnostics
        fs::write(
            data_dir.join(".credentials.json"),
            r#"{"entries":{"deepseek":"SECRET_CREDENTIAL_MARKER"}}"#,
        )
        .unwrap();
        fs::write(
            data_dir.join("checkpoints").join("job1.easysrt.json"),
            r#"{"sourceText":"PRIVATE_SUBTITLE_TEXT_NEVER_INCLUDE"}"#,
        )
        .unwrap();

        let catalog = setup_manager::load_embedded_catalog().unwrap();
        let gpu = GpuInfo {
            has_nvidia: true,
            gpu_name: Some("NVIDIA GeForce RTX 4060".to_string()),
            vram_mb: 8188,
            recommended_tier: VramTier::Vram8Gb,
            driver_version: Some("555.85".to_string()),
            cuda_supported: true,
            cuda_status_note: None,
        };

        let preview = collect_diagnostics_preview_for_paths(&paths, &catalog, &gpu, &[], "linux-x64");
        assert_eq!(preview.app_version, APP_VERSION);
        assert!(preview.log_excerpt.contains("/home/<USER>/bin/app"));
        assert!(preview.log_excerpt.contains("[REDACTED_API_KEY]"));
        assert!(preview.log_excerpt.contains(r"C:\Users\<USER>\AppData"));
        assert!(!preview.log_excerpt.contains("sk-supersecret123456"));
        assert!(!preview.log_excerpt.contains("Petar"));
        assert!(!preview.log_excerpt.contains("SECRET_CREDENTIAL_MARKER"));
        assert!(!preview.log_excerpt.contains("PRIVATE_SUBTITLE_TEXT_NEVER_INCLUDE"));

        let saved_path = save_diagnostics_local_to_data_dir(data_dir, &preview).unwrap();
        assert!(Path::new(&saved_path).is_file());
        let saved_content = fs::read_to_string(&saved_path).unwrap();
        assert!(saved_content.contains("EasySRT Studio Diagnostics Report"));
        assert!(saved_content.contains("NVIDIA GeForce RTX 4060"));
        assert!(!saved_content.contains("sk-supersecret123456"));
        assert!(!saved_content.contains("PRIVATE_SUBTITLE_TEXT_NEVER_INCLUDE"));

        let (crash_path, crash_payload) = write_subprocess_failure_report_to_data_dir(
            data_dir,
            "whisper-cli",
            "error loading C:\\Users\\Petar\\bin\\cublas64_12.dll: 0xC0000135",
            true,
        )
        .unwrap();
        assert!(crash_path.is_file());
        assert_eq!(crash_payload.crash_kind, "subprocess_crash");
        assert_eq!(crash_payload.fingerprint.len(), 16);
        assert!(!crash_payload.message.contains("Petar"));
        assert!(crash_payload.message.contains(r"C:\Users\<USER>\bin"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn telemetry_endpoint_resolution_defaults_and_supports_overrides() {
        assert_eq!(
            DEFAULT_TELEMETRY_URL,
            "https://easysrt-telemetry.ig-systems.rs"
        );
        assert_eq!(
            resolve_telemetry_endpoint_from(None, None, DEFAULT_TELEMETRY_URL),
            Some("https://easysrt-telemetry.ig-systems.rs".to_string())
        );
        // Explicit empty/disabled runtime override disables remote upload for offline testing
        assert_eq!(
            resolve_telemetry_endpoint_from(Some(""), None, DEFAULT_TELEMETRY_URL),
            None
        );
        assert_eq!(
            resolve_telemetry_endpoint_from(Some("off"), None, DEFAULT_TELEMETRY_URL),
            None
        );
        // Explicit empty default with no env overrides returns None
        assert_eq!(resolve_telemetry_endpoint_from(None, None, ""), None);
        // Custom override trims trailing slash
        assert_eq!(
            resolve_telemetry_endpoint_from(
                Some("https://custom.example.org///"),
                None,
                DEFAULT_TELEMETRY_URL
            ),
            Some("https://custom.example.org".to_string())
        );
    }
}
