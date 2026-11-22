//! Tauri IPC command surface matching `src/lib/tauri.ts`.

use crate::models::{
    ApiKeyStatus, AppConfig, CatalogItemKind, CatalogManifest, CustomModelEntry, ExportOptions,
    ExportResult, InstalledComponentStatus, JobOptions, SegmentRules, SerbianScript, SetupStatus,
    SubtitleCue, SubtitleProject, TranslationProviderKind,
};
use crate::pipeline::{self, PipelineState};
use crate::{setup_manager, storage};
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
