//! Subtitle generation pipeline (audio conversion, Whisper transcription,
//! segmentation, local/cloud translation, Serbian polishing, script conversion,
//! and export).
//! Stub module to be implemented in Wave 2.

use crate::models::{
    ExportOptions, ExportResult, JobOptions, SegmentRules, SerbianScript, SubtitleCue,
    SubtitleProject,
};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::AppHandle;
use tokio::sync::Mutex;

/// Shared pipeline runtime state registered via `tauri::Builder::manage`.
#[derive(Default)]
pub struct PipelineState {
    active_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl PipelineState {
    pub async fn register_job(&self, job_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut jobs = self.active_jobs.lock().await;
        jobs.insert(job_id.to_string(), Arc::clone(&flag));
        flag
    }

    pub async fn cancel_job(&self, job_id: &str) -> bool {
        let jobs = self.active_jobs.lock().await;
        if let Some(flag) = jobs.get(job_id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub async fn finish_job(&self, job_id: &str) {
        let mut jobs = self.active_jobs.lock().await;
        jobs.remove(job_id);
    }
}

pub async fn transcribe_audio(
    _app: &AppHandle,
    _state: &PipelineState,
    _audio_path: String,
    _options: JobOptions,
    _job_id: Option<String>,
) -> Result<SubtitleProject> {
    Err(anyhow!("pipeline::transcribe_audio not implemented"))
}

pub fn resegment_project(
    _project: SubtitleProject,
    _rules: SegmentRules,
) -> Result<SubtitleProject> {
    Err(anyhow!("pipeline::resegment_project not implemented"))
}

pub async fn translate_project(
    _app: &AppHandle,
    _state: &PipelineState,
    _project: SubtitleProject,
) -> Result<SubtitleProject> {
    Err(anyhow!("pipeline::translate_project not implemented"))
}

pub async fn polish_project(
    _app: &AppHandle,
    _state: &PipelineState,
    _project: SubtitleProject,
) -> Result<SubtitleProject> {
    Err(anyhow!("pipeline::polish_project not implemented"))
}

pub async fn run_full_pipeline(
    _app: &AppHandle,
    _state: &PipelineState,
    _audio_path: String,
    _options: JobOptions,
    _job_id: Option<String>,
) -> Result<SubtitleProject> {
    Err(anyhow!("pipeline::run_full_pipeline not implemented"))
}

pub fn convert_cues_script(
    _cues: Vec<SubtitleCue>,
    _target_script: SerbianScript,
) -> Result<Vec<SubtitleCue>> {
    Err(anyhow!("pipeline::convert_cues_script not implemented"))
}

pub fn export_subtitles(
    _app: &AppHandle,
    _project: SubtitleProject,
    _options: ExportOptions,
) -> Result<ExportResult> {
    Err(anyhow!("pipeline::export_subtitles not implemented"))
}
