//! End-to-end pipeline benchmark across `/home/gru/Downloads/voice_samples/*.mp3`.
//!
//! Run with:
//! `cargo run --release --example voice_samples_benchmark`

use anyhow::{Context, Result};
use easysrt_studio_lib::models::{
    ExportFormat, ExportOptions, JobOptions, JobPhase, SegmentRules, SerbianScript, SubtitleProject,
    TranslationProviderKind,
};
use easysrt_studio_lib::pipeline::{
    exporter, ffmpeg, llama_server, segmenter, sr_script, translator, whisper,
};
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
    ffmpeg_ms: u128,
    whisper_ms: u128,
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

    let mut mp3_files: Vec<PathBuf> = fs::read_dir(&samples_dir)?
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
        })
        .collect();
    mp3_files.sort();

    if mp3_files.is_empty() {
        anyhow::bail!("No MP3 files found in {}", samples_dir.display());
    }

    println!(
        "Running EasySRT Studio benchmark on {} files from {}\n",
        mp3_files.len(),
        samples_dir.display()
    );

    let mut results = Vec::new();
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
        let job_id = format!("bench-{stem}");
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
        let whisper_out = whisper::run_whisper_cli(
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
        let _ = fs::remove_file(&converted.wav_path);

        // 3. Segment words into cues
        let seg_t0 = Instant::now();
        let rules = segmenter::normalize_rules(&SegmentRules::default());
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
                provider_override: Some(TranslationProviderKind::LocalLlama),
            },
            words: whisper_out.words,
            cues,
            phase: JobPhase::Translating,
            completed_translation_batches: 0,
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
        }
        .to_string();

        // 4. Local llama-server start + batch translate/polish + stop
        let llama_t0 = Instant::now();
        let mut guard =
            llama_server::LlamaServerGuard::start(&llama_bin, &llm_model, 2048, &cancel_flag)
                .await?;
        let llama_start_ms = llama_t0.elapsed().as_millis();

        let endpoint = translator::LlmEndpointConfig {
            base_url: guard.base_url().to_string(),
            model: "local-llm".to_string(),
            api_key: None,
            batch_size: 25,
        };

        let infer_t0 = Instant::now();
        let checkpoint_path = checkpoints_dir.join(format!("{job_id}.easysrt.json"));
        let cp_clone = checkpoint_path.clone();
        translator::run_batch_operation(
            &mut project,
            operation,
            &endpoint,
            &cancel_flag,
            move |proj| {
                let data = serde_json::to_string_pretty(proj)?;
                fs::write(&cp_clone, data)?;
                Ok(())
            },
            |_pct, _detail| {},
        )
        .await?;
        let llm_infer_ms = infer_t0.elapsed().as_millis();
        guard.stop();
        let llama_total_ms = llama_t0.elapsed().as_millis();

        project.phase = JobPhase::Completed;
        project.updated_at = chrono::Utc::now().to_rfc3339();
        fs::write(&checkpoint_path, serde_json::to_string_pretty(&project)?)?;

        // 5. Export SRT files beside audio
        let exp_t0 = Instant::now();
        let export_res = exporter::export_project_files(
            &project,
            &ExportOptions {
                output_dir: None,
                save_beside_audio: true,
                include_source_srt: true,
                formats: vec![ExportFormat::Srt],
                utf8_bom: false,
            },
        )?;
        let export_ms = exp_t0.elapsed().as_millis();
        let total_ms = total_t0.elapsed().as_millis();

        let rtf = if converted.duration_ms > 0 {
            total_ms as f64 / converted.duration_ms as f64
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
            "[{file_name}] dur={:.1}s lang={detected_lang} op={op_label} cues={} (warn={warning_cues}) | ffmpeg={}ms whisper={}ms seg={}ms llama_start={}ms llm_infer={}ms warm={:.2}s cold_total={:.2}s (RTF={:.2}x)",
            converted.duration_ms as f64 / 1000.0,
            project.cues.len(),
            ffmpeg_ms,
            whisper_ms,
            segment_ms,
            llama_start_ms,
            llm_infer_ms,
            warm_ms as f64 / 1000.0,
            total_ms as f64 / 1000.0,
            rtf
        );

        results.push(FileBenchmarkResult {
            file_name,
            audio_duration_ms: converted.duration_ms,
            detected_language: whisper_out.detected_language,
            operation: op_label,
            cue_count: project.cues.len(),
            warning_cues,
            ffmpeg_ms,
            whisper_ms,
            segment_ms,
            llama_start_ms,
            llm_infer_ms,
            llama_total_ms,
            export_ms,
            total_ms,
            rtf,
            sample_cues,
            written_files: export_res.written_files,
        });
    }

    let report_path = Path::new("/tmp/voice_samples_benchmark_report.json");
    fs::write(report_path, serde_json::to_string_pretty(&results)?)?;
    println!("\nSaved full JSON benchmark report to {}", report_path.display());

    let sum_audio_ms: u64 = results.iter().map(|r| r.audio_duration_ms).sum();
    let sum_total_ms: u128 = results.iter().map(|r| r.total_ms).sum();
    let sum_whisper_ms: u128 = results.iter().map(|r| r.whisper_ms).sum();
    let sum_llm_infer_ms: u128 = results.iter().map(|r| r.llm_infer_ms).sum();
    let sum_llama_ms: u128 = results.iter().map(|r| r.llama_total_ms).sum();
    let sum_warm_ms = sum_whisper_ms + sum_llm_infer_ms;
    println!(
        "TOTAL across {} files: audio={:.1}s | whisper={:.1}s | llm_infer={:.1}s | llama_total={:.1}s | warm_infer={:.1}s (warm RTF={:.2}x) | cold_total={:.1}s (overall RTF={:.2}x)",
        results.len(),
        sum_audio_ms as f64 / 1000.0,
        sum_whisper_ms as f64 / 1000.0,
        sum_llm_infer_ms as f64 / 1000.0,
        sum_llama_ms as f64 / 1000.0,
        sum_warm_ms as f64 / 1000.0,
        sum_warm_ms as f64 / sum_audio_ms.max(1) as f64,
        sum_total_ms as f64 / 1000.0,
        sum_total_ms as f64 / sum_audio_ms.max(1) as f64
    );

    Ok(())
}
