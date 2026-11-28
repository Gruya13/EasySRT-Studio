//! End-to-end pipeline benchmark across `~/Downloads/voice_samples/*.mp3`.
//!
//! Run with:
//! `cargo run --release --example voice_samples_benchmark`

use anyhow::{Context, Result};
use easysrt_studio_lib::models::{
    ExportFormat, ExportOptions, JobOptions, JobPhase, SegmentRules, SerbianScript, StorageMode,
    StoragePaths, SubtitleProject, TranslationProviderKind, TranslationStyle,
};
use easysrt_studio_lib::pipeline::{
    diarizer, exporter, ffmpeg, llama_server, segmenter, sr_script, translator, whisper,
};
use easysrt_studio_lib::storage;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileBenchmarkResult {
    file_name: String,
    audio_duration_ms: u64,
    detected_language: Option<String>,
    operation: String,
    cue_count: usize,
    warning_cues: usize,
    speaker_changes_detected: usize,
    ffmpeg_ms: u128,
    whisper_ms: u128,
    diarizer_ms: u128,
    segment_ms: u128,
    llama_start_ms: u128,
    llm_infer_ms: u128,
    llama_total_ms: u128,
    export_ms: u128,
    total_ms: u128,
    rtf: f64,
    sample_cues: Vec<SampleCueSummary>,
    written_files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SampleCueSummary {
    id: u32,
    start_ms: u64,
    end_ms: u64,
    source_text: String,
    translated_text: String,
    cps: f64,
    warnings: Vec<String>,
}

fn find_system_ffmpeg() -> PathBuf {
    for candidate in ["/usr/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("ffmpeg")
}

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME").context("HOME env var missing")?;
    let app_data = PathBuf::from(&home).join(".local/share/com.easysrt.studio");
    let samples_dir = PathBuf::from(&home).join("Downloads/voice_samples");

    let ffmpeg_bin = find_system_ffmpeg();
    let whisper_bin = app_data.join("bin/whisper/whisper-bin-ubuntu-x64/whisper-cli");
    let llama_bin = app_data.join("bin/llama/llama-b12633/llama-server");
    let whisper_model = app_data.join("models/whisper/ggml-large-v3-turbo-q5_0.bin");
    let vad_model = app_data.join("models/vad/ggml-silero-v6.2.0.bin");
    let llm_model = app_data.join("models/llm/gemma-3-4b-it-Q4_K_M.gguf");
    let checkpoints_dir = app_data.join("data/checkpoints");
    let temp_dir = app_data.join("data/temp/bench");
    fs::create_dir_all(&checkpoints_dir)?;
    fs::create_dir_all(&temp_dir)?;

    let args: Vec<String> = std::env::args().collect();
    let arg_value = |flag: &str| -> Option<String> {
        let prefix = format!("{flag}=");
        args.iter()
            .find_map(|a| a.strip_prefix(&prefix).map(ToString::to_string))
            .or_else(|| {
                args.windows(2)
                    .find(|w| w[0] == flag)
                    .map(|w| w[1].clone())
            })
    };

    let use_deepseek = args.iter().any(|a| a == "--provider=deepseek")
        || args.windows(2).any(|w| w == ["--provider", "deepseek"])
        || std::env::var("BENCH_PROVIDER").is_ok_and(|v| v.eq_ignore_ascii_case("deepseek"));
    let pipelined = args.iter().any(|a| a == "--pipelined")
        || std::env::var("BENCH_PIPELINED")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let split_on_speaker_change = !args.iter().any(|a| a == "--no-speaker-split");
    let translation_style = match arg_value("--style").as_deref() {
        Some("formal") => TranslationStyle::Formal,
        _ => TranslationStyle::Natural,
    };
    let output_dir_override = arg_value("--output-dir");
    if let Some(dir) = output_dir_override.as_deref() {
        fs::create_dir_all(dir)?;
    }
    let input_path_override = arg_value("--input").map(PathBuf::from);

    let storage_paths = StoragePaths {
        mode: StorageMode::Installed,
        root_dir: app_data.to_string_lossy().to_string(),
        bin_dir: app_data.join("bin").to_string_lossy().to_string(),
        models_dir: app_data.join("models").to_string_lossy().to_string(),
        data_dir: app_data.join("data").to_string_lossy().to_string(),
        config_path: app_data.join("config.json").to_string_lossy().to_string(),
    };

    if let Ok(env_key) = std::env::var("DEEPSEEK_API_KEY") {
        let trimmed = env_key.trim();
        if !trimmed.is_empty() {
            let key_status = storage::set_api_key_for_paths(
                &storage_paths,
                TranslationProviderKind::DeepSeek,
                trimmed,
                false,
            )?;
            println!(
                "Persisted DeepSeek API key via '{}' backend (hasDeepSeekKey={})",
                key_status.storage_backend, key_status.has_deep_seek_key
            );
        }
    }

    let deepseek_api_key = if use_deepseek {
        let loaded = storage::get_api_key_for_paths(
            &storage_paths,
            TranslationProviderKind::DeepSeek,
            false,
        )?
        .filter(|k| !k.trim().is_empty())
        .context("DeepSeek API key not found in credential store or DEEPSEEK_API_KEY")?;
        Some(loaded)
    } else {
        None
    };

    let input_target = input_path_override.unwrap_or_else(|| samples_dir.clone());
    let mut mp3_files: Vec<PathBuf> = if input_target.is_file() {
        vec![input_target.clone()]
    } else {
        fs::read_dir(&input_target)?
            .filter_map(|e| e.ok().map(|entry| entry.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "mp3" | "wav" | "m4a" | "mp4" | "flac" | "ogg"
                        )
                    })
            })
            .collect()
    };
    mp3_files.sort();

    if mp3_files.is_empty() {
        anyhow::bail!("No audio files found in {}", input_target.display());
    }

    println!(
        "Running EasySRT Studio benchmark (provider={}, pipelined={}, speaker_split={}, style={:?}) on {} file(s) from {}\n",
        if use_deepseek { "deepseek" } else { "localLlama" },
        pipelined && use_deepseek,
        split_on_speaker_change,
        translation_style,
        mp3_files.len(),
        input_target.display()
    );

    if pipelined && !use_deepseek {
        println!("Note: pipelined execution is only enabled with cloud providers to avoid 4 GB VRAM contention.\n");
    }

    let wall_t0 = Instant::now();
    let mut results = Vec::new();
    let mut pending_cloud_tasks: Vec<tokio::task::JoinHandle<Result<FileBenchmarkResult>>> =
        Vec::new();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let glossary = BTreeMap::new();

    for audio_path in mp3_files {
        let file_name = audio_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sample.mp3")
            .to_string();
        let stem = audio_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sample")
            .to_string();
        let job_id = if use_deepseek {
            format!("bench-deepseek-{stem}")
        } else {
            format!("bench-{stem}")
        };
        let temp_wav = temp_dir.join(format!("{job_id}.16k.wav"));

        let total_t0 = Instant::now();

        // 1. FFmpeg decode
        let ff_t0 = Instant::now();
        let converted =
            ffmpeg::convert_to_16khz_mono_wav(&ffmpeg_bin, &audio_path, &temp_wav, &cancel_flag)
                .await?;
        let ffmpeg_ms = ff_t0.elapsed().as_millis();

        // 2. Whisper ASR + Silero VAD
        let wh_t0 = Instant::now();
        let mut whisper_out = whisper::run_whisper_cli(
            &whisper_bin,
            &whisper_model,
            Some(vad_model.as_path()),
            &converted.wav_path,
            "auto",
            &glossary,
            &cancel_flag,
            |_pct, _detail| {},
        )
        .await?;
        let whisper_ms = wh_t0.elapsed().as_millis();

        // 2b. Pure-Rust acoustic speaker-turn diarizer (runs before temp WAV cleanup)
        let diar_t0 = Instant::now();
        let speaker_boundaries = match diarizer::detect_speaker_change_boundaries(
            &converted.wav_path,
            &whisper_out.words,
            &whisper_out.vad_intervals,
        ) {
            Ok(indices) => indices,
            Err(err) => {
                eprintln!("[{file_name}] diarizer warning: {err:#}");
                Vec::new()
            }
        };
        for &idx in &speaker_boundaries {
            if let Some(word) = whisper_out.words.get_mut(idx) {
                word.speaker_change = true;
            }
        }
        let diarizer_ms = diar_t0.elapsed().as_millis();
        let speaker_changes_detected = speaker_boundaries.len();
        let _ = fs::remove_file(&converted.wav_path);

        // 3. Segment words into cues
        let seg_t0 = Instant::now();
        let rules = segmenter::normalize_rules(&SegmentRules {
            split_on_speaker_change,
            ..SegmentRules::default()
        });
        let mut cues = segmenter::segment_words(&whisper_out.words, &rules);
        let detected_lang = whisper_out
            .detected_language
            .clone()
            .unwrap_or_else(|| "en".to_string());
        if detected_lang.eq_ignore_ascii_case("sr") {
            for cue in &mut cues {
                cue.source_text = sr_script::convert_script(&cue.source_text, SerbianScript::Latin);
            }
        }
        let segment_ms = seg_t0.elapsed().as_millis();

        let mut project = SubtitleProject {
            job_id: job_id.clone(),
            audio_path: audio_path.to_string_lossy().to_string(),
            audio_file_name: file_name.clone(),
            duration_ms: converted.duration_ms,
            detected_language: whisper_out.detected_language.clone(),
            options: JobOptions {
                source_language: "auto".to_string(),
                target_language: "sr".to_string(),
                script: SerbianScript::Latin,
                segment_rules: rules,
                glossary: glossary.clone(),
                polish_serbian: true,
                correct_source_asr: false,
                translation_style,
                remove_fillers: true,
                provider_override: Some(if use_deepseek {
                    TranslationProviderKind::DeepSeek
                } else {
                    TranslationProviderKind::LocalLlama
                }),
                ..JobOptions::default()
            },
            words: whisper_out.words,
            cues,
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
            topic_context: None,
            completed_translation_style: None,
            error_message: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        let src_lower = detected_lang.to_lowercase();
        let operation = if src_lower == "sr" || src_lower == "hr" || src_lower == "bs" {
            translator::BatchOperation::PolishSerbian
        } else {
            translator::BatchOperation::Translate
        };
        let op_label = match operation {
            translator::BatchOperation::Translate => "translate",
            translator::BatchOperation::PolishSerbian => "polish_sr",
            translator::BatchOperation::CondenseCps => "condense_cps",
        }
        .to_string();

        // 4. LLM endpoint setup (local llama-server OR DeepSeek API) + batch translate/polish
        let llama_t0 = Instant::now();
        let mut llama_guard = None;
        let (llama_start_ms, endpoint) = if let Some(api_key) = deepseek_api_key.as_ref() {
            (
                0u128,
                translator::LlmEndpointConfig {
                    base_url: "https://api.deepseek.com/v1".to_string(),
                    model: "deepseek-chat".to_string(),
                    api_key: Some(api_key.clone()),
                    batch_size: 25,
                },
            )
        } else {
            let guard = llama_server::LlamaServerGuard::start_in_log_dir(
                &llama_bin,
                &llm_model,
                2048,
                &cancel_flag,
                &temp_dir,
            )
            .await?;
            let start_ms = llama_t0.elapsed().as_millis();
            let ep = translator::LlmEndpointConfig {
                base_url: guard.base_url().to_string(),
                model: "local-llm".to_string(),
                api_key: Some(guard.api_key().to_string()),
                batch_size: 25,
            };
            llama_guard = Some(guard);
            (start_ms, ep)
        };

        let checkpoint_path = checkpoints_dir.join(format!("{job_id}.easysrt.json"));
        let cancel_clone = Arc::clone(&cancel_flag);
        let duration_ms = converted.duration_ms;
        let output_dir_clone = output_dir_override.clone();

        let finalize_fut = async move {
            let infer_t0 = Instant::now();
            let cp_clone = checkpoint_path.clone();
            translator::run_batch_operation(
                &mut project,
                operation,
                &endpoint,
                &cancel_clone,
                move |proj| {
                    let data = serde_json::to_string_pretty(proj)?;
                    fs::write(&cp_clone, data)?;
                    Ok(())
                },
                |_pct, _detail| {},
            )
            .await?;
            let llm_infer_ms = infer_t0.elapsed().as_millis();
            if let Some(mut guard) = llama_guard.take() {
                guard.stop();
            }
            let llama_total_ms = llama_t0.elapsed().as_millis();

            project.phase = JobPhase::Completed;
            project.updated_at = chrono::Utc::now().to_rfc3339();
            fs::write(&checkpoint_path, serde_json::to_string_pretty(&project)?)?;

            // 5. Export SRT files beside audio or into --output-dir
            let exp_t0 = Instant::now();
            let export_res = exporter::export_project_files(
                &project,
                &ExportOptions {
                    output_dir: output_dir_clone.clone(),
                    save_beside_audio: output_dir_clone.is_none(),
                    include_source_srt: true,
                    formats: vec![ExportFormat::Srt],
                    utf8_bom: false,
                },
            )?;
            let export_ms = exp_t0.elapsed().as_millis();
            let total_ms = total_t0.elapsed().as_millis();

            let rtf = if duration_ms > 0 {
                total_ms as f64 / duration_ms as f64
            } else {
                0.0
            };
            let warning_cues = project
                .cues
                .iter()
                .filter(|c| !c.warnings.is_empty())
                .count();

            let sample_indices: Vec<usize> = match project.cues.len() {
                0 => vec![],
                1 => vec![0],
                2 => vec![0, 1],
                n => vec![0, 1, n - 1],
            };
            let sample_cues = sample_indices
                .into_iter()
                .map(|idx| {
                    let c = &project.cues[idx];
                    SampleCueSummary {
                        id: c.id,
                        start_ms: c.start_ms,
                        end_ms: c.end_ms,
                        source_text: c.source_text.clone(),
                        translated_text: c.translated_text.clone(),
                        cps: c.cps,
                        warnings: c.warnings.clone(),
                    }
                })
                .collect();

            let warm_ms = whisper_ms + llm_infer_ms;
            println!(
                "[{file_name}] dur={:.1}s lang={detected_lang} op={op_label} cues={} (warn={warning_cues}, spk_turns={speaker_changes_detected}) | ffmpeg={}ms whisper={}ms diar={}ms seg={}ms llama_start={}ms llm_infer={}ms warm={:.2}s cold_total={:.2}s (RTF={:.2}x)",
                duration_ms as f64 / 1000.0,
                project.cues.len(),
                ffmpeg_ms,
                whisper_ms,
                diarizer_ms,
                segment_ms,
                llama_start_ms,
                llm_infer_ms,
                warm_ms as f64 / 1000.0,
                total_ms as f64 / 1000.0,
                rtf
            );

            Ok::<FileBenchmarkResult, anyhow::Error>(FileBenchmarkResult {
                file_name,
                audio_duration_ms: duration_ms,
                detected_language: whisper_out.detected_language,
                operation: op_label,
                cue_count: project.cues.len(),
                warning_cues,
                speaker_changes_detected,
                ffmpeg_ms,
                whisper_ms,
                diarizer_ms,
                segment_ms,
                llama_start_ms,
                llm_infer_ms,
                llama_total_ms,
                export_ms,
                total_ms,
                rtf,
                sample_cues,
                written_files: export_res.written_files,
            })
        };

        if use_deepseek && pipelined {
            pending_cloud_tasks.push(tokio::spawn(finalize_fut));
        } else {
            results.push(finalize_fut.await?);
        }
    }

    for handle in pending_cloud_tasks {
        let res = handle.await.context("Pipelined benchmark task panicked")??;
        results.push(res);
    }

    let wall_elapsed_ms = wall_t0.elapsed().as_millis();
    let sum_audio_ms: u64 = results.iter().map(|r| r.audio_duration_ms).sum();
    let wall_rtf = wall_elapsed_ms as f64 / sum_audio_ms.max(1) as f64;

    if use_deepseek && pipelined {
        let report_path = Path::new("/tmp/voice_samples_deepseek_pipelined_report.json");
        let payload = serde_json::json!({
            "provider": "deepseek",
            "pipelined": true,
            "fileCount": results.len(),
            "audioTotalMs": sum_audio_ms,
            "wallTotalMs": wall_elapsed_ms,
            "wallRtf": wall_rtf,
            "files": results,
        });
        fs::write(report_path, serde_json::to_string_pretty(&payload)?)?;
        println!("\nSaved full JSON benchmark report to {}", report_path.display());
    } else {
        let report_path = if use_deepseek {
            Path::new("/tmp/voice_samples_deepseek_report.json")
        } else {
            Path::new("/tmp/voice_samples_benchmark_report.json")
        };
        fs::write(report_path, serde_json::to_string_pretty(&results)?)?;
        println!("\nSaved full JSON benchmark report to {}", report_path.display());
    }

    let sum_total_ms: u128 = results.iter().map(|r| r.total_ms).sum();
    let sum_whisper_ms: u128 = results.iter().map(|r| r.whisper_ms).sum();
    let sum_llm_infer_ms: u128 = results.iter().map(|r| r.llm_infer_ms).sum();
    let sum_llama_ms: u128 = results.iter().map(|r| r.llama_total_ms).sum();
    let sum_warm_ms = sum_whisper_ms + sum_llm_infer_ms;
    println!(
        "TOTAL across {} files: audio={:.1}s | whisper={:.1}s | llm_infer={:.1}s | llama_total={:.1}s | warm_infer={:.1}s (warm RTF={:.2}x) | sum_files={:.1}s | wall_total={:.2}s (wall RTF={:.2}x)",
        results.len(),
        sum_audio_ms as f64 / 1000.0,
        sum_whisper_ms as f64 / 1000.0,
        sum_llm_infer_ms as f64 / 1000.0,
        sum_llama_ms as f64 / 1000.0,
        sum_warm_ms as f64 / 1000.0,
        sum_warm_ms as f64 / sum_audio_ms.max(1) as f64,
        sum_total_ms as f64 / 1000.0,
        wall_elapsed_ms as f64 / 1000.0,
        wall_rtf
    );

    Ok(())
}
