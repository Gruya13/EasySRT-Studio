//! Subtitle generation pipeline (audio conversion, Whisper transcription,
//! segmentation, local/cloud translation, Serbian polishing, script conversion,
//! and export).

pub mod diarizer;
pub mod exporter;
pub mod ffmpeg;
pub mod importer;
pub mod llama_server;
pub mod probe;
pub mod segmenter;
pub mod sr_script;
pub mod telemetry_diff;
pub mod translator;
pub mod whisper;

use crate::models::{
    AddressForm, AppConfig, CatalogItemKind, ExportOptions, ExportResult, JobMetricsPayload,
    JobOptions, JobPhase, JobProgressEvent, SegmentPreset, SegmentRules, SerbianScript,
    SubtitleCue, SubtitleProject, TranslationProviderKind, TranslationStyle, VramTier,
    EVENT_JOB_PROGRESS,
};
use crate::pipeline::translator::{BatchOperation, LlmEndpointConfig};
use crate::{diagnostics, hardware, setup_manager, storage};
use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex as StdMutex, OnceLock,
};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Mutex};

struct ActiveProbeEntry {
    job_id: String,
    cancel_flag: Arc<AtomicBool>,
    done_rx: watch::Receiver<bool>,
}

/// RAII completion signal held by an active background media intake probe.
/// Dropping it immediately wakes any caller waiting inside `cancel_active_probe` or `begin_probe`.
pub struct ProbeRunGuard {
    done_tx: Option<watch::Sender<bool>>,
}

impl Drop for ProbeRunGuard {
    fn drop(&mut self) {
        if let Some(tx) = self.done_tx.take() {
            let _ = tx.send(true);
        }
    }
}

/// Shared pipeline runtime state registered via `tauri::Builder::manage`.
#[derive(Default)]
pub struct PipelineState {
    active_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    active_probe: Mutex<Option<ActiveProbeEntry>>,
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

    pub async fn has_active_jobs(&self) -> bool {
        !self.active_jobs.lock().await.is_empty()
    }

    /// Registers a new active media probe, preempting and awaiting clean exit of any prior probe
    /// so probes are strictly serialized one at a time without GPU/VRAM overlap.
    pub async fn begin_probe(&self, job_id: &str) -> (Arc<AtomicBool>, ProbeRunGuard) {
        loop {
            let prior_rx = {
                let mut slot = self.active_probe.lock().await;
                if let Some(entry) = slot.as_ref() {
                    if !*entry.done_rx.borrow() {
                        entry.cancel_flag.store(true, Ordering::SeqCst);
                        let rx = entry.done_rx.clone();
                        *slot = None;
                        Some(rx)
                    } else {
                        *slot = None;
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(mut rx) = prior_rx {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    rx.wait_for(|&done| done),
                )
                .await;
                continue;
            }

            let cancel_flag = Arc::new(AtomicBool::new(false));
            let (done_tx, done_rx) = watch::channel(false);
            let mut slot = self.active_probe.lock().await;
            if let Some(entry) = slot.as_ref() {
                if !*entry.done_rx.borrow() {
                    continue;
                }
            }
            *slot = Some(ActiveProbeEntry {
                job_id: job_id.to_string(),
                cancel_flag: Arc::clone(&cancel_flag),
                done_rx,
            });
            return (
                cancel_flag,
                ProbeRunGuard {
                    done_tx: Some(done_tx),
                },
            );
        }
    }

    /// Signals cancellation of the active background media probe (optionally filtered by `job_id`)
    /// and awaits its clean exit so GPU/VRAM is completely free before starting real jobs.
    pub async fn cancel_active_probe(&self, job_id: Option<&str>) -> bool {
        let wait_rx = {
            let mut slot = self.active_probe.lock().await;
            let Some(entry) = slot.as_ref() else {
                return false;
            };
            if *entry.done_rx.borrow() {
                *slot = None;
                return false;
            }
            if let Some(target_id) = job_id {
                if entry.job_id != target_id {
                    return false;
                }
            }
            entry.cancel_flag.store(true, Ordering::SeqCst);
            let rx = entry.done_rx.clone();
            *slot = None;
            Some(rx)
        };

        if let Some(mut rx) = wait_rx {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                rx.wait_for(|&done| done),
            )
            .await;
            return true;
        }
        false
    }
}

#[derive(Debug, Clone)]
struct WhisperStageStats {
    whisper_elapsed_ms: u64,
    diarizer_elapsed_ms: u64,
    whisper_backend: String,
}

type ExportDedupeSet = StdMutex<HashSet<(String, String, &'static str)>>;
static EXPORT_TELEMETRY_DEDUPE: OnceLock<ExportDedupeSet> = OnceLock::new();
static CONDENSED_JOBS: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();

fn detect_whisper_backend(whisper_bin: &Path) -> &'static str {
    if let Some(dir) = whisper_bin.parent() {
        if dir.join("ggml-cuda.dll").is_file() {
            return "cuda";
        }
        if dir.join("ggml-vulkan.dll").is_file() || dir.join("libggml-vulkan.so").is_file() {
            return "vulkan";
        }
    }
    let lowered = whisper_bin.to_string_lossy().to_ascii_lowercase();
    if lowered.contains("cuda") {
        "cuda"
    } else if lowered.contains("vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}

fn gpu_tier_label(tier: VramTier) -> &'static str {
    match tier {
        VramTier::CpuOnly => "cpuOnly",
        VramTier::Vram4Gb => "vram4Gb",
        VramTier::Vram8Gb => "vram8Gb",
        VramTier::Vram12GbPlus => "vram12GbPlus",
    }
}

fn llm_provider_label(provider: TranslationProviderKind) -> &'static str {
    match provider {
        TranslationProviderKind::LocalLlama => "localLlama",
        TranslationProviderKind::DeepSeek => "deepSeek",
        TranslationProviderKind::CustomOpenAi => "customOpenAi",
    }
}

fn translation_style_label(style: TranslationStyle) -> &'static str {
    match style {
        TranslationStyle::Natural => "natural",
        TranslationStyle::Formal => "formal",
    }
}

fn address_form_label(addr: AddressForm) -> &'static str {
    match addr {
        AddressForm::Auto => "auto",
        AddressForm::PoliteVi => "politeVi",
        AddressForm::InformalTi => "informalTi",
    }
}

fn segment_preset_label(preset: SegmentPreset) -> &'static str {
    match preset {
        SegmentPreset::Standard => "standard",
        SegmentPreset::Reels => "reels",
        SegmentPreset::Custom => "custom",
    }
}

fn is_custom_model_id(config: &AppConfig, model_id: &str) -> bool {
    model_id.starts_with("custom")
        || config.custom_models.iter().any(|m| m.id == model_id)
}

fn resolve_whisper_model_label(config: &AppConfig) -> String {
    match config.active_whisper_model_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => {
            if is_custom_model_id(config, id) {
                "custom".to_string()
            } else {
                id.to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

fn resolve_llm_model_label(
    config: &AppConfig,
    provider: TranslationProviderKind,
) -> String {
    match provider {
        TranslationProviderKind::LocalLlama => {
            match config.active_llm_model_id.as_deref().map(str::trim) {
                Some(id) if !id.is_empty() => {
                    if is_custom_model_id(config, id) {
                        "custom".to_string()
                    } else {
                        id.to_string()
                    }
                }
                _ => "unknown".to_string(),
            }
        }
        TranslationProviderKind::DeepSeek => {
            let trimmed = config.translation.deepseek_model.trim();
            if trimmed.is_empty() {
                "deepseek-chat".to_string()
            } else {
                trimmed.to_string()
            }
        }
        TranslationProviderKind::CustomOpenAi => "custom".to_string(),
    }
}

fn save_baseline_sidecar_quiet(app: &AppHandle, project: &SubtitleProject) {
    let baseline = telemetry_diff::snapshot_baseline_cues(&project.cues);
    if let Err(err) = storage::save_baseline_cues(app, &project.job_id, &baseline) {
        tracing::warn!(
            job_id = %project.job_id,
            "Failed to save AI baseline sidecar: {err:#}"
        );
    }
}

fn mark_job_condensed(job_id: &str) {
    let lock = CONDENSED_JOBS.get_or_init(|| StdMutex::new(HashSet::new()));
    let mut set = lock.lock().unwrap_or_else(|e| e.into_inner());
    if set.len() >= 512 {
        set.clear();
    }
    set.insert(job_id.to_string());
}

fn is_job_condensed(job_id: &str) -> bool {
    let lock = CONDENSED_JOBS.get_or_init(|| StdMutex::new(HashSet::new()));
    let set = lock.lock().unwrap_or_else(|e| e.into_inner());
    set.contains(job_id)
}

fn emit_job_metrics_if_consented(
    config: &AppConfig,
    project: &SubtitleProject,
    whisper_stats: Option<&WhisperStageStats>,
    llm_elapsed_ms: u64,
) {
    if cfg!(test)
        || !config.can_share_usage_metrics()
        || diagnostics::resolve_telemetry_endpoint().is_none()
    {
        return;
    }

    let provider = project
        .options
        .provider_override
        .unwrap_or(config.translation.provider);
    let (src_lang, target_lang) = resolved_source_and_target_lang(project);
    let audio_duration_ms = project.duration_ms;
    let whisper_elapsed_ms = whisper_stats.map(|s| s.whisper_elapsed_ms).unwrap_or(0);
    let diarizer_elapsed_ms = whisper_stats.map(|s| s.diarizer_elapsed_ms).unwrap_or(0);
    let whisper_backend = whisper_stats
        .map(|s| s.whisper_backend.clone())
        .unwrap_or_else(|| "none".to_string());
    let asr_rtf = if audio_duration_ms > 0 && whisper_elapsed_ms > 0 {
        whisper_elapsed_ms as f64 / audio_duration_ms as f64
    } else {
        0.0
    };
    let llm_rtf = if audio_duration_ms > 0 && llm_elapsed_ms > 0 {
        llm_elapsed_ms as f64 / audio_duration_ms as f64
    } else {
        0.0
    };

    let whisper_model_id = resolve_whisper_model_label(config);
    let llm_model_id = resolve_llm_model_label(config, provider);
    let cue_count = u32::try_from(project.cues.len()).unwrap_or(u32::MAX);
    let translation_style = translation_style_label(project.options.translation_style).to_string();
    let address_form = address_form_label(project.options.address_form).to_string();
    let preset = segment_preset_label(project.options.segment_rules.preset).to_string();
    let llm_provider = llm_provider_label(provider).to_string();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            let gpu_tier = tokio::task::spawn_blocking(|| {
                gpu_tier_label(hardware::detect_gpu().recommended_tier).to_string()
            })
            .await
            .unwrap_or_else(|_| "cpuOnly".to_string());

            let payload = JobMetricsPayload {
                app_version: diagnostics::current_app_version().to_string(),
                os: std::env::consts::OS.to_string(),
                gpu_tier,
                whisper_backend,
                whisper_model_id,
                llm_provider,
                llm_model_id,
                audio_duration_ms,
                whisper_elapsed_ms,
                diarizer_elapsed_ms,
                llm_elapsed_ms,
                asr_rtf,
                llm_rtf,
                oom_cpu_fallback: false,
                batch_subdivision_retries: 0,
                cue_count,
                source_lang: src_lang,
                target_lang,
                translation_style,
                address_form,
                preset,
            };
            diagnostics::send_job_metrics_async(payload);
        });
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

fn annotate_speaker_changes(wav_path: &Path, whisper_out: &mut whisper::WhisperOutput) {
    match diarizer::detect_speaker_change_boundaries(
        wav_path,
        &whisper_out.words,
        &whisper_out.vad_intervals,
    ) {
        Ok(indices) => {
            for idx in indices {
                if let Some(word) = whisper_out.words.get_mut(idx) {
                    word.speaker_change = true;
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                "Acoustic speaker-turn detection failed for {}: {err:#}",
                wav_path.display()
            );
        }
    }
}

async fn transcribe_audio_with_flag(
    app: &AppHandle,
    job_id: &str,
    audio_path: &str,
    options: JobOptions,
    topic_context: Option<String>,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(SubtitleProject, WhisperStageStats)> {
    let ffmpeg_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::Ffmpeg)?;
    let whisper_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::WhisperBin)?;
    let whisper_backend = detect_whisper_backend(&whisper_bin).to_string();
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

    let clean_topic_context = topic_context
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let whisper_started = Instant::now();
    let whisper_res = whisper::run_whisper_cli_with_topic(
        &whisper_bin,
        &whisper_model,
        vad_model.as_deref(),
        &converted.wav_path,
        &options.source_language,
        &options.glossary,
        clean_topic_context.as_deref(),
        cancel_flag,
        |pct, detail| {
            emit_job_progress(app, job_id, JobPhase::Transcribing, pct, detail);
        },
    )
    .await;
    let whisper_elapsed_ms = whisper_started.elapsed().as_millis() as u64;

    let mut diarizer_elapsed_ms = 0u64;
    let whisper_res = match whisper_res {
        Ok(mut whisper_out) => {
            let diarizer_started = Instant::now();
            annotate_speaker_changes(&converted.wav_path, &mut whisper_out);
            diarizer_elapsed_ms = diarizer_started.elapsed().as_millis() as u64;
            Ok(whisper_out)
        }
        Err(err) => Err(err),
    };

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
    let speaker_change_indices: Vec<usize> = whisper_out
        .words
        .iter()
        .enumerate()
        .filter_map(|(idx, word)| word.speaker_change.then_some(idx))
        .collect();
    let mut cues = segmenter::segment_words_with_options(
        &whisper_out.words,
        &rules,
        &speaker_change_indices,
        rules.split_on_speaker_change,
        options.remove_fillers,
    );

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
        topic_context: clean_topic_context,
        completed_translation_style: None,
        error_message: None,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    storage::save_checkpoint(app, &project)?;

    Ok((
        project,
        WhisperStageStats {
            whisper_elapsed_ms,
            diarizer_elapsed_ms,
            whisper_backend,
        },
    ))
}

fn resolved_source_and_target_lang(project: &SubtitleProject) -> (String, String) {
    let src_lang = project
        .detected_language
        .as_deref()
        .filter(|s| !s.is_empty() && *s != "auto")
        .unwrap_or(&project.options.source_language)
        .trim()
        .to_lowercase();
    let target_lang = project.options.target_language.trim().to_lowercase();
    (src_lang, target_lang)
}

fn determine_llm_operation(project: &SubtitleProject) -> Option<BatchOperation> {
    let (src_lang, target_lang) = resolved_source_and_target_lang(project);
    let is_same_serbian =
        (src_lang == "sr" || src_lang == "hr" || src_lang == "bs") && target_lang == "sr";

    if !target_lang.is_empty()
        && target_lang != "none"
        && src_lang != target_lang
        && !is_same_serbian
    {
        Some(BatchOperation::Translate)
    } else if project.options.polish_serbian
        && (target_lang == "sr" || src_lang == "sr" || src_lang == "hr" || src_lang == "bs")
    {
        Some(BatchOperation::PolishSerbian)
    } else {
        None
    }
}

pub(crate) fn build_local_endpoint(
    guard: &llama_server::LlamaServerGuard,
    config: &crate::models::AppConfig,
) -> LlmEndpointConfig {
    LlmEndpointConfig {
        base_url: guard.base_url().to_string(),
        model: "local-llm".to_string(),
        api_key: Some(guard.api_key().to_string()),
        batch_size: config.translation.batch_size,
    }
}

pub(crate) fn resolve_cloud_endpoint(
    app: &AppHandle,
    config: &crate::models::AppConfig,
    provider: TranslationProviderKind,
) -> Result<LlmEndpointConfig> {
    match provider {
        TranslationProviderKind::DeepSeek => {
            let api_key = storage::get_api_key_with_opt_in(
                app,
                TranslationProviderKind::DeepSeek,
                config.allow_local_encrypted_vault,
            )?
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
            Ok(LlmEndpointConfig {
                base_url: "https://api.deepseek.com/v1".to_string(),
                model,
                api_key: Some(api_key),
                batch_size: config.translation.batch_size,
            })
        }
        TranslationProviderKind::CustomOpenAi => {
            let api_key = storage::get_api_key_with_opt_in(
                app,
                TranslationProviderKind::CustomOpenAi,
                config.allow_local_encrypted_vault,
            )?
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
            let base_url = config.translation.custom_base_url.trim().to_string();
            if base_url.is_empty() {
                return Err(anyhow!(
                    "Custom OpenAI-compatible base URL is not configured in Settings."
                ));
            }
            let validated_url =
                translator::validate_translation_base_url(&base_url, api_key.is_some())?;
            let model = if config.translation.custom_model.trim().is_empty() {
                "default".to_string()
            } else {
                config.translation.custom_model.trim().to_string()
            };
            Ok(LlmEndpointConfig {
                base_url: validated_url.as_str().trim_end_matches('/').to_string(),
                model,
                api_key,
                batch_size: config.translation.batch_size,
            })
        }
        TranslationProviderKind::LocalLlama => Err(anyhow!(
            "resolve_cloud_endpoint called with LocalLlama provider"
        )),
    }
}

async fn execute_llm_operation_with_brief(
    app: &AppHandle,
    project: &mut SubtitleProject,
    operation: BatchOperation,
    phase: JobPhase,
    endpoint: &LlmEndpointConfig,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    let job_id = project.job_id.clone();
    let remove_fillers = project.options.remove_fillers;
    let mut merged_glossary: Option<std::collections::BTreeMap<String, String>> = None;
    let mut inferred_address: Option<AddressForm> = None;

    if matches!(
        operation,
        BatchOperation::Translate | BatchOperation::PolishSerbian
    ) && project.completed_translation_batches == 0
        && !project.cues.is_empty()
    {
        let (eff_src, _) = resolved_source_and_target_lang(project);
        match translator::build_translation_context_brief(
            endpoint,
            &project.cues,
            &eff_src,
            &project.options.target_language,
            project.topic_context.as_deref(),
            cancel_flag,
        )
        .await
        {
            Ok(brief) => {
                if project.topic_context.is_none() && brief.topic.is_some() {
                    project.topic_context = brief.topic;
                }
                if !brief.domain_glossary.is_empty() {
                    merged_glossary = Some(translator::merge_glossaries(
                        &project.options.glossary,
                        &brief.domain_glossary,
                    ));
                }
                if project.options.address_form == AddressForm::Auto {
                    inferred_address = brief.suggested_address;
                }
            }
            Err(err) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    return Err(err);
                }
                tracing::warn!(
                    job_id = %job_id,
                    "Pre-translation domain brief skipped due to error: {err:#}"
                );
            }
        }
    }

    let topic_context = project.topic_context.clone();
    translator::run_batch_operation_with_overrides(
        project,
        operation,
        endpoint,
        cancel_flag,
        |proj| storage::save_checkpoint(app, proj),
        |pct, detail| {
            emit_job_progress(app, &job_id, phase, pct, detail);
        },
        remove_fillers,
        topic_context.as_deref(),
        merged_glossary.as_ref(),
        inferred_address,
    )
    .await
}

async fn run_project_llm_stage_with_asr_stats(
    app: &AppHandle,
    project: &mut SubtitleProject,
    operation: BatchOperation,
    cancel_flag: &Arc<AtomicBool>,
    asr_stats: Option<&WhisperStageStats>,
) -> Result<()> {
    let config = storage::load_config(app)?;
    let provider = project
        .options
        .provider_override
        .unwrap_or(config.translation.provider);

    let phase = match operation {
        BatchOperation::Translate => JobPhase::Translating,
        BatchOperation::PolishSerbian | BatchOperation::CondenseCps => JobPhase::Polishing,
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
            BatchOperation::CondenseCps => "Preparing subtitle CPS condensing...",
        },
    );

    let llm_started = Instant::now();
    let mut llama_guard = None;
    let op_res: Result<()> = async {
        let endpoint = match provider {
            TranslationProviderKind::LocalLlama => {
                let llama_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::LlamaBin)?;
                let llm_model = setup_manager::resolve_active_llm_model_path(app)?;
                let storage_paths = storage::resolve_storage_paths(app, None)?;
                let temp_dir = Path::new(&storage_paths.data_dir).join("temp");
                emit_job_progress(
                    app,
                    &job_id,
                    phase,
                    5.0,
                    "Starting local llama-server...",
                );
                let guard = llama_server::LlamaServerGuard::start_in_log_dir(
                    &llama_bin,
                    &llm_model,
                    config.translation.context_window,
                    cancel_flag,
                    &temp_dir,
                )
                .await?;
                let ep = build_local_endpoint(&guard, &config);
                llama_guard = Some(guard);
                ep
            }
            other => resolve_cloud_endpoint(app, &config, other)?,
        };

        execute_llm_operation_with_brief(app, project, operation, phase, &endpoint, cancel_flag).await
    }
    .await;
    let llm_elapsed_ms = llm_started.elapsed().as_millis() as u64;

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
            if matches!(
                operation,
                BatchOperation::Translate | BatchOperation::PolishSerbian
            ) {
                save_baseline_sidecar_quiet(app, project);
            } else if operation == BatchOperation::CondenseCps {
                mark_job_condensed(&project.job_id);
            }
            emit_job_metrics_if_consented(&config, project, asr_stats, llm_elapsed_ms);
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

async fn run_project_llm_stage(
    app: &AppHandle,
    project: &mut SubtitleProject,
    operation: BatchOperation,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<()> {
    run_project_llm_stage_with_asr_stats(app, project, operation, cancel_flag, None).await
}

pub fn import_subtitle_file(
    app: &AppHandle,
    subtitle_path: String,
    options: JobOptions,
) -> Result<SubtitleProject> {
    let project = importer::import_subtitle_path(Path::new(&subtitle_path), &options)?;
    storage::save_checkpoint(app, &project)?;
    emit_job_progress(
        app,
        &project.job_id,
        JobPhase::Completed,
        100.0,
        "Subtitle file imported.",
    );
    Ok(project)
}

pub async fn transcribe_audio(
    app: &AppHandle,
    state: &PipelineState,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
    topic_context: Option<String>,
) -> Result<SubtitleProject> {
    let _ = state.cancel_active_probe(None).await;

    let resolved_job_id = job_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let cancel_flag = state.register_job(&resolved_job_id).await;
    let res = transcribe_audio_with_flag(
        app,
        &resolved_job_id,
        &audio_path,
        options,
        topic_context,
        &cancel_flag,
    )
    .await;
    state.finish_job(&resolved_job_id).await;

    match res {
        Ok((project, stats)) => {
            save_baseline_sidecar_quiet(app, &project);
            if let Ok(config) = storage::load_config(app) {
                emit_job_metrics_if_consented(&config, &project, Some(&stats), 0);
            }
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
        let speaker_change_indices: Vec<usize> = project
            .words
            .iter()
            .enumerate()
            .filter_map(|(idx, word)| word.speaker_change.then_some(idx))
            .collect();
        let mut new_cues = segmenter::segment_words_with_options(
            &project.words,
            &normalized,
            &speaker_change_indices,
            normalized.split_on_speaker_change,
            project.options.remove_fillers,
        );
        let detected_or_source = project
            .detected_language
            .as_deref()
            .unwrap_or(&project.options.source_language);
        if detected_or_source.eq_ignore_ascii_case("sr") {
            new_cues = convert_cues_script(new_cues, project.options.script)?;
        }
        project.cues = new_cues;
        project.completed_translation_batches = 0;
        project.completed_translation_style = None;
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
            for variant_text in cue.translated_variants.values_mut() {
                if !variant_text.trim().is_empty() {
                    *variant_text = segmenter::wrap_balanced_lines(
                        variant_text,
                        normalized.max_chars_per_line,
                        normalized.max_lines,
                    );
                }
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
    let _ = state.cancel_active_probe(None).await;

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
    let _ = state.cancel_active_probe(None).await;

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

pub async fn condense_project_cps(
    app: &AppHandle,
    state: &PipelineState,
    mut project: SubtitleProject,
) -> Result<SubtitleProject> {
    let _ = state.cancel_active_probe(None).await;

    let job_id = project.job_id.clone();
    let cancel_flag = state.register_job(&job_id).await;
    let res =
        run_project_llm_stage(app, &mut project, BatchOperation::CondenseCps, &cancel_flag).await;
    state.finish_job(&job_id).await;
    res?;

    emit_job_progress(
        app,
        &job_id,
        JobPhase::Completed,
        100.0,
        "Subtitle CPS condensing completed.",
    );
    Ok(project)
}

pub async fn run_full_pipeline(
    app: &AppHandle,
    state: &PipelineState,
    audio_path: String,
    options: JobOptions,
    job_id: Option<String>,
    topic_context: Option<String>,
) -> Result<SubtitleProject> {
    let _ = state.cancel_active_probe(None).await;

    let resolved_job_id = job_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let cancel_flag = state.register_job(&resolved_job_id).await;

    let run_res: Result<SubtitleProject> = async {
        let (mut project, asr_stats) = transcribe_audio_with_flag(
            app,
            &resolved_job_id,
            &audio_path,
            options,
            topic_context,
            &cancel_flag,
        )
        .await?;

        match determine_llm_operation(&project) {
            Some(operation) => {
                project.completed_translation_batches = 0;
                project.completed_translation_style = None;
                run_project_llm_stage_with_asr_stats(
                    app,
                    &mut project,
                    operation,
                    &cancel_flag,
                    Some(&asr_stats),
                )
                .await?;
            }
            None => {
                let (src_lang, target_lang) = resolved_source_and_target_lang(&project);
                if target_lang == "sr" || src_lang == "sr" {
                    project.cues = convert_cues_script(project.cues, project.options.script)?;
                }
                project.phase = JobPhase::Completed;
                project.error_message = None;
                project.updated_at = chrono::Utc::now().to_rfc3339();
                storage::save_checkpoint(app, &project)?;
                save_baseline_sidecar_quiet(app, &project);
                if let Ok(config) = storage::load_config(app) {
                    emit_job_metrics_if_consented(&config, &project, Some(&asr_stats), 0);
                }
                return Ok(project);
            }
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

/// Phase 2 of the offline batch pipeline: starts `llama-server` ONCE (when effective
/// provider is `LocalLlama`), runs translation / Serbian polishing across all `projects`
/// against the single warm server, checkpoints each project, and stops `llama-server`
/// once at the end (even on error).
pub async fn run_local_llm_batch(
    app: &AppHandle,
    state: &PipelineState,
    mut projects: Vec<SubtitleProject>,
) -> Result<Vec<SubtitleProject>> {
    let _ = state.cancel_active_probe(None).await;

    if projects.is_empty() {
        return Ok(projects);
    }

    let config = storage::load_config(app)?;

    let mut project_flags: Vec<(String, Arc<AtomicBool>)> = Vec::with_capacity(projects.len());
    for project in &projects {
        let flag = state.register_job(&project.job_id).await;
        project_flags.push((project.job_id.clone(), flag));
    }

    let mut llama_guard: Option<llama_server::LlamaServerGuard> = None;

    let batch_res: Result<()> = async {
        let needs_local_llama = projects.iter().any(|p| {
            let provider = p
                .options
                .provider_override
                .unwrap_or(config.translation.provider);
            provider == TranslationProviderKind::LocalLlama
                && determine_llm_operation(p).is_some()
        });

        let local_endpoint = if needs_local_llama {
            let llama_bin = setup_manager::resolve_binary_path(app, CatalogItemKind::LlamaBin)?;
            let llm_model = setup_manager::resolve_active_llm_model_path(app)?;
            let storage_paths = storage::resolve_storage_paths(app, None)?;
            let temp_dir = Path::new(&storage_paths.data_dir).join("temp");

            for project in &projects {
                if let Some(op) = determine_llm_operation(project) {
                    let phase = match op {
                        BatchOperation::Translate => JobPhase::Translating,
                        BatchOperation::PolishSerbian | BatchOperation::CondenseCps => {
                            JobPhase::Polishing
                        }
                    };
                    emit_job_progress(
                        app,
                        &project.job_id,
                        phase,
                        3.0,
                        "Starting local llama-server...",
                    );
                }
            }

            let startup_cancel = Arc::new(AtomicBool::new(false));
            let startup_cancel_clone = Arc::clone(&startup_cancel);
            let flags_to_watch: Vec<Arc<AtomicBool>> =
                project_flags.iter().map(|(_, f)| Arc::clone(f)).collect();
            let watcher = tokio::spawn(async move {
                loop {
                    if flags_to_watch.iter().any(|f| f.load(Ordering::SeqCst)) {
                        startup_cancel_clone.store(true, Ordering::SeqCst);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            });

            let guard_res = llama_server::LlamaServerGuard::start_in_log_dir(
                &llama_bin,
                &llm_model,
                config.translation.context_window,
                &startup_cancel,
                &temp_dir,
            )
            .await;
            watcher.abort();
            let guard = guard_res?;
            let ep = build_local_endpoint(&guard, &config);
            llama_guard = Some(guard);
            Some(ep)
        } else {
            None
        };

        for (idx, project) in projects.iter_mut().enumerate() {
            let job_id = project.job_id.clone();
            let cancel_flag = &project_flags[idx].1;

            if cancel_flag.load(Ordering::SeqCst) {
                project.phase = JobPhase::Cancelled;
                project.error_message = Some("Job cancelled".to_string());
                project.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = storage::save_checkpoint(app, project);
                emit_job_progress(app, &job_id, JobPhase::Cancelled, 0.0, "Job cancelled");
                return Err(anyhow!("Job cancelled"));
            }

            let op = determine_llm_operation(project);
            if let Some(operation) = op {
                let phase = match operation {
                    BatchOperation::Translate => JobPhase::Translating,
                    BatchOperation::PolishSerbian | BatchOperation::CondenseCps => {
                        JobPhase::Polishing
                    }
                };
                let provider = project
                    .options
                    .provider_override
                    .unwrap_or(config.translation.provider);
                let endpoint = match provider {
                    TranslationProviderKind::LocalLlama => local_endpoint
                        .clone()
                        .ok_or_else(|| anyhow!("Local llama-server endpoint unavailable"))?,
                    other => resolve_cloud_endpoint(app, &config, other)?,
                };

                project.phase = phase;
                project.error_message = None;
                project.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = storage::save_checkpoint(app, project);

                emit_job_progress(
                    app,
                    &job_id,
                    phase,
                    5.0,
                    match operation {
                        BatchOperation::Translate => "Translating subtitles...",
                        BatchOperation::PolishSerbian => "Polishing Serbian subtitles...",
                        BatchOperation::CondenseCps => "Condensing subtitle CPS...",
                    },
                );

                let llm_started = Instant::now();
                let res = execute_llm_operation_with_brief(
                    app,
                    project,
                    operation,
                    phase,
                    &endpoint,
                    cancel_flag,
                )
                .await;
                let llm_elapsed_ms = llm_started.elapsed().as_millis() as u64;

                if let Err(err) = res {
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
                    return Err(err);
                }

                if matches!(
                    operation,
                    BatchOperation::Translate | BatchOperation::PolishSerbian
                ) {
                    save_baseline_sidecar_quiet(app, project);
                } else if operation == BatchOperation::CondenseCps {
                    mark_job_condensed(&project.job_id);
                }
                emit_job_metrics_if_consented(&config, project, None, llm_elapsed_ms);
            } else {
                let (src_lang, target_lang) = resolved_source_and_target_lang(project);
                if target_lang == "sr" || src_lang == "sr" {
                    project.cues = convert_cues_script(
                        std::mem::take(&mut project.cues),
                        project.options.script,
                    )?;
                }
            }

            project.phase = JobPhase::Completed;
            project.error_message = None;
            project.updated_at = chrono::Utc::now().to_rfc3339();
            storage::save_checkpoint(app, project)?;
            emit_job_progress(
                app,
                &job_id,
                JobPhase::Completed,
                100.0,
                match op {
                    Some(BatchOperation::PolishSerbian) => "Serbian subtitle polishing completed.",
                    _ => "Translation completed.",
                },
            );
        }

        Ok(())
    }
    .await;

    // Guarantee local llama-server is stopped once at the end (even on error)
    if let Some(mut guard) = llama_guard.take() {
        guard.stop();
    }

    for (job_id, _) in project_flags {
        state.finish_job(&job_id).await;
    }

    batch_res?;
    Ok(projects)
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
        sr_script::convert_script_values(cue.translated_variants.values_mut(), target_script);
    }
    Ok(cues)
}

pub fn compute_cues_export_hash(cues: &[SubtitleCue]) -> String {
    let mut hasher = Sha256::new();
    for cue in cues {
        hasher.update(cue.id.to_le_bytes());
        hasher.update(cue.start_ms.to_le_bytes());
        hasher.update(cue.end_ms.to_le_bytes());
        hasher.update(cue.source_text.as_bytes());
        hasher.update(b"\0");
        hasher.update(cue.translated_text.as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

fn mark_export_telemetry_channel(job_id: &str, cue_hash: &str, channel: &'static str) -> bool {
    let lock = EXPORT_TELEMETRY_DEDUPE.get_or_init(|| StdMutex::new(HashSet::new()));
    let mut set = lock.lock().unwrap_or_else(|e| e.into_inner());
    if set.len() >= 512 {
        set.clear();
    }
    set.insert((job_id.to_string(), cue_hash.to_string(), channel))
}

pub fn get_project_modified_cue_count(app: &AppHandle, job_id: &str) -> Result<usize> {
    storage::validate_job_id(job_id)?;
    let baseline = storage::load_baseline_cues(app, job_id)?;
    if baseline.is_empty() {
        return Ok(0);
    }
    let project = match storage::load_checkpoint(app, job_id) {
        Ok(p) => p,
        Err(_) => return Ok(0),
    };
    Ok(telemetry_diff::count_modified_training_cues(
        &baseline,
        &project.cues,
    ))
}

pub fn export_subtitles(
    app: &AppHandle,
    project: SubtitleProject,
    options: ExportOptions,
    skip_training_donation: Option<bool>,
) -> Result<ExportResult> {
    let result = exporter::export_project_files(&project, &options)?;

    if cfg!(test) || diagnostics::resolve_telemetry_endpoint().is_none() {
        return Ok(result);
    }

    let Ok(mut config) = storage::load_config(app) else {
        return Ok(result);
    };
    let share_metrics = config.can_share_usage_metrics();
    let share_training =
        config.can_share_translation_edits() && !skip_training_donation.unwrap_or(false);
    if !share_metrics && !share_training {
        return Ok(result);
    }

    let Ok(baseline) = storage::load_baseline_cues(app, &project.job_id) else {
        return Ok(result);
    };
    if baseline.is_empty() {
        return Ok(result);
    }

    let cue_hash = compute_cues_export_hash(&project.cues);
    let send_metrics =
        share_metrics && mark_export_telemetry_channel(&project.job_id, &cue_hash, "export");
    let send_training =
        share_training && mark_export_telemetry_channel(&project.job_id, &cue_hash, "training");
    if !send_metrics && !send_training {
        return Ok(result);
    }

    if send_metrics && diagnostics::ensure_monthly_cohort_salt(&mut config) {
        let salt = config.telemetry_cohort_salt.clone();
        let month = config.telemetry_cohort_month.clone();
        let _ = storage::update_config(app, move |cfg| {
            cfg.telemetry_cohort_salt = salt;
            cfg.telemetry_cohort_month = month;
        });
    }

    let report = telemetry_diff::analyze_export_diff(
        &baseline,
        &project.cues,
        &project.options,
        &config.telemetry_cohort_salt,
        send_metrics,
        send_training,
        is_job_condensed(&project.job_id),
    );

    if send_metrics {
        let (src_lang, target_lang) = resolved_source_and_target_lang(&project);
        diagnostics::send_export_telemetry_async(diagnostics::WorkerExportTelemetryPayload {
            app_version: diagnostics::current_app_version().to_string(),
            os: std::env::consts::OS.to_string(),
            source_lang: src_lang,
            target_lang,
            translation_style: translation_style_label(project.options.translation_style)
                .to_string(),
            address_form: address_form_label(project.options.address_form).to_string(),
            metrics: report.metrics,
            ngram_diffs: report.ngram_diffs,
        });
    }

    if send_training && !report.training_pairs.is_empty() {
        diagnostics::send_training_pairs_async(report.training_pairs);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExportFormat, SegmentPreset, TimedWord, TranslationStyle};
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::process::Command;

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
                    speaker_change: false,
                },
                TimedWord {
                    start_ms: 400,
                    end_ms: 800,
                    text: "i".to_string(),
                    probability: 0.99,
                    speaker_change: false,
                },
                TimedWord {
                    start_ms: 800,
                    end_ms: 1400,
                    text: "pažnja.".to_string(),
                    probability: 0.99,
                    speaker_change: false,
                },
            ],
            cues: vec![],
            phase: JobPhase::Completed,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn full_pipeline_end_to_end_ffmpeg_whisper_segment_translate_polish_export() {
        let ffmpeg_path = Path::new("/usr/bin/ffmpeg");
        assert!(
            ffmpeg_path.exists(),
            "/usr/bin/ffmpeg is required for the full end-to-end pipeline test"
        );

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_e2e_pipeline_{}",
            uuid::Uuid::new_v4()
        ));
        let data_dir = temp_dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let src_audio = temp_dir.join("epizoda_test.wav");
        let converted_wav = temp_dir.join("epizoda_test.16k.wav");

        // 1. Generate a real synthetic 2-second audio file via /usr/bin/ffmpeg
        let gen_status = Command::new(ffmpeg_path)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100:duration=2",
                "-ac",
                "2",
                "-y",
            ])
            .arg(&src_audio)
            .status()
            .await
            .unwrap();
        assert!(gen_status.success());

        // 2. Run ffmpeg::convert_to_16khz_mono_wav
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let converted = ffmpeg::convert_to_16khz_mono_wav(
            ffmpeg_path,
            &src_audio,
            &converted_wav,
            &cancel_flag,
        )
        .await
        .unwrap();
        assert!(converted.wav_path.exists());
        assert!(
            (1950..=2050).contains(&converted.duration_ms),
            "expected ~2000ms duration, got {}ms",
            converted.duration_ms
        );

        // 3. Create a temporary executable mock `whisper-cli` shell script that parses `-of <prefix>`
        // and writes a realistic multi-token `-ojf` `<prefix>.json` file.
        let mock_model = temp_dir.join("ggml-large-v3-turbo.bin");
        fs::write(&mock_model, b"mock-whisper-model").unwrap();

        let mock_whisper = temp_dir.join("mock-whisper-cli.sh");
        let whisper_script = r#"#!/usr/bin/env sh
OUT_PREFIX=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -of)
      OUT_PREFIX="$2"
      shift 2
      ;;
    *)
      shift 1
      ;;
  esac
done
if [ -z "$OUT_PREFIX" ]; then
  echo "Missing -of argument" >&2
  exit 1
fi
cat << 'EOF' > "${OUT_PREFIX}.json"
{
  "result": { "language": "en" },
  "transcription": [
    {
      "offsets": { "from": 0, "to": 1300 },
      "text": " Good morning everyone.",
      "tokens": [
        { "text": "[_BEG_]", "offsets": { "from": 0, "to": 0 }, "p": 0.99 },
        { "text": " Good", "offsets": { "from": 100, "to": 400 }, "p": 0.98 },
        { "text": " morning", "offsets": { "from": 420, "to": 780 }, "p": 0.97 },
        { "text": " every", "offsets": { "from": 800, "to": 1020 }, "p": 0.95 },
        { "text": "one", "offsets": { "from": 1020, "to": 1220 }, "p": 0.96 },
        { "text": ".", "offsets": { "from": 1220, "to": 1280 }, "p": 0.99 }
      ]
    },
    {
      "offsets": { "from": 1450, "to": 3800 },
      "text": " Welcome to Belgrade and our new subtitle studio today!",
      "tokens": [
        { "text": "[_BEG_]", "offsets": { "from": 1450, "to": 1450 }, "p": 0.99 },
        { "text": " Welcome", "offsets": { "from": 1500, "to": 1850 }, "p": 0.97 },
        { "text": " to", "offsets": { "from": 1860, "to": 2000 }, "p": 0.99 },
        { "text": " Belgrade", "offsets": { "from": 2020, "to": 2450 }, "p": 0.96 },
        { "text": " and", "offsets": { "from": 2470, "to": 2650 }, "p": 0.98 },
        { "text": " our", "offsets": { "from": 2670, "to": 2850 }, "p": 0.98 },
        { "text": " new", "offsets": { "from": 2870, "to": 3050 }, "p": 0.97 },
        { "text": " subtitle", "offsets": { "from": 3070, "to": 3400 }, "p": 0.95 },
        { "text": " studio!", "offsets": { "from": 3420, "to": 3750 }, "p": 0.96 }
      ]
    }
  ]
}
EOF
"#;
        {
            let mut file = fs::File::create(&mock_whisper).unwrap();
            file.write_all(whisper_script.as_bytes()).unwrap();
            file.sync_all().unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&mock_whisper, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Small pause to avoid Linux ETXTBSY race on newly closed executable fd
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        let mut glossary = BTreeMap::new();
        glossary.insert("Belgrade".to_string(), "Београд".to_string());
        glossary.insert("subtitle studio".to_string(), "студио за титлове".to_string());

        let mut whisper_progress_calls = 0usize;
        let whisper_out = whisper::run_whisper_cli(
            &mock_whisper,
            &mock_model,
            None,
            &converted.wav_path,
            "auto",
            &glossary,
            &cancel_flag,
            |_pct, _detail| {
                whisper_progress_calls += 1;
            },
        )
        .await
        .unwrap();

        assert!(whisper_progress_calls >= 2);
        assert_eq!(whisper_out.detected_language.as_deref(), Some("en"));
        assert_eq!(whisper_out.words.len(), 11);
        assert_eq!(whisper_out.words[2].text, "everyone.");

        // 4. Segment words verifying both `Standard` and `Reels` presets
        let standard_rules = segmenter::normalize_rules(&SegmentRules {
            preset: SegmentPreset::Standard,
            ..SegmentRules::default()
        });
        let standard_cues = segmenter::segment_words(&whisper_out.words, &standard_rules);
        assert_eq!(standard_cues.len(), 2);
        assert_eq!(standard_cues[0].source_text, "Good morning everyone.");
        // Cue 2 is 48 chars (> 42 chars per line), so Standard preset wraps it into 2 balanced lines
        assert_eq!(standard_cues[1].source_text.lines().count(), 2);

        let reels_rules = segmenter::normalize_rules(&SegmentRules {
            preset: SegmentPreset::Reels,
            ..SegmentRules::default()
        });
        let reels_cues = segmenter::segment_words(&whisper_out.words, &reels_rules);
        assert!(
            reels_cues.len() > standard_cues.len(),
            "Reels preset should produce more shorter cues than Standard preset"
        );
        for rc in &reels_cues {
            assert_eq!(rc.source_text.lines().count(), 1);
            assert!(rc.source_text.chars().count() <= 20);
        }

        // 5. Spin up a local mock OpenAI-compatible TcpListener server and run both
        //    BatchOperation::Translate (EN -> SR with glossary and script conversion)
        //    and BatchOperation::PolishSerbian (with checkpoint callback verification).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let http_calls = Arc::new(AtomicUsize::new(0));
        let http_calls_clone = Arc::clone(&http_calls);

        let server_task = tokio::spawn(async move {
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
                    let body_part = req_str.split("\r\n\r\n").nth(1).unwrap_or("");
                    let req_json: serde_json::Value =
                        serde_json::from_str(body_part).unwrap_or_default();
                    let sys_prompt = req_json["messages"][0]["content"]
                        .as_str()
                        .unwrap_or("");
                    let user_content = req_json["messages"][1]["content"]
                        .as_str()
                        .unwrap_or("{}");
                    let prompt_payload: serde_json::Value =
                        serde_json::from_str(user_content).unwrap_or_default();
                    let translate_cues = prompt_payload["translate_cues"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();

                    let is_polish = sys_prompt.contains("Serbian subtitle editor");

                    let items: Vec<serde_json::Value> = translate_cues
                        .iter()
                        .map(|c| {
                            let id = c["id"].as_u64().unwrap_or(1);
                            let text = if is_polish {
                                match id {
                                    1 => "Dobro jutro svima, lepo vreme danas.",
                                    2 => "Dobrodošli u Beograd i naš novi studio za titlove!",
                                    _ => "Ispoliran titl.",
                                }
                            } else {
                                match id {
                                    1 => "Dobro jutro svima, lijepo vrijeme danas.",
                                    2 => "Dobrodošli u Beograd i naš novi studio za titlove!",
                                    _ => "Preveden titl.",
                                }
                            };
                            serde_json::json!({ "id": id, "text": text })
                        })
                        .collect();

                    let llm_content = format!(
                        "<think>Processing batch</think>\n```json\n{}\n```",
                        serde_json::to_string(&items).unwrap()
                    );
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
            job_id: "e2e-job-001".to_string(),
            audio_path: src_audio.to_string_lossy().to_string(),
            audio_file_name: "epizoda_test.wav".to_string(),
            duration_ms: converted.duration_ms,
            detected_language: whisper_out.detected_language.clone(),
            options: JobOptions {
                source_language: "en".to_string(),
                target_language: "sr".to_string(),
                script: SerbianScript::Cyrillic,
                segment_rules: standard_rules.clone(),
                glossary,
                polish_serbian: true,
                correct_source_asr: false,
                translation_style: TranslationStyle::default(),
                address_form: AddressForm::Auto,
                remove_fillers: true,
                provider_override: Some(TranslationProviderKind::CustomOpenAi),
            },
            words: whisper_out.words,
            cues: standard_cues,
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        let endpoint = LlmEndpointConfig {
            base_url: format!("http://127.0.0.1:{}/v1", addr.port()),
            model: "mock-qwen".to_string(),
            api_key: Some("e2e-key".to_string()),
            batch_size: 2,
        };

        let mut translate_checkpoints = 0usize;
        translator::run_batch_operation(
            &mut project,
            BatchOperation::Translate,
            &endpoint,
            &cancel_flag,
            |proj| {
                translate_checkpoints += 1;
                storage::save_checkpoint_to_data_dir(&data_dir, proj)
            },
            |_pct, _detail| {},
        )
        .await
        .unwrap();

        assert_eq!(translate_checkpoints, 1);
        assert_eq!(project.completed_translation_batches, 1);
        assert!(project.cues[0].translated_text.contains("Добро јутро свима"));
        assert!(project.cues[1].translated_text.contains("Београд"));
        // Cue 2 translated text is > 42 chars, so it must be wrapped into 2 balanced lines
        assert_eq!(project.cues[1].translated_text.lines().count(), 2);

        // Now run BatchOperation::PolishSerbian (normalizing Ijekavian "lijepo vrijeme" -> Ekavian "lepo vreme")
        for cue in &mut project.cues {
            cue.source_text = cue.translated_text.clone();
            cue.translated_text.clear();
        }
        project.completed_translation_batches = 0;
        project.phase = JobPhase::Polishing;

        let mut polish_checkpoints = 0usize;
        translator::run_batch_operation(
            &mut project,
            BatchOperation::PolishSerbian,
            &endpoint,
            &cancel_flag,
            |proj| {
                polish_checkpoints += 1;
                storage::save_checkpoint_to_data_dir(&data_dir, proj)
            },
            |_pct, _detail| {},
        )
        .await
        .unwrap();

        server_task.abort();

        assert_eq!(polish_checkpoints, 1);
        assert_eq!(http_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            project.cues[0].translated_text,
            "Добро јутро свима, лепо време данас."
        );

        // Verify checkpoint was persisted to disk and loads back identically
        let loaded_checkpoint =
            storage::load_checkpoint_from_data_dir(&data_dir, &project.job_id).unwrap();
        assert_eq!(loaded_checkpoint.cues, project.cues);

        // Restore original English source_text before dual export (.sr.srt + .en.srt)
        project.cues[0].source_text = "Good morning everyone.".to_string();
        project.cues[1].source_text =
            "Welcome to Belgrade and\nour new subtitle studio!".to_string();

        // 6. Export `.srt` (with UTF-8 BOM and `include_source_srt: true`), `.vtt`, and `.txt`
        let export_opts = ExportOptions {
            output_dir: Some(temp_dir.to_string_lossy().to_string()),
            save_beside_audio: true,
            include_source_srt: true,
            formats: vec![ExportFormat::Srt, ExportFormat::Vtt, ExportFormat::Txt],
            utf8_bom: true,
        };
        let export_res = exporter::export_project_files(&project, &export_opts).unwrap();
        assert_eq!(export_res.written_files.len(), 4);

        let sr_srt_path = temp_dir.join("epizoda_test.sr.srt");
        let sr_vtt_path = temp_dir.join("epizoda_test.sr.vtt");
        let sr_txt_path = temp_dir.join("epizoda_test.sr.txt");
        let en_srt_path = temp_dir.join("epizoda_test.en.srt");

        assert!(sr_srt_path.is_file());
        assert!(sr_vtt_path.is_file());
        assert!(sr_txt_path.is_file());
        assert!(en_srt_path.is_file());

        let sr_srt_raw = fs::read_to_string(&sr_srt_path).unwrap();
        let sr_vtt_raw = fs::read_to_string(&sr_vtt_path).unwrap();
        let sr_txt_raw = fs::read_to_string(&sr_txt_path).unwrap();
        let en_srt_raw = fs::read_to_string(&en_srt_path).unwrap();

        assert!(sr_srt_raw.starts_with('\u{FEFF}'));
        assert!(sr_vtt_raw.starts_with("\u{FEFF}WEBVTT"));
        assert!(sr_txt_raw.contains("Добро јутро свима, лепо време данас."));

        // 7. Re-import exported `.srt` and `.vtt` using `importer::parse_subtitle_content`
        //    and prove 100% round-trip fidelity!
        let reimported_srt =
            importer::parse_subtitle_content(&sr_srt_raw, &standard_rules).unwrap();
        let reimported_vtt =
            importer::parse_subtitle_content(&sr_vtt_raw, &standard_rules).unwrap();
        let reimported_en_srt =
            importer::parse_subtitle_content(&en_srt_raw, &standard_rules).unwrap();

        assert_eq!(reimported_srt.len(), project.cues.len());
        assert_eq!(reimported_vtt.len(), project.cues.len());
        assert_eq!(reimported_en_srt.len(), project.cues.len());

        for i in 0..project.cues.len() {
            let orig = &project.cues[i];
            let srt_cue = &reimported_srt[i];
            let vtt_cue = &reimported_vtt[i];
            let en_cue = &reimported_en_srt[i];

            assert_eq!(srt_cue.id, orig.id);
            assert_eq!(srt_cue.start_ms, orig.start_ms);
            assert_eq!(srt_cue.end_ms, orig.end_ms);
            assert_eq!(srt_cue.source_text, orig.translated_text);
            assert_eq!(srt_cue.cps, orig.cps);
            assert_eq!(srt_cue.warnings, orig.warnings);

            assert_eq!(vtt_cue.id, orig.id);
            assert_eq!(vtt_cue.start_ms, orig.start_ms);
            assert_eq!(vtt_cue.end_ms, orig.end_ms);
            assert_eq!(vtt_cue.source_text, orig.translated_text);
            assert_eq!(vtt_cue.cps, orig.cps);
            assert_eq!(vtt_cue.warnings, orig.warnings);

            assert_eq!(en_cue.id, orig.id);
            assert_eq!(en_cue.start_ms, orig.start_ms);
            assert_eq!(en_cue.end_ms, orig.end_ms);
            assert_eq!(en_cue.source_text, orig.source_text);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn determines_llm_operation_for_translate_polish_and_none() {
        let base = |detected: Option<&str>, src: &str, dst: &str, polish: bool| SubtitleProject {
            job_id: "op-test".to_string(),
            audio_path: "/tmp/a.mp3".to_string(),
            audio_file_name: "a.mp3".to_string(),
            duration_ms: 1000,
            detected_language: detected.map(str::to_string),
            options: JobOptions {
                source_language: src.to_string(),
                target_language: dst.to_string(),
                polish_serbian: polish,
                ..JobOptions::default()
            },
            words: vec![],
            cues: vec![],
            phase: JobPhase::Completed,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-22T22:00:00Z".to_string(),
        };

        assert_eq!(
            determine_llm_operation(&base(Some("en"), "auto", "sr", true)),
            Some(BatchOperation::Translate)
        );
        assert_eq!(
            determine_llm_operation(&base(Some("sr"), "auto", "sr", true)),
            Some(BatchOperation::PolishSerbian)
        );
        assert_eq!(
            determine_llm_operation(&base(Some("hr"), "auto", "sr", true)),
            Some(BatchOperation::PolishSerbian)
        );
        assert_eq!(
            determine_llm_operation(&base(Some("sr"), "sr", "none", true)),
            Some(BatchOperation::PolishSerbian)
        );
        assert_eq!(
            determine_llm_operation(&base(Some("en"), "en", "none", true)),
            None
        );
        assert_eq!(
            determine_llm_operation(&base(Some("sr"), "sr", "sr", false)),
            None
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn single_warm_llama_server_processes_multiple_projects_and_stops_once() {
        use crate::models::AppConfig;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_warm_batch_test_{}",
            uuid::Uuid::new_v4()
        ));
        let data_dir = temp_dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let mock_model = temp_dir.join("dummy.gguf");
        fs::write(&mock_model, b"GGUF").unwrap();

        let startup_log = temp_dir.join("startups.log");
        let mock_bin = temp_dir.join("mock-llama-server");

        let script = format!(
            r#"#!/usr/bin/env python3
import http.server
import json
import sys

args = sys.argv[1:]
with open({startup_log:?}, "a", encoding="utf-8") as f:
    f.write("started\n")

port = int(args[args.index("--port") + 1])
expected_key = args[args.index("--api-key") + 1]

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            if self.headers.get("Authorization") != f"Bearer {{expected_key}}":
                self.send_response(401)
                self.end_headers()
                return
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b'{{"status":"ok"}}')
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.headers.get("Authorization") != f"Bearer {{expected_key}}":
            self.send_response(401)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length).decode("utf-8"))
        user_payload = json.loads(body["messages"][1]["content"])
        sys_prompt = body["messages"][0]["content"]
        is_polish = "Serbian subtitle editor" in sys_prompt
        out = {{}}
        for cue in user_payload["translate_cues"]:
            cid = str(cue["id"])
            out[cid] = "Ispoliran tekst." if is_polish else "Preveden tekst."
        resp = json.dumps({{"choices": [{{"message": {{"role": "assistant", "content": json.dumps(out)}}}}]}}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(resp)))
        self.end_headers()
        self.wfile.write(resp)

    def log_message(self, format, *args):
        pass

http.server.HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#,
            startup_log = startup_log.to_string_lossy().to_string(),
        );

        fs::write(&mock_bin, script).unwrap();
        let mut perms = fs::metadata(&mock_bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&mock_bin, perms).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let log_dir = data_dir.join("temp");
        let mut guard = llama_server::LlamaServerGuard::start_with_timeout_and_log_dir(
            &mock_bin,
            &mock_model,
            2048,
            &cancel,
            std::time::Duration::from_secs(10),
            Some(&log_dir),
        )
        .await
        .unwrap();

        let endpoint = build_local_endpoint(&guard, &AppConfig::default());
        assert_eq!(endpoint.api_key.as_deref(), Some(guard.api_key()));

        let mut p1 = SubtitleProject {
            job_id: "batch-p1".to_string(),
            audio_path: "/tmp/p1.mp3".to_string(),
            audio_file_name: "p1.mp3".to_string(),
            duration_ms: 2000,
            detected_language: Some("en".to_string()),
            options: JobOptions::default(),
            words: vec![],
            cues: vec![SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 2000,
                source_text: "Hello world.".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 6.0,
                warnings: vec![],
            }],
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-22T22:00:00Z".to_string(),
        };

        let mut p2 = SubtitleProject {
            job_id: "batch-p2".to_string(),
            audio_path: "/tmp/p2.mp3".to_string(),
            audio_file_name: "p2.mp3".to_string(),
            duration_ms: 2000,
            detected_language: Some("sr".to_string()),
            options: JobOptions::default(),
            words: vec![],
            cues: vec![SubtitleCue {
                id: 1,
                start_ms: 0,
                end_ms: 2000,
                source_text: "Lijepo vrijeme.".to_string(),
                translated_text: String::new(),
                translated_variants: BTreeMap::new(),
                cps: 7.5,
                warnings: vec![],
            }],
            phase: JobPhase::Polishing,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-22T22:00:00Z".to_string(),
        };

        for p in [&mut p1, &mut p2] {
            let op = determine_llm_operation(p).unwrap();
            translator::run_batch_operation(
                p,
                op,
                &endpoint,
                &cancel,
                |proj| storage::save_checkpoint_to_data_dir(&data_dir, proj),
                |_, _| {},
            )
            .await
            .unwrap();
        }

        guard.stop();

        let startups = fs::read_to_string(&startup_log).unwrap();
        assert_eq!(startups.lines().count(), 1);
        assert_eq!(p1.cues[0].translated_text, "Preveden tekst.");
        assert_eq!(p2.cues[0].translated_text, "Ispoliran tekst.");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn resegment_project_preserves_split_on_speaker_change_and_annotate_is_non_fatal() {
        let mut whisper_out = whisper::WhisperOutput {
            detected_language: Some("sr".to_string()),
            words: vec![
                TimedWord {
                    start_ms: 0,
                    end_ms: 300,
                    text: "Mislim".to_string(),
                    probability: 0.99,
                    speaker_change: false,
                },
                TimedWord {
                    start_ms: 320,
                    end_ms: 620,
                    text: "da".to_string(),
                    probability: 0.99,
                    speaker_change: false,
                },
                TimedWord {
                    start_ms: 740,
                    end_ms: 1050,
                    text: "jeste".to_string(),
                    probability: 0.99,
                    speaker_change: true,
                },
                TimedWord {
                    start_ms: 1070,
                    end_ms: 1400,
                    text: "tako.".to_string(),
                    probability: 0.99,
                    speaker_change: false,
                },
            ],
            vad_intervals: Vec::new(),
        };

        // Missing WAV must log warning non-fatally and leave words untouched
        let missing = std::env::temp_dir().join(format!(
            "easysrt_nonexistent_diarizer_{}.wav",
            uuid::Uuid::new_v4()
        ));
        annotate_speaker_changes(&missing, &mut whisper_out);
        assert!(whisper_out.words[2].speaker_change);

        let project = SubtitleProject {
            job_id: "job-speaker-reseg".to_string(),
            audio_path: "/tmp/interview.wav".to_string(),
            audio_file_name: "interview.wav".to_string(),
            duration_ms: 2000,
            detected_language: Some("sr".to_string()),
            options: JobOptions::default(),
            words: whisper_out.words,
            cues: vec![],
            phase: JobPhase::Completed,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-24T22:00:00Z".to_string(),
        };

        let split_project = resegment_project(
            project,
            SegmentRules {
                preset: SegmentPreset::Standard,
                split_on_speaker_change: true,
                ..SegmentRules::default()
            },
        )
        .unwrap();
        assert!(split_project.options.segment_rules.split_on_speaker_change);
        assert_eq!(split_project.cues.len(), 2);
        assert_eq!(split_project.cues[0].source_text, "Mislim da");
        assert_eq!(split_project.cues[1].source_text, "jeste tako.");

        let merged_project = resegment_project(
            split_project,
            SegmentRules {
                preset: SegmentPreset::Standard,
                split_on_speaker_change: false,
                ..SegmentRules::default()
            },
        )
        .unwrap();
        assert!(!merged_project.options.segment_rules.split_on_speaker_change);
        assert_eq!(merged_project.cues.len(), 1);
        assert_eq!(merged_project.cues[0].source_text, "Mislim da jeste tako.");
    }

    #[test]
    fn convert_cues_script_updates_translated_text_and_all_dual_style_variants() {
        let mut variants = BTreeMap::new();
        variants.insert(TranslationStyle::Natural, "Ljubav i pažnja.".to_string());
        variants.insert(
            TranslationStyle::Formal,
            "Poštovanje i pažnja u Beogradu.".to_string(),
        );
        let cues = vec![SubtitleCue {
            id: 1,
            start_ms: 0,
            end_ms: 2000,
            source_text: "Love and care.".to_string(),
            translated_text: "Ljubav i pažnja.".to_string(),
            translated_variants: variants,
            cps: 8.0,
            warnings: vec![],
        }];

        let cyr = convert_cues_script(cues, SerbianScript::Cyrillic).unwrap();
        assert_eq!(cyr[0].translated_text, "Љубав и пажња.");
        assert_eq!(
            cyr[0]
                .translated_variants
                .get(&TranslationStyle::Natural)
                .map(String::as_str),
            Some("Љубав и пажња.")
        );
        assert_eq!(
            cyr[0]
                .translated_variants
                .get(&TranslationStyle::Formal)
                .map(String::as_str),
            Some("Поштовање и пажња у Београду.")
        );

        let lat = convert_cues_script(cyr, SerbianScript::Latin).unwrap();
        assert_eq!(lat[0].translated_text, "Ljubav i pažnja.");
        assert_eq!(
            lat[0]
                .translated_variants
                .get(&TranslationStyle::Formal)
                .map(String::as_str),
            Some("Poštovanje i pažnja u Beogradu.")
        );
    }

    #[test]
    fn resegment_project_wires_remove_fillers_option() {
        let words = vec![
            TimedWord {
                start_ms: 0,
                end_ms: 200,
                text: "Um,".to_string(),
                probability: 0.9,
                speaker_change: false,
            },
            TimedWord {
                start_ms: 250,
                end_ms: 600,
                text: "hello".to_string(),
                probability: 0.99,
                speaker_change: false,
            },
            TimedWord {
                start_ms: 650,
                end_ms: 950,
                text: "world.".to_string(),
                probability: 0.99,
                speaker_change: false,
            },
        ];

        let mk_proj = |remove_fillers: bool| SubtitleProject {
            job_id: "job-fillers".to_string(),
            audio_path: "/tmp/f.wav".to_string(),
            audio_file_name: "f.wav".to_string(),
            duration_ms: 1500,
            detected_language: Some("en".to_string()),
            options: JobOptions {
                remove_fillers,
                ..JobOptions::default()
            },
            words: words.clone(),
            cues: vec![],
            phase: JobPhase::Completed,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: "2026-11-25T10:00:00Z".to_string(),
        };

        let stripped = resegment_project(mk_proj(true), SegmentRules::default()).unwrap();
        assert_eq!(stripped.cues[0].source_text, "hello world.");

        let verbatim = resegment_project(mk_proj(false), SegmentRules::default()).unwrap();
        assert_eq!(verbatim.cues[0].source_text, "Um, hello world.");
    }

    #[test]
    fn export_dedupe_and_model_label_helpers_behave_deterministically() {
        let cues = vec![SubtitleCue {
            id: 1,
            start_ms: 0,
            end_ms: 1500,
            source_text: "Hello".to_string(),
            translated_text: "Zdravo".to_string(),
            translated_variants: BTreeMap::new(),
            cps: 4.0,
            warnings: vec![],
        }];
        let h1 = compute_cues_export_hash(&cues);
        let h2 = compute_cues_export_hash(&cues);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);

        let job = format!("dedupe-test-{}", uuid::Uuid::new_v4());
        assert!(mark_export_telemetry_channel(&job, &h1, "export"));
        assert!(!mark_export_telemetry_channel(&job, &h1, "export"));
        assert!(mark_export_telemetry_channel(&job, &h1, "training"));
        assert!(!mark_export_telemetry_channel(&job, &h1, "training"));

        assert_eq!(
            detect_whisper_backend(Path::new("/tmp/whisper-win-cuda/whisper-cli.exe")),
            "cuda"
        );
        assert_eq!(
            detect_whisper_backend(Path::new("/tmp/whisper-linux-vulkan/whisper-cli")),
            "vulkan"
        );
        assert_eq!(
            detect_whisper_backend(Path::new("/usr/bin/whisper-cli")),
            "cpu"
        );

        let cfg = AppConfig {
            active_whisper_model_id: Some("whisper-large-v3-turbo-q5_0".to_string()),
            active_llm_model_id: Some("custom-local-gguf".to_string()),
            ..AppConfig::default()
        };
        assert_eq!(resolve_whisper_model_label(&cfg), "whisper-large-v3-turbo-q5_0");
        assert_eq!(
            resolve_llm_model_label(&cfg, TranslationProviderKind::LocalLlama),
            "custom"
        );
        assert_eq!(
            resolve_llm_model_label(&cfg, TranslationProviderKind::CustomOpenAi),
            "custom"
        );
        assert_eq!(
            resolve_llm_model_label(&cfg, TranslationProviderKind::DeepSeek),
            "deepseek-chat"
        );
    }
}
