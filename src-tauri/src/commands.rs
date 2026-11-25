//! Tauri IPC command surface matching `src/lib/tauri.ts`.

use crate::models::{
    ApiKeyStatus, AppConfig, BugReportRequest, CatalogItemKind, CatalogManifest,
    CrashReportRequest, CustomModelEntry, DiagnosticsPreview, ExportOptions, ExportResult,
    InstalledComponentStatus, JobOptions, MediaProbeResult, ReportSubmitResult, SegmentRules,
    SerbianScript, SetupStatus, SubtitleCue, SubtitleProject, TranslationProviderKind,
    WaveformPreviewData,
};
use crate::pipeline::{self, PipelineState};
use crate::{diagnostics, setup_manager, storage};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Manager, State};

const DEFAULT_WAVEFORM_BUCKETS: u32 = 2_000;
const MAX_WAVEFORM_BUCKETS: u32 = 16_384;

fn map_err(err: anyhow::Error) -> String {
    err.to_string()
}

/// Offloads synchronous filesystem, subprocess, or CPU-heavy work onto Tokio's blocking pool
/// so async IPC handlers never stall the runtime (`BUG-M2`).
async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("Background task panicked: {e}"))?
        .map_err(map_err)
}

// ---------------------------------------------------------------------------
// Setup & Catalog Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_setup_status(app: AppHandle) -> Result<SetupStatus, String> {
    run_blocking(move || setup_manager::get_setup_status(&app)).await
}

#[tauri::command]
pub async fn get_catalog(app: AppHandle) -> Result<CatalogManifest, String> {
    run_blocking(move || setup_manager::load_catalog(&app)).await
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
pub async fn install_core_runtime(app: AppHandle) -> Result<SetupStatus, String> {
    setup_manager::install_core_runtime(&app)
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
    run_blocking(move || setup_manager::import_custom_model(&app, &source_path, kind, name)).await
}

#[tauri::command]
pub async fn delete_model(app: AppHandle, model_id: String) -> Result<(), String> {
    run_blocking(move || setup_manager::delete_model(&app, &model_id)).await
}

// ---------------------------------------------------------------------------
// Config & API Key Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    run_blocking(move || storage::load_config(&app)).await
}

#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    run_blocking(move || storage::save_config(&app, &config)).await
}

#[tauri::command]
pub async fn get_api_key_status(app: AppHandle) -> Result<ApiKeyStatus, String> {
    run_blocking(move || storage::get_api_key_status(&app)).await
}

#[tauri::command]
pub async fn set_api_key(
    app: AppHandle,
    provider: TranslationProviderKind,
    api_key: String,
) -> Result<ApiKeyStatus, String> {
    run_blocking(move || storage::set_api_key(&app, provider, &api_key)).await
}

#[tauri::command]
pub async fn clear_api_key(
    app: AppHandle,
    provider: TranslationProviderKind,
) -> Result<ApiKeyStatus, String> {
    run_blocking(move || storage::clear_api_key(&app, provider)).await
}

// ---------------------------------------------------------------------------
// Checkpoint Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_checkpoints(app: AppHandle) -> Result<Vec<SubtitleProject>, String> {
    run_blocking(move || storage::list_checkpoints(&app)).await
}

#[tauri::command]
pub async fn load_checkpoint(app: AppHandle, job_id: String) -> Result<SubtitleProject, String> {
    run_blocking(move || storage::load_checkpoint(&app, &job_id)).await
}

#[tauri::command]
pub async fn save_project_checkpoint(
    app: AppHandle,
    project: SubtitleProject,
) -> Result<(), String> {
    run_blocking(move || {
        storage::validate_job_id(&project.job_id)?;
        storage::save_checkpoint(&app, &project)
    })
    .await
}

#[tauri::command]
pub async fn delete_checkpoint(app: AppHandle, job_id: String) -> Result<(), String> {
    run_blocking(move || storage::delete_checkpoint(&app, &job_id)).await
}

// ---------------------------------------------------------------------------
// Pipeline & Background Media Probe Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn import_subtitle_file(
    app: AppHandle,
    subtitle_path: String,
    options: JobOptions,
) -> Result<SubtitleProject, String> {
    run_blocking(move || pipeline::import_subtitle_file(&app, subtitle_path, options)).await
}

#[tauri::command]
pub async fn try_load_companion_subtitles(
    audio_path: String,
    options: JobOptions,
) -> Result<Option<SubtitleProject>, String> {
    run_blocking(move || {
        pipeline::importer::try_load_companion_subtitles(PathBuf::from(&audio_path).as_path(), &options)
    })
    .await
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
    run_blocking(move || pipeline::resegment_project(project, rules)).await
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

#[tauri::command]
pub async fn probe_media_file(
    app: AppHandle,
    state: State<'_, PipelineState>,
    job_id: String,
    audio_path: String,
) -> Result<MediaProbeResult, String> {
    pipeline::probe::probe_media_file(&app, state.inner(), &job_id, &audio_path)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn cancel_media_probe(
    state: State<'_, PipelineState>,
    job_id: Option<String>,
) -> Result<bool, String> {
    Ok(state.cancel_active_probe(job_id.as_deref()).await)
}

// ---------------------------------------------------------------------------
// Script Conversion, Export & Scoped Streaming Waveform Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn convert_cues_script(
    cues: Vec<SubtitleCue>,
    target_script: SerbianScript,
) -> Result<Vec<SubtitleCue>, String> {
    run_blocking(move || pipeline::convert_cues_script(cues, target_script)).await
}

#[tauri::command]
pub async fn export_subtitles(
    app: AppHandle,
    project: SubtitleProject,
    options: ExportOptions,
) -> Result<ExportResult, String> {
    run_blocking(move || pipeline::export_subtitles(&app, project, options)).await
}

/// Validates that `audio_path` resolves to a regular file with an approved media extension,
/// computes bounded-RAM waveform peaks via FFmpeg, and dynamically grants read-only access
/// to ONLY that canonical file on Tauri's `asset://` protocol scope (`SEC-H1`, WS6).
#[tauri::command]
pub async fn prepare_audio_waveform(
    app: AppHandle,
    audio_path: String,
    target_buckets: Option<u32>,
) -> Result<WaveformPreviewData, String> {
    let app_for_resolve = app.clone();
    let (canonical_path, ffmpeg_bin) = run_blocking(move || {
        let canonical = pipeline::ffmpeg::validate_approved_media_file(&audio_path)?;
        let ffmpeg = setup_manager::resolve_binary_path(&app_for_resolve, CatalogItemKind::Ffmpeg)?;
        Ok((canonical, ffmpeg))
    })
    .await?;

    let buckets = target_buckets
        .unwrap_or(DEFAULT_WAVEFORM_BUCKETS)
        .clamp(1, MAX_WAVEFORM_BUCKETS) as usize;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let (peaks, duration_ms) = pipeline::ffmpeg::extract_waveform_peaks(
        &ffmpeg_bin,
        &canonical_path,
        buckets,
        &cancel_flag,
    )
    .await
    .map_err(map_err)?;

    app.asset_protocol_scope()
        .allow_file(&canonical_path)
        .map_err(|e| format!("Failed to grant asset protocol access for audio preview: {e}"))?;

    Ok(WaveformPreviewData {
        canonical_path: canonical_path.to_string_lossy().into_owned(),
        duration_ms,
        peaks,
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

// ---------------------------------------------------------------------------
// Diagnostics, Crash Reporting, & Bug Report Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn collect_diagnostics_preview(app: AppHandle) -> Result<DiagnosticsPreview, String> {
    diagnostics::collect_diagnostics_preview(&app)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn submit_bug_report(
    app: AppHandle,
    request: BugReportRequest,
) -> Result<ReportSubmitResult, String> {
    diagnostics::submit_bug_report(&app, request)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn submit_crash_report(
    app: AppHandle,
    request: CrashReportRequest,
) -> Result<ReportSubmitResult, String> {
    diagnostics::submit_crash_report(&app, request)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn flush_pending_crash_reports(app: AppHandle) -> Result<u32, String> {
    diagnostics::flush_pending_crash_reports(&app)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn save_diagnostics_local(app: AppHandle) -> Result<String, String> {
    run_blocking(move || diagnostics::save_diagnostics_local(&app)).await
}

#[cfg(test)]
mod tests {
    use crate::pipeline::ffmpeg::validate_approved_media_file;
    use std::fs;

    #[test]
    fn media_allowlist_rejects_sensitive_files_directories_and_non_media_symlinks() {
        let base = std::env::temp_dir().join(format!(
            "easysrt_media_allowlist_{}",
            uuid::Uuid::new_v4()
        ));
        let ssh_dir = base.join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();

        let json_file = base.join("config.json");
        let ssh_key = ssh_dir.join("id_rsa");
        let mp3_file = base.join("sample.MP3");
        let webm_file = base.join("video.webm");
        let opus_file = base.join("voice.opus");
        let wma_file = base.join("legacy.wma");

        fs::write(&json_file, b"{}").unwrap();
        fs::write(&ssh_key, b"-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
        fs::write(&mp3_file, b"ID3").unwrap();
        fs::write(&webm_file, b"WEBM").unwrap();
        fs::write(&opus_file, b"OggS").unwrap();
        fs::write(&wma_file, b"ASF").unwrap();

        // Must reject non-media files, extensionless keys, directories, empty/NUL, and missing paths
        assert!(validate_approved_media_file(json_file.to_str().unwrap()).is_err());
        assert!(validate_approved_media_file(ssh_key.to_str().unwrap()).is_err());
        assert!(validate_approved_media_file(base.to_str().unwrap()).is_err());
        assert!(validate_approved_media_file("").is_err());
        assert!(validate_approved_media_file("clip.mp3\0.json").is_err());
        assert!(validate_approved_media_file(base.join("missing.mp3").to_str().unwrap()).is_err());

        // Must accept approved media extensions (case-insensitive)
        assert!(validate_approved_media_file(mp3_file.to_str().unwrap()).is_ok());
        assert!(validate_approved_media_file(webm_file.to_str().unwrap()).is_ok());
        assert!(validate_approved_media_file(opus_file.to_str().unwrap()).is_ok());
        assert!(validate_approved_media_file(wma_file.to_str().unwrap()).is_ok());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            // Symlinks disguised with `.mp3` extension pointing to non-media files MUST be rejected
            let evil_json_link = base.join("disguised_json.mp3");
            let evil_ssh_link = base.join("disguised_key.wav");
            let evil_dir_link = base.join("disguised_dir.mp4");
            let good_media_link = base.join("alias_clip.mp3");

            symlink(&json_file, &evil_json_link).unwrap();
            symlink(&ssh_key, &evil_ssh_link).unwrap();
            symlink(&ssh_dir, &evil_dir_link).unwrap();
            symlink(&mp3_file, &good_media_link).unwrap();

            assert!(
                validate_approved_media_file(evil_json_link.to_str().unwrap()).is_err(),
                "Symlink fake.mp3 -> config.json must be rejected"
            );
            assert!(
                validate_approved_media_file(evil_ssh_link.to_str().unwrap()).is_err(),
                "Symlink fake.wav -> .ssh/id_rsa must be rejected"
            );
            assert!(
                validate_approved_media_file(evil_dir_link.to_str().unwrap()).is_err(),
                "Symlink fake.mp4 -> directory must be rejected"
            );

            // Symlink pointing to a genuine approved media file resolves to its canonical target
            let resolved =
                validate_approved_media_file(good_media_link.to_str().unwrap()).unwrap();
            assert_eq!(resolved, fs::canonicalize(&mp3_file).unwrap());
        }

        let _ = fs::remove_dir_all(&base);
    }
}
