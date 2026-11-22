//! Subtitle generation pipeline (audio conversion, Whisper transcription,
//! segmentation, local/cloud translation, Serbian polishing, script conversion,
//! and export).

pub mod exporter;
pub mod ffmpeg;
pub mod llama_server;
pub mod segmenter;
pub mod sr_script;
pub mod translator;
pub mod whisper;

use crate::models::{
    CatalogItemKind, ExportOptions, ExportResult, JobOptions, JobPhase, JobProgressEvent,
    SegmentRules, SerbianScript, SubtitleCue, SubtitleProject, TranslationProviderKind,
    EVENT_JOB_PROGRESS,
};
use crate::pipeline::translator::{BatchOperation, LlmEndpointConfig};
use crate::{setup_manager, storage};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};
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

fn emit_job_progress(
    app: &AppHandle,
    job_id: &str,
    phase: JobPhase,
    percent: f64,
    detail: impl Into<String>,
) {
    let event = JobProgressEvent {
        job_id: job_id.to_string(),
        phase,
        percent: percent.clamp(0.0, 100.0),
        detail: detail.into(),
    };
    let _ = app.emit(EVENT_JOB_PROGRESS, event);
}

async fn transcribe_audio_with_flag(
    app: &AppHandle,
    job_id: &str,
    audio_path: &str,
    options: JobOptions,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<SubtitleProject> {
    let ffmpeg_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::Ffmpeg)?;
    let whisper_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::WhisperBin)?;
    let whisper_model = setup_manager::resolve_active_whisper_model_path(app)?;
    let vad_model = setup_manager::resolve_vad_model_path(app)?;
    let storage_paths = storage::resolve_storage_paths(app, None)?;

    let input_path = Path::new(audio_path);
    let audio_file_name = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("audio.wav")
        .to_string();

    emit_job_progress(
        app,
        job_id,
        JobPhase::ConvertingAudio,
        5.0,
        format!("Converting {audio_file_name} to 16 kHz mono WAV..."),
    );

    let temp_wav = Path::new(&storage_paths.data_dir)
        .join("temp")
        .join(format!("{job_id}.16k.wav"));

    let converted =
        ffmpeg::convert_to_16khz_mono_wav(&ffmpeg_bin, input_path, &temp_wav, cancel_flag).await?;

    emit_job_progress(
        app,
        job_id,
        JobPhase::Transcribing,
        15.0,
        "Running Whisper ASR transcription...",
    );

    let whisper_res = whisper::run_whisper_cli(
        &whisper_bin,
        &whisper_model,
        vad_model.as_deref(),
        &converted.wav_path,
        &options.source_language,
        &options.glossary,
        cancel_flag,
        |pct, detail| {
            emit_job_progress(app, job_id, JobPhase::Transcribing, pct, detail);
        },
    )
    .await;

    // Always clean up temporary 16 kHz WAV file
    let _ = fs::remove_file(&converted.wav_path);

    let whisper_out = whisper_res?;

    emit_job_progress(
        app,
        job_id,
        JobPhase::Segmenting,
        92.0,
        "Segmenting words into subtitle cues...",
    );

    let rules = segmenter::normalize_rules(&options.segment_rules);
    let mut cues = segmenter::segment_words(&whisper_out.words, &rules);

    let detected_or_source = whisper_out
        .detected_language
        .as_deref()
        .unwrap_or(&options.source_language);
    if detected_or_source.eq_ignore_ascii_case("sr") {
        cues = convert_cues_script(cues, options.script)?;
    }

    let project = SubtitleProject {
        job_id: job_id.to_string(),
        audio_path: audio_path.to_string(),
        audio_file_name,
        duration_ms: converted.duration_ms,
        detected_language: whisper_out.detected_language,
        options: JobOptions {
            segment_rules: rules,
            ..options
        },
        words: whisper_out.words,
        cues,
        phase: JobPhase::Completed,
        completed_translation_batches: 0,
        error_message: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    storage::save_checkpoint(app, &project)?;

    Ok(project)
}

async fn run_project_llm_stage(
    app: &AppHandle,
    project: &mut SubtitleProject,
    operation: BatchOperation,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    let config = storage::load_config(app)?;
    let provider = project
        .options
        .provider_override
        .unwrap_or(config.translation.provider);

    let phase = match operation {
        BatchOperation::Translate => JobPhase::Translating,
        BatchOperation::PolishSerbian => JobPhase::Polishing,
    };

    project.phase = phase;
    project.error_message = None;
    project.updated_at = chrono::Utc::now().to_rfc3339();
    let _ = storage::save_checkpoint(app, project);

    let job_id = project.job_id.clone();
    emit_job_progress(
        app,
        &job_id,
        phase,
        2.0,
        match operation {
            BatchOperation::Translate => "Preparing subtitle translation...",
            BatchOperation::PolishSerbian => "Preparing Serbian subtitle polishing...",
        },
    );

    let mut llama_guard = None;
    let endpoint = match provider {
        TranslationProviderKind::LocalLlama => {
            let llama_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::LlamaBin)?;
            let llm_model = setup_manager::resolve_active_llm_model_path(app)?;
            emit_job_progress(
                app,
                &job_id,
                phase,
                5.0,
                "Starting local llama-server...",
            );
            let guard = llama_server::LlamaServerGuard::start(
                &llama_bin,
                &llm_model,
                config.translation.context_window,
                cancel_flag,
            )
            .await?;
            let ep = LlmEndpointConfig {
                base_url: guard.base_url().to_string(),
                model: "local-llm".to_string(),
                api_key: None,
                batch_size: config.translation.batch_size,
            };
            llama_guard = Some(guard);
            ep
        }
        TranslationProviderKind::DeepSeek => {
            let api_key = storage::get_api_key(app, TranslationProviderKind::DeepSeek)?
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "DeepSeek API key is not configured. Please add your key in Settings -> Cloud API."
                    )
                })?;
            let model = if config.translation.deepseek_model.trim().is_empty() {
                "deepseek-chat".to_string()
            } else {
                config.translation.deepseek_model.trim().to_string()
            };
            LlmEndpointConfig {
                base_url: "https://api.deepseek.com/v1".to_string(),
                model,
                api_key: Some(api_key),
                batch_size: config.translation.batch_size,
            }
        }
        TranslationProviderKind::CustomOpenAi => {
            let api_key = storage::get_api_key(app, TranslationProviderKind::CustomOpenAi)?;
            let base_url = config.translation.custom_base_url.trim().to_string();
            if base_url.is_empty() {
                return Err(anyhow!(
                    "Custom OpenAI-compatible base URL is not configured in Settings."
                ));
            }
            let model = if config.translation.custom_model.trim().is_empty() {
                "default".to_string()
            } else {
                config.translation.custom_model.trim().to_string()
            };
            LlmEndpointConfig {
                base_url,
                model,
                api_key,
                batch_size: config.translation.batch_size,
            }
        }
    };

    let op_res = translator::run_batch_operation(
        project,
        operation,
        &endpoint,
        cancel_flag,
        |proj| storage::save_checkpoint(app, proj),
        |pct, detail| {
            emit_job_progress(app, &job_id, phase, pct, detail);
        },
    )
    .await;

    // Guarantee local llama-server is stopped immediately after batch operation completes
    if let Some(mut guard) = llama_guard.take() {
        guard.stop();
    }

    match op_res {
        Ok(()) => {
            project.phase = JobPhase::Completed;
            project.error_message = None;
            project.updated_at = chrono::Utc::now().to_rfc3339();
            storage::save_checkpoint(app, project)?;
            Ok(())
        }
        Err(err) => {
            let is_cancelled = cancel_flag.load(Ordering::SeqCst);
            project.phase = if is_cancelled {
                JobPhase::Cancelled
            } else {
                JobPhase::Failed
            };
            project.error_message = Some(err.to_string());
            project.updated_at = chrono::Utc::now().to_rfc3339();
            let _ = storage::save_checkpoint(app, project);
            emit_job_progress(app, &job_id, project.phase, 0.0, err.to_string());
            Err(err)
        }
    }
}

pub async fn transcribe_audio(
    app: &AppHandle,
    state: &PipelineState,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
) -> Result<SubtitleProject> {
    let resolved_job_id = job_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let cancel_flag = state.register_job(&resolved_job_id).await;
    let res =
        transcribe_audio_with_flag(app, &resolved_job_id, &audio_path, options, &cancel_flag).await;
    state.finish_job(&resolved_job_id).await;

    match res {
        Ok(project) => {
            emit_job_progress(
                app,
                &resolved_job_id,
                JobPhase::Completed,
                100.0,
                "Transcription completed.",
            );
            Ok(project)
        }
        Err(err) => {
            let phase = if cancel_flag.load(Ordering::SeqCst) {
                JobPhase::Cancelled
            } else {
                JobPhase::Failed
            };
            emit_job_progress(app, &resolved_job_id, phase, 0.0, err.to_string());
            Err(err)
        }
    }
}

pub fn resegment_project(
    mut project: SubtitleProject,
    rules: SegmentRules,
) -> Result<SubtitleProject> {
    let normalized = segmenter::normalize_rules(&rules);

    if !project.words.is_empty() {
        let mut new_cues = segmenter::segment_words(&project.words, &normalized);
        let detected_or_source = project
            .detected_language
            .as_deref()
            .unwrap_or(&project.options.source_language);
        if detected_or_source.eq_ignore_ascii_case("sr") {
            new_cues = convert_cues_script(new_cues, project.options.script)?;
        }
        project.cues = new_cues;
        project.completed_translation_batches = 0;
    } else {
        for cue in &mut project.cues {
            cue.source_text = segmenter::wrap_balanced_lines(
                &cue.source_text,
                normalized.max_chars_per_line,
                normalized.max_lines,
            );
            if !cue.translated_text.trim().is_empty() {
                cue.translated_text = segmenter::wrap_balanced_lines(
                    &cue.translated_text,
                    normalized.max_chars_per_line,
                    normalized.max_lines,
                );
            }
            segmenter::compute_cue_metrics(cue, &normalized);
        }
    }

    project.options.segment_rules = normalized;
    project.updated_at = chrono::Utc::now().to_rfc3339();
    Ok(project)
}

pub async fn translate_project(
    app: &AppHandle,
    state: &PipelineState,
    mut project: SubtitleProject,
) -> Result<SubtitleProject> {
    let job_id = project.job_id.clone();
    let cancel_flag = state.register_job(&job_id).await;
    let res =
        run_project_llm_stage(app, &mut project, BatchOperation::Translate, &cancel_flag).await;
    state.finish_job(&job_id).await;
    res?;

    emit_job_progress(
        app,
        &job_id,
        JobPhase::Completed,
        100.0,
        "Translation completed.",
    );
    Ok(project)
}

pub async fn polish_project(
    app: &AppHandle,
    state: &PipelineState,
    mut project: SubtitleProject,
) -> Result<SubtitleProject> {
    let job_id = project.job_id.clone();
    let cancel_flag = state.register_job(&job_id).await;
    let res =
        run_project_llm_stage(app, &mut project, BatchOperation::PolishSerbian, &cancel_flag).await;
    state.finish_job(&job_id).await;
    res?;

    emit_job_progress(
        app,
        &job_id,
        JobPhase::Completed,
        100.0,
        "Serbian subtitle polishing completed.",
    );
    Ok(project)
}

pub async fn run_full_pipeline(
    app: &AppHandle,
    state: &PipelineState,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
) -> Result<SubtitleProject> {
    let resolved_job_id = job_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let cancel_flag = state.register_job(&resolved_job_id).await;

    let run_res: Result<SubtitleProject> = async {
        let mut project = transcribe_audio_with_flag(
            app,
            &resolved_job_id,
            &audio_path,
            options,
            &cancel_flag,
        )
        .await?;

        let src_lang = project
            .detected_language
            .as_deref()
            .filter(|s| !s.is_empty() && *s != "auto")
            .unwrap_or(&project.options.source_language)
            .to_lowercase();
        let target_lang = project.options.target_language.trim().to_lowercase();

        let is_same_serbian =
            (src_lang == "sr" || src_lang == "hr" || src_lang == "bs") && target_lang == "sr";

        if !target_lang.is_empty() && src_lang != target_lang && !is_same_serbian {
            project.completed_translation_batches = 0;
            run_project_llm_stage(app, &mut project, BatchOperation::Translate, &cancel_flag)
                .await?;
        } else if project.options.polish_serbian && (target_lang == "sr" || src_lang == "sr") {
            project.completed_translation_batches = 0;
            run_project_llm_stage(
                app,
                &mut project,
                BatchOperation::PolishSerbian,
                &cancel_flag,
            )
            .await?;
        } else if target_lang == "sr" || src_lang == "sr" {
            project.cues = convert_cues_script(project.cues, project.options.script)?;
        }

        project.phase = JobPhase::Completed;
        project.error_message = None;
        project.updated_at = chrono::Utc::now().to_rfc3339();
        storage::save_checkpoint(app, &project)?;

        Ok(project)
    }
    .await;

    state.finish_job(&resolved_job_id).await;

    match run_res {
        Ok(project) => {
            emit_job_progress(
                app,
                &resolved_job_id,
                JobPhase::Completed,
                100.0,
                "Full subtitle pipeline completed.",
            );
            Ok(project)
        }
        Err(err) => {
            let phase = if cancel_flag.load(Ordering::SeqCst) {
                JobPhase::Cancelled
            } else {
                JobPhase::Failed
            };
            emit_job_progress(app, &resolved_job_id, phase, 0.0, err.to_string());
            Err(err)
        }
    }
}

pub fn convert_cues_script(
    mut cues: Vec<SubtitleCue>,
    target_script: SerbianScript,
) -> Result<Vec<SubtitleCue>> {
    for cue in &mut cues {
        if !cue.translated_text.trim().is_empty() {
            cue.translated_text = sr_script::convert_script(&cue.translated_text, target_script);
        } else {
            cue.source_text = sr_script::convert_script(&cue.source_text, target_script);
        }
    }
    Ok(cues)
}

pub fn export_subtitles(
    _app: &AppHandle,
    project: SubtitleProject,
    options: ExportOptions,
) -> Result<ExportResult> {
    exporter::export_project_files(&project, &options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SegmentPreset, TimedWord};

    #[test]
    fn resegment_and_convert_cues_script_preserve_content_and_script() {
        let project = SubtitleProject {
            job_id: "job-reseg".to_string(),
            audio_path: "/tmp/test.wav".to_string(),
            audio_file_name: "test.wav".to_string(),
            duration_ms: 3000,
            detected_language: Some("sr".to_string()),
            options: JobOptions {
                script: SerbianScript::Cyrillic,
                ..JobOptions::default()
            },
            words: vec![
                TimedWord {
                    start_ms: 0,
                    end_ms: 400,
                    text: "Ljubav".to_string(),
                    probability: 0.99,
                },
                TimedWord {
                    start_ms: 400,
                    end_ms: 800,
                    text: "i".to_string(),
                    probability: 0.99,
                },
                TimedWord {
                    start_ms: 800,
                    end_ms: 1400,
                    text: "pažnja.".to_string(),
                    probability: 0.99,
                },
            ],
            cues: vec![],
            phase: JobPhase::Completed,
            completed_translation_batches: 0,
            error_message: None,
            updated_at: "2026-11-22T18:00:00Z".to_string(),
        };

        let resegmented = resegment_project(
            project,
            SegmentRules {
                preset: SegmentPreset::Standard,
                ..SegmentRules::default()
            },
        )
        .unwrap();

        assert_eq!(resegmented.cues.len(), 1);
        assert_eq!(resegmented.cues[0].source_text, "Љубав и пажња.");

        let latin_cues = convert_cues_script(resegmented.cues, SerbianScript::Latin).unwrap();
        assert_eq!(latin_cues[0].source_text, "Ljubav i pažnja.");
    }
}
