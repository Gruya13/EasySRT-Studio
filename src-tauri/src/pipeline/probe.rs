//! Workstream 5 — Preemptible 3-Stage Background Media Intake Probe.
//!
//! Runs quietly after audio/video import (strictly serialized one file at a time,
//! zero overlap with real transcription/translation jobs) and progressively emits
//! `probe-progress` (`EVENT_PROBE_PROGRESS`) events across:
//! 1. Stage 1 (`"acoustic"`): FFmpeg duration + sparse 12 s 16 kHz PCM windows +
//!    CPU MFCC/F0 speaker count (`diarizer::estimate_speaker_count_from_wav_slices`)
//!    and voiced speech ratio.
//! 2. Stage 2 (`"asr"`): `whisper-cli` on the first ~35 s window for `detected_language`
//!    and a ~80–120 word transcript snippet.
//! 3. Stage 3 (`"topic"`): Configured cloud LLM (or idle GPU local `llama-server`)
//!    via `translator::classify_media_topic` for concise topic + suggested register.
//!
//! Clicking Run/Transcribe or calling `cancel_media_probe` sets `cancel_flag` and
//! awaits clean teardown (including temp WAV cleanup and local `llama-server` stop)
//! so zero GPU/VRAM contention occurs.

use crate::models::{
    CatalogItemKind, MediaProbeResult, TranslationProviderKind, VramTier, EVENT_PROBE_PROGRESS,
};
use crate::pipeline::{diarizer, ffmpeg, llama_server, translator, whisper, PipelineState};
use crate::{hardware, setup_manager, storage};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter};

const SHORT_CLIP_THRESHOLD_MS: u64 = 90_000;
const SPARSE_WINDOW_SECS: f64 = 14.0;
const SPARSE_WINDOW_FRACTIONS: [f64; 6] = [0.0, 0.16, 0.34, 0.52, 0.70, 0.88];
const ASR_SNIPPET_WINDOW_SECS: f64 = 35.0;
const MAX_SNIPPET_WORDS: usize = 120;
const MIN_SNIPPET_WORDS_FOR_TOPIC: usize = 5;

const SPEECH_FRAME_LEN: usize = 512; // 32 ms @ 16 kHz
const SPEECH_HOP_LEN: usize = 320; // 20 ms @ 16 kHz
const SPEECH_ABS_RMS_FLOOR: f64 = 0.004;
const SPEECH_REL_RMS_RATIO: f64 = 0.12;

/// RAII cleanup guard guaranteeing every temporary probe WAV file is deleted
/// on normal completion, cancellation, error, or panic.
struct TempFileCleanupGuard {
    paths: Vec<PathBuf>,
}

impl TempFileCleanupGuard {
    fn new() -> Self {
        Self { paths: Vec::new() }
    }

    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for TempFileCleanupGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// Computes sparse `(start_secs, duration_secs)` windows for Stage 1 acoustic probing:
/// - Clips `<= 90 s`: single full-clip window `(0.0, dur_secs)`.
/// - Clips `> 90 s`: up to 6 sparse 14-second windows at `0%`, `16%`, `34%`, `52%`, `70%`, `88%`.
pub fn compute_probe_window_plan(duration_ms: u64) -> Vec<(f64, f64)> {
    if duration_ms == 0 {
        return vec![(0.0, SPARSE_WINDOW_SECS)];
    }

    let dur_secs = (duration_ms as f64) / 1000.0;
    if duration_ms <= SHORT_CLIP_THRESHOLD_MS {
        return vec![(0.0, dur_secs.max(0.5))];
    }

    let mut windows: Vec<(f64, f64)> = Vec::with_capacity(SPARSE_WINDOW_FRACTIONS.len());
    let max_start = (dur_secs - SPARSE_WINDOW_SECS).max(0.0);

    for frac in SPARSE_WINDOW_FRACTIONS {
        let start = (dur_secs * frac).min(max_start).max(0.0);
        let win_dur = SPARSE_WINDOW_SECS.min((dur_secs - start).max(0.5));
        if windows
            .last()
            .is_some_and(|&(prev_s, prev_d)| start < prev_s + prev_d - 0.5)
        {
            continue;
        }
        windows.push((start, win_dur));
    }

    if windows.is_empty() {
        windows.push((0.0, dur_secs.clamp(0.5, SPARSE_WINDOW_SECS)));
    }

    windows
}

/// Reads a 16-bit PCM WAV file's `data` chunk samples normalized to `[-1.0, 1.0]`.
fn read_wav_pcm16_samples(path: &Path) -> Result<Vec<f64>> {
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open probe WAV slice: {}", path.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("Failed to inspect probe WAV slice: {}", path.display()))?
        .len();
    if file_len < 12 {
        return Ok(Vec::new());
    }

    let mut riff = [0u8; 12];
    file.read_exact(&mut riff)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Err(anyhow!("Invalid RIFF/WAVE header in {}", path.display()));
    }

    let mut pos: u64 = 12;
    let mut data_loc: Option<(u64, u64)> = None;
    while pos.saturating_add(8) <= file_len {
        let mut hdr = [0u8; 8];
        file.read_exact(&mut hdr)?;
        let chunk_size = u64::from(u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]));
        let payload_start = pos.saturating_add(8);

        if &hdr[0..4] == b"data" {
            let avail = file_len.saturating_sub(payload_start);
            let pcm_bytes = chunk_size.min(avail) & !1;
            data_loc = Some((payload_start, pcm_bytes));
            break;
        }

        let padded = chunk_size.saturating_add(chunk_size & 1);
        let Some(next_pos) = payload_start.checked_add(padded) else {
            break;
        };
        if next_pos <= pos || next_pos >= file_len {
            break;
        }
        file.seek(SeekFrom::Start(next_pos))?;
        pos = next_pos;
    }

    let Some((offset, pcm_bytes)) = data_loc else {
        return Ok(Vec::new());
    };
    if pcm_bytes == 0 {
        return Ok(Vec::new());
    }

    file.seek(SeekFrom::Start(offset))?;
    let mut raw = vec![0u8; pcm_bytes as usize];
    file.read_exact(&mut raw)?;

    let mut samples = Vec::with_capacity((pcm_bytes / 2) as usize);
    for &[lo, hi] in raw.as_chunks::<2>().0 {
        let s = i16::from_le_bytes([lo, hi]);
        samples.push(f64::from(s) / 32_768.0);
    }
    Ok(samples)
}

/// Computes active speech frame ratio in `0.0..=1.0` across one or more 16 kHz mono PCM16 WAV slices.
pub fn compute_speech_ratio_from_wav_slices(
    wav_paths: &[&Path],
    cancel_flag: &Arc<AtomicBool>,
) -> Result<f64> {
    if wav_paths.is_empty() {
        return Ok(0.0);
    }

    let mut frame_rms_values: Vec<f64> = Vec::new();
    let mut peak_rms = 0.0_f64;

    for &wav_path in wav_paths {
        if cancel_flag.load(Ordering::SeqCst) {
            return Err(anyhow!("Probe cancelled"));
        }
        let samples = read_wav_pcm16_samples(wav_path)?;
        if samples.len() < SPEECH_FRAME_LEN {
            continue;
        }
        let inv_n = 1.0 / (SPEECH_FRAME_LEN as f64);
        let mut off = 0usize;
        while off.saturating_add(SPEECH_FRAME_LEN) <= samples.len() {
            let slice = &samples[off..off + SPEECH_FRAME_LEN];
            let mut sum = 0.0_f64;
            for &s in slice {
                sum += s;
            }
            let mean = sum * inv_n;
            let mut sq = 0.0_f64;
            for &s in slice {
                let d = s - mean;
                sq += d * d;
            }
            let rms = (sq * inv_n).max(0.0).sqrt();
            if rms.is_finite() {
                if rms > peak_rms {
                    peak_rms = rms;
                }
                frame_rms_values.push(rms);
            }
            off = off.saturating_add(SPEECH_HOP_LEN);
        }
    }

    if frame_rms_values.is_empty() || peak_rms < SPEECH_ABS_RMS_FLOOR {
        return Ok(0.0);
    }

    let gate = (peak_rms * SPEECH_REL_RMS_RATIO).max(SPEECH_ABS_RMS_FLOOR);
    let active = frame_rms_values.iter().filter(|&&r| r >= gate).count();
    let ratio = (active as f64) / (frame_rms_values.len() as f64);
    Ok(((ratio.clamp(0.0, 1.0)) * 1000.0).round() / 1000.0)
}

/// Runs CPU-only Stage 1 acoustic analysis (`estimated_speakers`, `speech_ratio`)
/// over extracted 16 kHz mono PCM16 WAV slices, checking `cancel_flag` cooperatively.
pub fn analyze_acoustic_wav_slices(
    wav_paths: &[&Path],
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(u32, f64)> {
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Probe cancelled"));
    }

    let speech_ratio = compute_speech_ratio_from_wav_slices(wav_paths, cancel_flag)?;
    if cancel_flag.load(Ordering::SeqCst) {
        return Err(anyhow!("Probe cancelled"));
    }

    let speakers = diarizer::estimate_speaker_count_from_wav_slices(wav_paths)? as u32;
    Ok((speakers.clamp(1, 3), speech_ratio))
}

fn extract_snippet_text(words: &[crate::models::TimedWord], max_words: usize) -> String {
    words
        .iter()
        .map(|w| w.text.trim())
        .filter(|t| !t.is_empty())
        .take(max_words)
        .collect::<Vec<_>>()
        .join(" ")
}

fn emit_probe_progress(app: &AppHandle, result: &MediaProbeResult) {
    let _ = app.emit(EVENT_PROBE_PROGRESS, result.clone());
}

fn build_probe_temp_path(temp_dir: &Path, probe_nonce: &str, suffix: &str) -> PathBuf {
    temp_dir.join(format!("probe-{probe_nonce}-{suffix}.wav"))
}

/// Executes the 3-stage preemptible background media intake probe for `audio_path`.
///
/// Emits progressive `probe-progress` events (`"acoustic"` -> `"asr"` -> `"topic"` -> `"completed"`
/// or `"cancelled"` / `"failed"`) and guarantees all temporary WAV files and any transient
/// `llama-server` process are cleaned up before returning.
pub async fn probe_media_file(
    app: &AppHandle,
    state: &PipelineState,
    job_id: &str,
    audio_path: &str,
) -> Result<MediaProbeResult> {
    let resolved_job_id = job_id.trim();
    if resolved_job_id.is_empty() {
        return Err(anyhow!("Media probe job_id must not be empty"));
    }

    // Serialize probes strictly one-at-a-time: preempt any previous in-flight probe first.
    let (cancel_flag, _probe_guard) = state.begin_probe(resolved_job_id).await;

    let mut result = MediaProbeResult {
        job_id: resolved_job_id.to_string(),
        audio_path: audio_path.to_string(),
        stage: "acoustic".to_string(),
        duration_ms: None,
        speech_ratio: None,
        estimated_speakers: None,
        detected_language: None,
        topic: None,
        suggested_style: None,
        suggested_address: None,
        error: None,
    };

    if cancel_flag.load(Ordering::SeqCst) {
        result.stage = "cancelled".to_string();
        emit_probe_progress(app, &result);
        return Ok(result);
    }

    let raw_audio_path = audio_path.to_string();
    let canonical_path =
        match tokio::task::spawn_blocking(move || ffmpeg::validate_approved_media_file(&raw_audio_path))
            .await
            .map_err(|e| anyhow!("Media validation task panicked: {e}"))?
        {
            Ok(p) => p,
            Err(err) => {
                result.stage = "failed".to_string();
                result.error = Some(err.to_string());
                emit_probe_progress(app, &result);
                return Err(err);
            }
        };

    let app_clone = app.clone();
    let (ffmpeg_bin, storage_paths) = match tokio::task::spawn_blocking(move || -> Result<_> {
        let ffmpeg = setup_manager::resolve_binary_path(&app_clone, CatalogItemKind::Ffmpeg)?;
        let paths = storage::resolve_storage_paths(&app_clone, None)?;
        Ok((ffmpeg, paths))
    })
    .await
    .map_err(|e| anyhow!("Probe path resolution task panicked: {e}"))?
    {
        Ok(pair) => pair,
        Err(err) => {
            result.stage = "failed".to_string();
            result.error = Some(err.to_string());
            emit_probe_progress(app, &result);
            return Err(err);
        }
    };

    let temp_dir = Path::new(&storage_paths.data_dir).join("temp");
    let _ = fs::create_dir_all(&temp_dir);
    let probe_nonce = uuid::Uuid::new_v4().to_string();
    let mut cleanup = TempFileCleanupGuard::new();

    // -----------------------------------------------------------------------
    // Stage 1: Acoustic & Speaker Probe (FFmpeg duration + full <= 90 s or sparse 14 s windows)
    // -----------------------------------------------------------------------
    let duration_ms = match ffmpeg::probe_media_duration_ms(&ffmpeg_bin, &canonical_path).await {
        Ok(ms) => ms,
        Err(err) => {
            if cancel_flag.load(Ordering::SeqCst) {
                result.stage = "cancelled".to_string();
                emit_probe_progress(app, &result);
                return Ok(result);
            }
            result.stage = "failed".to_string();
            result.error = Some(err.to_string());
            emit_probe_progress(app, &result);
            return Err(err);
        }
    };
    result.duration_ms = Some(duration_ms);

    if cancel_flag.load(Ordering::SeqCst) {
        result.stage = "cancelled".to_string();
        emit_probe_progress(app, &result);
        return Ok(result);
    }

    let window_plan = compute_probe_window_plan(duration_ms);
    let mut window_entries: Vec<(f64, PathBuf)> = Vec::with_capacity(window_plan.len());
    for (idx, (start_secs, win_dur)) in window_plan.iter().enumerate() {
        if cancel_flag.load(Ordering::SeqCst) {
            result.stage = "cancelled".to_string();
            emit_probe_progress(app, &result);
            return Ok(result);
        }
        let win_path = build_probe_temp_path(&temp_dir, &probe_nonce, &format!("win{idx}"));
        cleanup.track(win_path.clone());
        match ffmpeg::extract_pcm16_window_wav(
            &ffmpeg_bin,
            &canonical_path,
            &win_path,
            *start_secs,
            *win_dur,
            &cancel_flag,
        )
        .await
        {
            Ok(extracted_ms) if extracted_ms > 0 => {
                window_entries.push((*start_secs, win_path));
            }
            Ok(_) => {}
            Err(err) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    result.stage = "cancelled".to_string();
                    emit_probe_progress(app, &result);
                    return Ok(result);
                }
                tracing::warn!(
                    "Probe window {idx} extraction failed for {}: {err:#}",
                    canonical_path.display()
                );
            }
        }
    }

    if !window_entries.is_empty() {
        let paths_for_cpu: Vec<PathBuf> = window_entries.iter().map(|(_, p)| p.clone()).collect();
        let flag_for_cpu = Arc::clone(&cancel_flag);
        let acoustic_res = tokio::task::spawn_blocking(move || {
            let refs: Vec<&Path> = paths_for_cpu.iter().map(PathBuf::as_path).collect();
            analyze_acoustic_wav_slices(&refs, &flag_for_cpu)
        })
        .await
        .map_err(|e| anyhow!("Acoustic probe task panicked: {e}"))?;

        match acoustic_res {
            Ok((speakers, speech_ratio)) => {
                result.estimated_speakers = Some(speakers);
                result.speech_ratio = Some(speech_ratio);
            }
            Err(err) => {
                if cancel_flag.load(Ordering::SeqCst) {
                    result.stage = "cancelled".to_string();
                    emit_probe_progress(app, &result);
                    return Ok(result);
                }
                tracing::warn!("Acoustic probe analysis failed: {err:#}");
            }
        }
    }

    result.stage = "acoustic".to_string();
    emit_probe_progress(app, &result);

    if cancel_flag.load(Ordering::SeqCst) {
        result.stage = "cancelled".to_string();
        emit_probe_progress(app, &result);
        return Ok(result);
    }

    // -----------------------------------------------------------------------
    // Stage 2: Short Speech Snippet ASR (~35 s window via whisper-cli)
    // -----------------------------------------------------------------------
    let app_for_whisper = app.clone();
    let whisper_assets = tokio::task::spawn_blocking(move || {
        let bin = setup_manager::resolve_binary_path(&app_for_whisper, CatalogItemKind::WhisperBin)
            .ok();
        let model = setup_manager::resolve_active_whisper_model_path(&app_for_whisper).ok();
        let vad = setup_manager::resolve_vad_model_path(&app_for_whisper)
            .ok()
            .flatten();
        (bin, model, vad)
    })
    .await
    .unwrap_or((None, None, None));

    let mut snippet_text = String::new();
    if let (Some(whisper_bin), Some(whisper_model), vad_model) = whisper_assets {
        let asr_wav = if duration_ms > 0
            && duration_ms <= (ASR_SNIPPET_WINDOW_SECS as u64) * 1000
            && window_entries.len() == 1
        {
            Some(window_entries[0].1.clone())
        } else {
            let asr_dur = if duration_ms > 0 {
                ASR_SNIPPET_WINDOW_SECS.min((duration_ms as f64 / 1000.0).max(1.0))
            } else {
                ASR_SNIPPET_WINDOW_SECS
            };
            let asr_path = build_probe_temp_path(&temp_dir, &probe_nonce, "asr35s");
            cleanup.track(asr_path.clone());
            match ffmpeg::extract_pcm16_window_wav(
                &ffmpeg_bin,
                &canonical_path,
                &asr_path,
                0.0,
                asr_dur,
                &cancel_flag,
            )
            .await
            {
                Ok(ms) if ms > 0 => Some(asr_path),
                Ok(_) => None,
                Err(err) => {
                    if cancel_flag.load(Ordering::SeqCst) {
                        result.stage = "cancelled".to_string();
                        emit_probe_progress(app, &result);
                        return Ok(result);
                    }
                    tracing::warn!("Probe Stage 2 WAV extraction failed: {err:#}");
                    None
                }
            }
        };

        if let Some(asr_wav_path) = asr_wav {
            if cancel_flag.load(Ordering::SeqCst) {
                result.stage = "cancelled".to_string();
                emit_probe_progress(app, &result);
                return Ok(result);
            }

            let empty_glossary = BTreeMap::new();
            match whisper::run_whisper_cli(
                &whisper_bin,
                &whisper_model,
                vad_model.as_deref(),
                &asr_wav_path,
                "auto",
                &empty_glossary,
                &cancel_flag,
                |_, _| {},
            )
            .await
            {
                Ok(whisper_out) => {
                    if duration_ms > SHORT_CLIP_THRESHOLD_MS
                        && result.estimated_speakers.unwrap_or(1) <= 1
                        && !window_entries.is_empty()
                        && asr_wav_path != window_entries[0].1
                        && !cancel_flag.load(Ordering::SeqCst)
                    {
                        let asr_dur =
                            ASR_SNIPPET_WINDOW_SECS.min((duration_ms as f64 / 1000.0).max(1.0));
                        let mut refined_paths = Vec::with_capacity(1 + window_entries.len());
                        refined_paths.push(asr_wav_path.clone());
                        for (start_secs, p) in &window_entries {
                            if *start_secs >= asr_dur - 1.0 {
                                refined_paths.push(p.clone());
                            }
                        }
                        if let Ok(Ok(refined_spk)) = tokio::task::spawn_blocking(move || {
                            let refs: Vec<&Path> =
                                refined_paths.iter().map(PathBuf::as_path).collect();
                            diarizer::estimate_speaker_count_from_wav_slices(&refs)
                        })
                        .await
                        {
                            if cancel_flag.load(Ordering::SeqCst) {
                                result.stage = "cancelled".to_string();
                                emit_probe_progress(app, &result);
                                return Ok(result);
                            }
                            let refined_u32 = (refined_spk as u32).clamp(1, 3);
                            if refined_u32 > result.estimated_speakers.unwrap_or(1) {
                                result.estimated_speakers = Some(refined_u32);
                            }
                        }
                    }

                    result.detected_language = whisper_out.detected_language.clone();
                    snippet_text = extract_snippet_text(&whisper_out.words, MAX_SNIPPET_WORDS);
                    result.stage = "asr".to_string();
                    emit_probe_progress(app, &result);
                }
                Err(err) => {
                    if cancel_flag.load(Ordering::SeqCst) {
                        result.stage = "cancelled".to_string();
                        emit_probe_progress(app, &result);
                        return Ok(result);
                    }
                    tracing::warn!("Probe Stage 2 Whisper ASR skipped due to error: {err:#}");
                }
            }
        }
    }

    if cancel_flag.load(Ordering::SeqCst) {
        result.stage = "cancelled".to_string();
        emit_probe_progress(app, &result);
        return Ok(result);
    }

    // -----------------------------------------------------------------------
    // Stage 3: Topic & Style Inference (Cloud key OR idle GPU local llama-server)
    // -----------------------------------------------------------------------
    let snippet_word_count = snippet_text.split_whitespace().count();
    if snippet_word_count >= MIN_SNIPPET_WORDS_FOR_TOPIC {
        let app_for_cfg = app.clone();
        let config_opt = tokio::task::spawn_blocking(move || storage::load_config(&app_for_cfg).ok())
            .await
            .ok()
            .flatten();

        if let Some(config) = config_opt {
            let mut local_guard: Option<llama_server::LlamaServerGuard> = None;

            let endpoint_opt = match config.translation.provider {
                TranslationProviderKind::DeepSeek | TranslationProviderKind::CustomOpenAi => {
                    let app_ep = app.clone();
                    let cfg_ep = config.clone();
                    let prov = config.translation.provider;
                    tokio::task::spawn_blocking(move || {
                        super::resolve_cloud_endpoint(&app_ep, &cfg_ep, prov).ok()
                    })
                    .await
                    .ok()
                    .flatten()
                }
                TranslationProviderKind::LocalLlama => {
                    // Only spin up local llama-server for background topic probe when GPU is available
                    // and no active pipeline jobs are running.
                    if !state.has_active_jobs().await {
                        let app_local = app.clone();
                        let local_ready = tokio::task::spawn_blocking(move || {
                            let gpu = hardware::detect_gpu();
                            if gpu.recommended_tier == VramTier::CpuOnly {
                                return None;
                            }
                            let llama_bin = setup_manager::resolve_binary_path(
                                &app_local,
                                CatalogItemKind::LlamaBin,
                            )
                            .ok()?;
                            let llm_model =
                                setup_manager::resolve_active_llm_model_path(&app_local).ok()?;
                            Some((llama_bin, llm_model))
                        })
                        .await
                        .ok()
                        .flatten();

                        if let Some((llama_bin, llm_model)) = local_ready {
                            if !cancel_flag.load(Ordering::SeqCst) {
                                match llama_server::LlamaServerGuard::start_in_log_dir(
                                    &llama_bin,
                                    &llm_model,
                                    2048,
                                    &cancel_flag,
                                    &temp_dir,
                                )
                                .await
                                {
                                    Ok(guard) => {
                                        let ep = super::build_local_endpoint(&guard, &config);
                                        local_guard = Some(guard);
                                        Some(ep)
                                    }
                                    Err(err) => {
                                        if cancel_flag.load(Ordering::SeqCst) {
                                            result.stage = "cancelled".to_string();
                                            emit_probe_progress(app, &result);
                                            return Ok(result);
                                        }
                                        tracing::warn!(
                                            "Probe Stage 3 local llama-server skipped: {err:#}"
                                        );
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            };

            if let Some(endpoint) = endpoint_opt {
                let topic_res = translator::classify_media_topic(
                    &endpoint,
                    &snippet_text,
                    result.detected_language.as_deref(),
                    &cancel_flag,
                )
                .await;

                if let Some(mut guard) = local_guard.take() {
                    guard.stop();
                }

                match topic_res {
                    Ok(classification) => {
                        result.topic = Some(classification.topic);
                        result.suggested_style = Some(classification.suggested_style);
                        if classification.suggested_address.is_some() {
                            result.suggested_address = classification.suggested_address;
                        }
                        if let Some(llm_speakers) = classification.estimated_speakers {
                            let merged = result
                                .estimated_speakers
                                .unwrap_or(1)
                                .max(llm_speakers)
                                .clamp(1, 3);
                            result.estimated_speakers = Some(merged);
                        }
                        result.stage = "topic".to_string();
                        emit_probe_progress(app, &result);
                    }
                    Err(err) => {
                        if cancel_flag.load(Ordering::SeqCst) {
                            result.stage = "cancelled".to_string();
                            emit_probe_progress(app, &result);
                            return Ok(result);
                        }
                        tracing::warn!("Probe Stage 3 topic classification skipped: {err:#}");
                    }
                }
            } else if let Some(mut guard) = local_guard.take() {
                guard.stop();
            }
        }
    }

    if cancel_flag.load(Ordering::SeqCst) {
        result.stage = "cancelled".to_string();
        emit_probe_progress(app, &result);
        return Ok(result);
    }

    result.stage = "completed".to_string();
    emit_probe_progress(app, &result);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    struct SynthFormant {
        center_hz: f64,
        bandwidth_hz: f64,
        gain: f64,
    }

    #[derive(Clone, Copy)]
    struct SynthVoice {
        f0_hz: f64,
        formants: [SynthFormant; 3],
    }

    const VOICE_120HZ: SynthVoice = SynthVoice {
        f0_hz: 120.0,
        formants: [
            SynthFormant {
                center_hz: 450.0,
                bandwidth_hz: 120.0,
                gain: 1.3,
            },
            SynthFormant {
                center_hz: 1_050.0,
                bandwidth_hz: 180.0,
                gain: 0.9,
            },
            SynthFormant {
                center_hz: 2_400.0,
                bandwidth_hz: 250.0,
                gain: 0.35,
            },
        ],
    };

    const VOICE_215HZ: SynthVoice = SynthVoice {
        f0_hz: 215.0,
        formants: [
            SynthFormant {
                center_hz: 680.0,
                bandwidth_hz: 140.0,
                gain: 0.95,
            },
            SynthFormant {
                center_hz: 1_950.0,
                bandwidth_hz: 220.0,
                gain: 1.3,
            },
            SynthFormant {
                center_hz: 3_100.0,
                bandwidth_hz: 300.0,
                gain: 0.85,
            },
        ],
    };

    fn synthesize_voice_wav(path: &Path, voice: SynthVoice, duration_secs: f64) {
        let sample_rate = 16_000usize;
        let total_samples = ((sample_rate as f64) * duration_secs).round() as usize;
        let mut pcm = Vec::with_capacity(total_samples);

        // Precompute harmonic weights shaped by formant resonances
        let max_h = ((7_200.0 / voice.f0_hz).floor() as usize).clamp(1, 40);
        let mut harmonic_amps = Vec::with_capacity(max_h);
        for h in 1..=max_h {
            let freq = (h as f64) * voice.f0_hz;
            let glottal = 1.0 / (h as f64).powf(0.85);
            let mut env = 0.05;
            for f in &voice.formants {
                let delta = (freq - f.center_hz) / f.bandwidth_hz;
                env += f.gain / (1.0 + delta * delta);
            }
            harmonic_amps.push((freq, glottal * env));
        }

        let norm_sum: f64 = harmonic_amps.iter().map(|(_, a)| *a).sum::<f64>().max(1.0);
        for i in 0..total_samples {
            let t = (i as f64) / (sample_rate as f64);
            let mut v = 0.0_f64;
            for &(freq, amp) in &harmonic_amps {
                v += amp * (TAU * freq * t).sin();
            }
            let sample = ((v / norm_sum) * 0.75).clamp(-0.98, 0.98);
            pcm.push((sample * 32_767.0).round() as i16);
        }

        write_pcm16_wav(path, &pcm);
    }

    fn write_pcm16_wav(path: &Path, samples: &[i16]) {
        let data_len = (samples.len() * 2) as u32;
        let riff_len = data_len + 36;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_len.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&32_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for &s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn computes_sparse_probe_window_plans_for_short_and_long_media() {
        let short = compute_probe_window_plan(28_000);
        assert_eq!(short.len(), 1);
        assert!((short[0].0 - 0.0).abs() < 1e-6);
        assert!((short[0].1 - 28.0).abs() < 1e-6);

        let medium = compute_probe_window_plan(80_640);
        assert_eq!(medium.len(), 1);
        assert!((medium[0].0 - 0.0).abs() < 1e-6);
        assert!((medium[0].1 - 80.64).abs() < 1e-6);

        let long = compute_probe_window_plan(600_000); // 10 minutes
        assert_eq!(long.len(), 6);
        let expected_starts = [0.0, 96.0, 204.0, 312.0, 420.0, 528.0];
        for ((start, dur), exp_start) in long.into_iter().zip(expected_starts) {
            assert!((start - exp_start).abs() < 1e-6);
            assert!((dur - 14.0).abs() < 1e-6);
        }
    }

    #[test]
    fn acoustic_probe_clusters_one_vs_two_synthetic_speakers_and_speech_ratio() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_probe_speakers_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let w0 = temp_dir.join("spk_a_0.wav");
        let w1 = temp_dir.join("spk_a_1.wav");
        let w2 = temp_dir.join("spk_b_0.wav");
        let w3 = temp_dir.join("spk_b_1.wav");
        let silent = temp_dir.join("silent.wav");

        synthesize_voice_wav(&w0, VOICE_120HZ, 4.5);
        synthesize_voice_wav(&w1, VOICE_120HZ, 4.5);
        synthesize_voice_wav(&w2, VOICE_215HZ, 4.5);
        synthesize_voice_wav(&w3, VOICE_215HZ, 4.5);
        write_pcm16_wav(&silent, &vec![0i16; 16_000 * 2]);

        let cancel = Arc::new(AtomicBool::new(false));

        // 1-speaker monologue across multiple windows -> 1 speaker, high speech ratio
        let (spk1, ratio1) =
            analyze_acoustic_wav_slices(&[w0.as_path(), w1.as_path()], &cancel).unwrap();
        assert_eq!(spk1, 1, "Expected 1 speaker for single 120 Hz voice");
        assert!(ratio1 > 0.85, "Expected high speech ratio, got {ratio1}");

        // 2-speaker interview (120 Hz + 215 Hz) across 4 windows -> 2 speakers
        let (spk2, ratio2) = analyze_acoustic_wav_slices(
            &[w0.as_path(), w2.as_path(), w1.as_path(), w3.as_path()],
            &cancel,
        )
        .unwrap();
        assert_eq!(
            spk2, 2,
            "Expected 2 speakers for alternating 120 Hz + 215 Hz voices"
        );
        assert!(ratio2 > 0.85, "Expected high speech ratio, got {ratio2}");

        // Pure silence -> speech ratio == 0.0
        let silent_ratio =
            compute_speech_ratio_from_wav_slices(&[silent.as_path()], &cancel).unwrap();
        assert_eq!(silent_ratio, 0.0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn cancel_flag_preempts_inflight_probe_within_100ms_and_cleans_temp_files() {
        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_probe_cancel_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let temp_wav = temp_dir.join("probe_inflight.wav");
        fs::write(&temp_wav, b"temp-wav-payload").unwrap();

        let state = Arc::new(PipelineState::default());
        let state_bg = Arc::clone(&state);
        let wav_bg = temp_wav.clone();

        let started = Arc::new(tokio::sync::Notify::new());
        let started_bg = Arc::clone(&started);

        let probe_task = tokio::spawn(async move {
            let (cancel_flag, _probe_guard) = state_bg.begin_probe("probe-job-99").await;
            let mut cleanup = TempFileCleanupGuard::new();
            cleanup.track(wav_bg);
            started_bg.notify_one();

            // Simulate an in-flight cooperative probe loop checking cancel_flag
            while !cancel_flag.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Drop cleanup guard and _probe_guard immediately on exit
        });

        started.notified().await;

        let t0 = Instant::now();
        let was_cancelled = state.cancel_active_probe(Some("probe-job-99")).await;
        let elapsed = t0.elapsed();

        assert!(was_cancelled, "Expected active probe to be cancelled");
        assert!(
            elapsed < Duration::from_millis(100),
            "Expected probe preemption in < 100 ms, took {:?}",
            elapsed
        );

        probe_task.await.unwrap();
        assert!(
            !temp_wav.exists(),
            "Temp WAV file must be cleaned up on probe cancellation"
        );

        // Pre-set cancel_flag must abort analyze_acoustic_wav_slices immediately
        let preset_cancel = Arc::new(AtomicBool::new(true));
        let t1 = Instant::now();
        let aborted = analyze_acoustic_wav_slices(&[], &preset_cancel);
        assert!(aborted.is_err());
        assert!(t1.elapsed() < Duration::from_millis(100));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    #[ignore = "Manual real-audio probe verification via EASYSRT_VERIFY_PROBE_FILES"]
    async fn verify_real_media_probe_from_env() {
        let Ok(raw_list) = std::env::var("EASYSRT_VERIFY_PROBE_FILES") else {
            return;
        };
        let ffmpeg_bin = std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| {
                        dir.join(if cfg!(windows) {
                            "ffmpeg.exe"
                        } else {
                            "ffmpeg"
                        })
                    })
                    .find(|p| p.is_file())
            })
            .expect("ffmpeg binary must be present in PATH");

        let temp_dir = std::env::temp_dir().join(format!(
            "easysrt_real_probe_verify_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut cleanup = TempFileCleanupGuard::new();

        for (file_idx, raw_path) in raw_list
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .enumerate()
        {
            let media_path = ffmpeg::validate_approved_media_file(raw_path).unwrap();
            let dur_ms = ffmpeg::probe_media_duration_ms(&ffmpeg_bin, &media_path)
                .await
                .unwrap();
            let plan = compute_probe_window_plan(dur_ms);
            let mut wav_paths = Vec::with_capacity(plan.len());

            for (win_idx, (start_secs, win_dur)) in plan.iter().enumerate() {
                let win_wav = temp_dir.join(format!("real_{file_idx}_{win_idx}.wav"));
                cleanup.track(win_wav.clone());
                let extracted = ffmpeg::extract_pcm16_window_wav(
                    &ffmpeg_bin,
                    &media_path,
                    &win_wav,
                    *start_secs,
                    *win_dur,
                    &cancel,
                )
                .await
                .unwrap();
                if extracted > 0 {
                    wav_paths.push(win_wav);
                }
            }

            let t0 = Instant::now();
            let refs: Vec<&Path> = wav_paths.iter().map(PathBuf::as_path).collect();
            let (speakers, speech_ratio) = analyze_acoustic_wav_slices(&refs, &cancel).unwrap();
            let cpu_ms = t0.elapsed().as_millis();

            let file_name = media_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(raw_path);
            println!(
                "{file_name} | dur_ms={dur_ms} | windows={} | speakers={speakers} | speech_ratio={speech_ratio:.3} | cpu_ms={cpu_ms}",
                wav_paths.len()
            );
        }

        drop(cleanup);
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
