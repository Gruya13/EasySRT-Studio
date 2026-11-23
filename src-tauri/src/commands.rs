//! Tauri IPC command surface matching `src/lib/tauri.ts`.

use crate::models::{
    ApiKeyStatus, AppConfig, AudioPreviewData, CatalogItemKind, CatalogManifest, CustomModelEntry,
    ExportOptions, ExportResult, InstalledComponentStatus, JobOptions, SegmentRules,
    SerbianScript, SetupStatus, SubtitleCue, SubtitleProject, TranslationProviderKind,
};
use crate::pipeline::{self, PipelineState};
use crate::{setup_manager, storage};
use std::path::Path;
use tauri::{AppHandle, State};

fn map_err(err: anyhow::Error) -> String {
    err.to_string()
}

// ---------------------------------------------------------------------------
// Setup & Catalog Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_setup_status(app: AppHandle) -> Result<SetupStatus, String> {
    setup_manager::get_setup_status(&app).map_err(map_err)
}

#[tauri::command]
pub async fn get_catalog(app: AppHandle) -> Result<CatalogManifest, String> {
    setup_manager::load_catalog(&app).map_err(map_err)
}

#[tauri::command]
pub async fn install_component(
    app: AppHandle,
    item_id: String,
) -> Result<InstalledComponentStatus, String> {
    setup_manager::install_component(&app, &item_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn install_recommended_bundle(app: AppHandle) -> Result<SetupStatus, String> {
    setup_manager::install_recommended_bundle(&app)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn import_custom_model(
    app: AppHandle,
    source_path: String,
    kind: CatalogItemKind,
    name: Option<String>,
) -> Result<CustomModelEntry, String> {
    setup_manager::import_custom_model(&app, &source_path, kind, name).map_err(map_err)
}

#[tauri::command]
pub async fn delete_model(app: AppHandle, model_id: String) -> Result<(), String> {
    setup_manager::delete_model(&app, &model_id).map_err(map_err)
}

// ---------------------------------------------------------------------------
// Config & API Key Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    storage::load_config(&app).map_err(map_err)
}

#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    storage::save_config(&app, &config).map_err(map_err)
}

#[tauri::command]
pub async fn get_api_key_status(app: AppHandle) -> Result<ApiKeyStatus, String> {
    storage::get_api_key_status(&app).map_err(map_err)
}

#[tauri::command]
pub async fn set_api_key(
    app: AppHandle,
    provider: TranslationProviderKind,
    api_key: String,
) -> Result<ApiKeyStatus, String> {
    storage::set_api_key(&app, provider, &api_key).map_err(map_err)
}

#[tauri::command]
pub async fn clear_api_key(
    app: AppHandle,
    provider: TranslationProviderKind,
) -> Result<ApiKeyStatus, String> {
    storage::clear_api_key(&app, provider).map_err(map_err)
}

// ---------------------------------------------------------------------------
// Checkpoint Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_checkpoints(app: AppHandle) -> Result<Vec<SubtitleProject>, String> {
    storage::list_checkpoints(&app).map_err(map_err)
}

#[tauri::command]
pub async fn load_checkpoint(app: AppHandle, job_id: String) -> Result<SubtitleProject, String> {
    storage::load_checkpoint(&app, &job_id).map_err(map_err)
}

#[tauri::command]
pub async fn delete_checkpoint(app: AppHandle, job_id: String) -> Result<(), String> {
    storage::delete_checkpoint(&app, &job_id).map_err(map_err)
}

// ---------------------------------------------------------------------------
// Pipeline Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn import_subtitle_file(
    app: AppHandle,
    subtitle_path: String,
    options: JobOptions,
) -> Result<SubtitleProject, String> {
    pipeline::import_subtitle_file(&app, subtitle_path, options).map_err(map_err)
}

#[tauri::command]
pub async fn try_load_companion_subtitles(
    audio_path: String,
    options: JobOptions,
) -> Result<Option<SubtitleProject>, String> {
    pipeline::importer::try_load_companion_subtitles(std::path::Path::new(&audio_path), &options)
        .map_err(map_err)
}

#[tauri::command]
pub async fn transcribe_audio(
    app: AppHandle,
    state: State<'_, PipelineState>,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
) -> Result<SubtitleProject, String> {
    pipeline::transcribe_audio(&app, state.inner(), audio_path, options, job_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn resegment_project(
    project: SubtitleProject,
    rules: SegmentRules,
) -> Result<SubtitleProject, String> {
    pipeline::resegment_project(project, rules).map_err(map_err)
}

#[tauri::command]
pub async fn translate_project(
    app: AppHandle,
    state: State<'_, PipelineState>,
    project: SubtitleProject,
) -> Result<SubtitleProject, String> {
    pipeline::translate_project(&app, state.inner(), project)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn polish_project(
    app: AppHandle,
    state: State<'_, PipelineState>,
    project: SubtitleProject,
) -> Result<SubtitleProject, String> {
    pipeline::polish_project(&app, state.inner(), project)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn condense_project_cps(
    app: AppHandle,
    state: State<'_, PipelineState>,
    project: SubtitleProject,
) -> Result<SubtitleProject, String> {
    pipeline::condense_project_cps(&app, state.inner(), project)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn run_full_pipeline(
    app: AppHandle,
    state: State<'_, PipelineState>,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
) -> Result<SubtitleProject, String> {
    pipeline::run_full_pipeline(&app, state.inner(), audio_path, options, job_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn run_local_llm_batch(
    app: AppHandle,
    state: State<'_, PipelineState>,
    projects: Vec<SubtitleProject>,
) -> Result<Vec<SubtitleProject>, String> {
    pipeline::run_local_llm_batch(&app, state.inner(), projects)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn cancel_job(
    state: State<'_, PipelineState>,
    job_id: String,
) -> Result<(), String> {
    let _ = state.cancel_job(&job_id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Script Conversion & Export Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn convert_cues_script(
    cues: Vec<SubtitleCue>,
    target_script: SerbianScript,
) -> Result<Vec<SubtitleCue>, String> {
    pipeline::convert_cues_script(cues, target_script).map_err(map_err)
}

#[tauri::command]
pub async fn export_subtitles(
    app: AppHandle,
    project: SubtitleProject,
    options: ExportOptions,
) -> Result<ExportResult, String> {
    pipeline::export_subtitles(&app, project, options).map_err(map_err)
}

fn detect_audio_mime(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" | "aac" | "mp4" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[tauri::command]
pub async fn read_audio_preview_base64(audio_path: String) -> Result<AudioPreviewData, String> {
    let path = Path::new(&audio_path);
    if !path.is_file() {
        return Err(format!("Audio file not found: {audio_path}"));
    }
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("Failed to inspect audio file: {e}"))?;
    const MAX_PREVIEW_BYTES: u64 = 128 * 1024 * 1024;
    if meta.len() > MAX_PREVIEW_BYTES {
        return Err(format!(
            "Audio file exceeds {} MB preview limit ({} bytes)",
            MAX_PREVIEW_BYTES / (1024 * 1024),
            meta.len()
        ));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;
    Ok(AudioPreviewData {
        mime_type: detect_audio_mime(path).to_string(),
        base64_data: encode_base64(&bytes),
    })
}

#[cfg(target_os = "linux")]
async fn hyprland_set_maximized(pid: u32, maximized: Option<bool>) -> Option<bool> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let out = tokio::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let clients: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    let target_client = clients
        .iter()
        .find(|c| c.get("pid").and_then(|v| v.as_u64()) == Some(u64::from(pid)))
        .or_else(|| {
            clients.iter().find(|c| {
                let cls = c
                    .get("class")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let title = c
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                cls.contains("easysrt") || title.contains("easysrt")
            })
        })?;

    let addr = target_client.get("address").and_then(|v| v.as_str())?;
    let current = target_client
        .get("fullscreen")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        != 0;
    let target = maximized.unwrap_or(!current);
    if target != current {
        let action = if target { "set" } else { "unset" };
        let expr = format!(
            "hl.dsp.window.fullscreen({{ mode = \"maximized\", action = \"{action}\", window = \"address:{addr}\" }})"
        );
        let _ = tokio::process::Command::new("hyprctl")
            .args(["dispatch", &expr])
            .output()
            .await;
    }
    Some(target)
}

#[tauri::command]
pub async fn set_window_maximized(
    window: tauri::WebviewWindow,
    maximized: Option<bool>,
) -> std::result::Result<bool, String> {
    #[cfg(target_os = "linux")]
    if let Some(target) = hyprland_set_maximized(std::process::id(), maximized).await {
        if target {
            let _ = window.maximize();
        } else {
            let _ = window.unmaximize();
        }
        return Ok(target);
    }

    let current = window.is_maximized().unwrap_or(false);
    let target = maximized.unwrap_or(!current);
    if target {
        window.maximize().map_err(|e| e.to_string())?;
    } else {
        window.unmaximize().map_err(|e| e.to_string())?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rfc4648_base64_and_detects_mime_types() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode_base64(&[0xff, 0x00, 0xab]), "/wCr");

        assert_eq!(detect_audio_mime(Path::new("/tmp/test.MP3")), "audio/mpeg");
        assert_eq!(detect_audio_mime(Path::new("/tmp/test.wav")), "audio/wav");
        assert_eq!(detect_audio_mime(Path::new("/tmp/test.opus")), "audio/ogg");
        assert_eq!(detect_audio_mime(Path::new("/tmp/test.flac")), "audio/flac");
        assert_eq!(detect_audio_mime(Path::new("/tmp/test.m4a")), "audio/mp4");
        assert_eq!(detect_audio_mime(Path::new("/tmp/test.webm")), "audio/webm");
    }
}
