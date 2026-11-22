//! Subtitle generation pipeline (audio conversion, Whisper transcription,
//! segmentation, local/cloud translation, Serbian polishing, script conversion,
//! and export).

pub mod exporter;
pub mod ffmpeg;
pub mod importer;
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

    let mut llama_guard = None;
    let op_res: Result<()> = async {
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

        translator::run_batch_operation(
            project,
            operation,
            &endpoint,
            cancel_flag,
            |proj| storage::save_checkpoint(app, proj),
            |pct, detail| {
                emit_job_progress(app, &job_id, phase, pct, detail);
            },
        )
        .await
    }
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

pub async fn condense_project_cps(
    app: &AppHandle,
    state: &PipelineState,
    mut project: SubtitleProject,
) -> Result<SubtitleProject> {
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
    use crate::models::{ExportFormat, SegmentPreset, TimedWord};
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
                provider_override: Some(TranslationProviderKind::CustomOpenAi),
            },
            words: whisper_out.words,
            cues: standard_cues,
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
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
}
