//! Pure-Rust acoustic speaker-turn boundary detector (Phase 1, zero external
//! dependencies, 0 MB model download, 0 VRAM).
//!
//! Operates directly on the pipeline's 16 kHz mono PCM16 WAV file, Whisper
//! word timestamps (`TimedWord`), and optional Silero VAD speech intervals to
//! locate inter-word boundaries where the active speaker changes.

use crate::models::TimedWord;
use anyhow::{anyhow, Context, Result};
use std::f64::consts::TAU;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const SAMPLE_RATE: usize = 16_000;
const SAMPLE_RATE_F64: f64 = 16_000.0;
const SAMPLES_PER_MS: usize = SAMPLE_RATE / 1_000;

const FRAME_LEN: usize = 512; // 32 ms at 16 kHz (power of 2 for radix-2 FFT)
const ACF_FFT_LEN: usize = FRAME_LEN * 2; // 1024-pt zero-padded linear autocorrelation
const HALF_FFT: usize = FRAME_LEN / 2;
const HOP_LEN: usize = 320; // 20 ms hop (50 frames/sec)

const MIN_F0_HZ: f64 = 75.0;
const MAX_F0_HZ: f64 = 350.0;
const MIN_PITCH_LAG: usize = 45; // floor(16000 / 350)
const MAX_PITCH_LAG: usize = 214; // ceil(16000 / 75)

const NUM_MEL_BANDS: usize = 16;
const MEL_MIN_HZ: f64 = 80.0;
const MEL_MAX_HZ: f64 = 7_600.0;
const MEL_LOG_FLOOR: f64 = 1e-5;

const MIN_GAP_CANDIDATE_MS: u64 = 180;
const CONTEXT_WINDOW_MS: u64 = 1_500;
const MIN_CLIPPED_WINDOW_MS: u64 = 250;

const MIN_ABS_RMS: f64 = 0.003; // ~ -50 dBFS absolute silence gate
const REL_RMS_RATIO: f64 = 0.15; // ~ -16.5 dB below local window peak
const VOICING_THRESHOLD: f64 = 0.45;
const MPM_PEAK_RATIO: f64 = 0.90;
const MIN_ACTIVE_FRAMES: usize = 4;
const MIN_VOICED_FRAMES: usize = 3;

const BASE_TURN_THRESHOLD: f64 = 0.72;
const SHORT_WINDOW_PENALTY: f64 = 0.08;
const SHORT_WINDOW_FRAME_CUTOFF: usize = 10;
const HYSTERESIS_COOLDOWN_MS: u64 = 750;
const HYSTERESIS_TURN_THRESHOLD: f64 = 1.05;
const MICRO_PEAK_MERGE_MS: u64 = 350;
const MIN_ACOUSTIC_GATE_FOR_PAUSE_BONUS: f64 = 0.42;

/// Microsoft `KSDATAFORMAT_SUBTYPE_PCM` GUID for `WAVE_FORMAT_EXTENSIBLE` (0xFFFE).
const KSDATAFORMAT_SUBTYPE_PCM: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// Detects inter-word boundaries where a new speaker begins.
///
/// Returns sorted, deduplicated 0-based indices `idx` into `words` (`1 <= idx < words.len()`)
/// indicating that `words[idx]` begins a new speaker turn.
pub fn detect_speaker_change_boundaries(
    wav_path: &Path,
    words: &[TimedWord],
    vad_intervals: &[(u64, u64)],
) -> Result<Vec<usize>> {
    if words.len() < 2 {
        return Ok(Vec::new());
    }

    let mut wav = WavPcm16Reader::open(wav_path)?;
    if wav.total_samples < FRAME_LEN {
        return Ok(Vec::new());
    }

    let normalized_vad = normalize_vad_intervals(vad_intervals);
    let candidates = collect_candidate_boundaries(words);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut dsp = DspContext::new();
    let audio_dur_ms = wav.duration_ms();
    let mut accepted: Vec<(usize, u64, f64)> = Vec::new();

    for (cand_pos, &word_idx) in candidates.iter().enumerate() {
        let prev_word = &words[word_idx - 1];
        let next_word = &words[word_idx];
        let gap_ms = next_word.start_ms.saturating_sub(prev_word.end_ms);
        let has_sentence_punct = ends_with_sentence_punct(&prev_word.text);

        let midpoint = prev_word.end_ms / 2
            + next_word.start_ms / 2
            + ((prev_word.end_ms & 1) & (next_word.start_ms & 1));
        let left_end = prev_word.end_ms.min(midpoint).min(audio_dur_ms);
        let right_start = next_word.start_ms.max(midpoint).min(audio_dur_ms);

        let hard_left_floor = accepted
            .last()
            .map(|&(prev_turn_idx, _, _)| words[prev_turn_idx].start_ms)
            .unwrap_or(0);

        let full_left_start = left_end
            .saturating_sub(CONTEXT_WINDOW_MS)
            .max(hard_left_floor);
        let clipped_left_start = cand_pos
            .checked_sub(1)
            .map(|p| words[candidates[p]].start_ms)
            .filter(|&cand_start| {
                cand_start > full_left_start
                    && left_end.saturating_sub(cand_start) >= MIN_CLIPPED_WINDOW_MS
            })
            .unwrap_or(full_left_start);

        let full_right_end = right_start
            .saturating_add(CONTEXT_WINDOW_MS)
            .min(audio_dur_ms);
        let clipped_right_end = candidates
            .get(cand_pos + 1)
            .map(|&next_cand_idx| words[next_cand_idx - 1].end_ms)
            .filter(|&cand_end| {
                cand_end < full_right_end
                    && cand_end.saturating_sub(right_start) >= MIN_CLIPPED_WINDOW_MS
            })
            .unwrap_or(full_right_end);

        let left_feats = extract_window_with_fallback(
            &mut wav,
            &mut dsp,
            clipped_left_start,
            full_left_start,
            left_end,
            &normalized_vad,
        )?;
        let Some(left) = left_feats else {
            continue;
        };

        let right_feats = match extract_window_features(
            &mut wav,
            &mut dsp,
            right_start,
            clipped_right_end,
            &normalized_vad,
        )? {
            Some(feats) => Some(feats),
            None if clipped_right_end < full_right_end => extract_window_features(
                &mut wav,
                &mut dsp,
                right_start,
                full_right_end,
                &normalized_vad,
            )?,
            None => None,
        };
        let Some(right) = right_feats else {
            continue;
        };

        let score = score_speaker_change(&left, &right, gap_ms, has_sentence_punct);
        let min_frames = left.voiced_frames.min(right.voiced_frames);
        let base_threshold = if min_frames < SHORT_WINDOW_FRAME_CUTOFF {
            BASE_TURN_THRESHOLD + SHORT_WINDOW_PENALTY
        } else {
            BASE_TURN_THRESHOLD
        };

        let boundary_time_ms = next_word.start_ms;
        if let Some(&mut (ref mut prev_idx, ref mut prev_time, ref mut prev_score)) =
            accepted.last_mut()
        {
            let elapsed_ms = boundary_time_ms.saturating_sub(*prev_time);
            if elapsed_ms < MICRO_PEAK_MERGE_MS {
                // Local peak picking: suppress micro-split or upgrade to stronger adjacent peak.
                if score > *prev_score && score >= base_threshold {
                    *prev_idx = word_idx;
                    *prev_time = boundary_time_ms;
                    *prev_score = score;
                }
                continue;
            }
            if elapsed_ms < HYSTERESIS_COOLDOWN_MS {
                let hyst_threshold = HYSTERESIS_TURN_THRESHOLD.max(base_threshold);
                if score >= hyst_threshold {
                    accepted.push((word_idx, boundary_time_ms, score));
                }
                continue;
            }
        }

        if score >= base_threshold {
            accepted.push((word_idx, boundary_time_ms, score));
        }
    }

    let mut indices: Vec<usize> = accepted.into_iter().map(|(idx, _, _)| idx).collect();
    indices.sort_unstable();
    indices.dedup();
    Ok(indices)
}

fn extract_window_with_fallback(
    wav: &mut WavPcm16Reader,
    dsp: &mut DspContext,
    clipped_start_ms: u64,
    fallback_start_ms: u64,
    end_ms: u64,
    vad_intervals: &[(u64, u64)],
) -> Result<Option<WindowFeatures>> {
    if let Some(feats) = extract_window_features(wav, dsp, clipped_start_ms, end_ms, vad_intervals)?
    {
        return Ok(Some(feats));
    }
    if fallback_start_ms < clipped_start_ms {
        return extract_window_features(wav, dsp, fallback_start_ms, end_ms, vad_intervals);
    }
    Ok(None)
}

fn ends_with_sentence_punct(text: &str) -> bool {
    let trimmed = text.trim().trim_end_matches(|c: char| {
        matches!(c, '"' | '\'' | '”' | '’' | '»' | ')' | ']')
    });
    matches!(trimmed.chars().last(), Some('.' | '!' | '?' | '…'))
}

fn collect_candidate_boundaries(words: &[TimedWord]) -> Vec<usize> {
    let mut candidates = Vec::new();
    for idx in 1..words.len() {
        let gap_ms = words[idx].start_ms.saturating_sub(words[idx - 1].end_ms);
        if gap_ms >= MIN_GAP_CANDIDATE_MS || ends_with_sentence_punct(&words[idx - 1].text) {
            candidates.push(idx);
        }
    }
    candidates
}

fn normalize_vad_intervals(vad_intervals: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut valid: Vec<(u64, u64)> = vad_intervals
        .iter()
        .copied()
        .filter(|&(s, e)| e > s)
        .collect();
    if valid.is_empty() {
        return valid;
    }
    valid.sort_unstable_by_key(|&(s, e)| (s, e));

    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(valid.len());
    for (s, e) in valid {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}

struct WavPcm16Reader {
    file: File,
    data_offset: u64,
    total_samples: usize,
    raw_buf: Vec<u8>,
}

fn ms_to_sample(ms: u64) -> usize {
    usize::try_from(ms)
        .unwrap_or(usize::MAX)
        .saturating_mul(SAMPLES_PER_MS)
}

impl WavPcm16Reader {
    fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)
            .with_context(|| format!("Failed to open WAV file at {}", path.display()))?;
        let file_len = file
            .metadata()
            .with_context(|| format!("Failed to read WAV metadata at {}", path.display()))?
            .len();

        if file_len < 12 {
            return Err(anyhow!(
                "Invalid WAV file (too small: {file_len} bytes) at {}",
                path.display()
            ));
        }

        let mut riff_hdr = [0u8; 12];
        file.read_exact(&mut riff_hdr)
            .with_context(|| format!("Failed to read RIFF header at {}", path.display()))?;
        if &riff_hdr[0..4] != b"RIFF" || &riff_hdr[8..12] != b"WAVE" {
            return Err(anyhow!(
                "Invalid RIFF/WAVE header in WAV file at {}",
                path.display()
            ));
        }

        let mut pos: u64 = 12;
        let mut fmt_valid = false;
        let mut data_loc: Option<(u64, u64)> = None;

        while pos.saturating_add(8) <= file_len {
            let mut chunk_hdr = [0u8; 8];
            file.read_exact(&mut chunk_hdr)
                .with_context(|| format!("Failed to read WAV chunk header at {}", path.display()))?;
            let chunk_id = &chunk_hdr[0..4];
            let chunk_size = u64::from(u32::from_le_bytes([
                chunk_hdr[4],
                chunk_hdr[5],
                chunk_hdr[6],
                chunk_hdr[7],
            ]));
            let payload_start = pos.saturating_add(8);

            if chunk_id == b"fmt " {
                if chunk_size < 16 || payload_start.saturating_add(16) > file_len {
                    return Err(anyhow!(
                        "Corrupt WAV 'fmt ' chunk (size {chunk_size}) at {}",
                        path.display()
                    ));
                }
                let to_read = (chunk_size.min(40)) as usize;
                let avail = file_len.saturating_sub(payload_start) as usize;
                let read_len = to_read.min(avail);
                if read_len < 16 {
                    return Err(anyhow!("Truncated WAV 'fmt ' chunk at {}", path.display()));
                }
                let mut fmt_buf = [0u8; 40];
                file.read_exact(&mut fmt_buf[..read_len]).with_context(|| {
                    format!("Failed to read WAV 'fmt ' chunk at {}", path.display())
                })?;
                validate_pcm16_mono_16khz_fmt(&fmt_buf[..read_len], path)?;
                fmt_valid = true;
            } else if chunk_id == b"data" {
                let avail = file_len.saturating_sub(payload_start);
                let pcm_bytes = chunk_size.min(avail) & !1;
                data_loc = Some((payload_start, pcm_bytes));
            }

            if fmt_valid && data_loc.is_some() {
                break;
            }

            // RIFF chunks are word-aligned (padded to an even byte boundary).
            let padded_size = chunk_size.saturating_add(chunk_size & 1);
            let Some(next_pos) = payload_start.checked_add(padded_size) else {
                break;
            };
            if next_pos <= pos || next_pos >= file_len {
                break;
            }
            file.seek(SeekFrom::Start(next_pos)).with_context(|| {
                format!("Failed seeking WAV chunks in {}", path.display())
            })?;
            pos = next_pos;
        }

        if !fmt_valid {
            return Err(anyhow!(
                "WAV file missing valid 'fmt ' chunk at {}",
                path.display()
            ));
        }
        let (data_offset, pcm_bytes) = data_loc
            .ok_or_else(|| anyhow!("WAV file missing 'data' chunk at {}", path.display()))?;

        let total_samples = (pcm_bytes / 2) as usize;
        Ok(Self {
            file,
            data_offset,
            total_samples,
            raw_buf: Vec::with_capacity((CONTEXT_WINDOW_MS as usize) * SAMPLES_PER_MS * 2),
        })
    }

    fn duration_ms(&self) -> u64 {
        (self.total_samples / SAMPLES_PER_MS) as u64
    }

    fn read_samples_ms(
        &mut self,
        start_ms: u64,
        end_ms: u64,
        out: &mut Vec<f64>,
    ) -> Result<usize> {
        out.clear();
        if end_ms <= start_ms {
            return Ok(0);
        }

        let start_sample = ms_to_sample(start_ms).min(self.total_samples);
        let end_sample = ms_to_sample(end_ms).min(self.total_samples);
        if end_sample <= start_sample {
            return Ok(start_sample);
        }

        let sample_count = end_sample - start_sample;
        let byte_count = sample_count * 2;
        let byte_offset = self.data_offset.saturating_add((start_sample as u64) * 2);

        self.raw_buf.resize(byte_count, 0);
        self.file.seek(SeekFrom::Start(byte_offset))?;
        self.file.read_exact(&mut self.raw_buf)?;

        out.reserve(sample_count);
        for &[lo, hi] in self.raw_buf.as_chunks::<2>().0 {
            let sample = i16::from_le_bytes([lo, hi]);
            out.push(f64::from(sample) / 32_768.0);
        }
        Ok(start_sample)
    }
}

fn validate_pcm16_mono_16khz_fmt(fmt: &[u8], path: &Path) -> Result<()> {
    let audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);

    let is_pcm = audio_format == 1
        || (audio_format == 0xFFFE
            && fmt.len() >= 40
            && fmt[24..40] == KSDATAFORMAT_SUBTYPE_PCM);

    if !is_pcm {
        return Err(anyhow!(
            "Unsupported WAV audio format 0x{audio_format:04x} (expected PCM16) at {}",
            path.display()
        ));
    }
    if channels != 1 {
        return Err(anyhow!(
            "Unsupported WAV channel count {channels} (expected mono = 1) at {}",
            path.display()
        ));
    }
    if sample_rate != SAMPLE_RATE as u32 {
        return Err(anyhow!(
            "Unsupported WAV sample rate {sample_rate} Hz (expected {SAMPLE_RATE} Hz) at {}",
            path.display()
        ));
    }
    if bits_per_sample != 16 {
        return Err(anyhow!(
            "Unsupported WAV bit depth {bits_per_sample}-bit (expected 16-bit PCM) at {}",
            path.display()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WindowFeatures {
    median_f0_hz: Option<f64>,
    mel_shape: [f64; NUM_MEL_BANDS],
    median_centroid_hz: f64,
    voiced_frames: usize,
}

#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}

struct FftPlan {
    n: usize,
    bit_rev: Vec<usize>,
    tw_cos: Vec<f64>,
    tw_sin: Vec<f64>,
}

impl FftPlan {
    fn new(n: usize) -> Self {
        debug_assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let mut bit_rev = vec![0usize; n];
        for (i, slot) in bit_rev.iter_mut().enumerate() {
            *slot = i.reverse_bits() >> (usize::BITS - bits);
        }

        let half = n / 2;
        let mut tw_cos = Vec::with_capacity(half);
        let mut tw_sin = Vec::with_capacity(half);
        for k in 0..half {
            let angle = TAU * (k as f64) / (n as f64);
            tw_cos.push(angle.cos());
            tw_sin.push(angle.sin());
        }

        Self {
            n,
            bit_rev,
            tw_cos,
            tw_sin,
        }
    }

    fn transform_in_place(&self, buf: &mut [Complex], inverse: bool) {
        debug_assert_eq!(buf.len(), self.n);
        for (i, &rev) in self.bit_rev.iter().enumerate() {
            if i < rev {
                buf.swap(i, rev);
            }
        }

        let mut len = 2usize;
        while len <= self.n {
            let half = len / 2;
            let step = self.n / len;
            for start in (0..self.n).step_by(len) {
                for j in 0..half {
                    let tw_idx = j * step;
                    let wr = self.tw_cos[tw_idx];
                    let wi = if inverse {
                        self.tw_sin[tw_idx]
                    } else {
                        -self.tw_sin[tw_idx]
                    };
                    let u = buf[start + j];
                    let v = buf[start + j + half];
                    let tr = v.re * wr - v.im * wi;
                    let ti = v.re * wi + v.im * wr;
                    buf[start + j] = Complex {
                        re: u.re + tr,
                        im: u.im + ti,
                    };
                    buf[start + j + half] = Complex {
                        re: u.re - tr,
                        im: u.im - ti,
                    };
                }
            }
            len <<= 1;
        }

        if inverse {
            let scale = 1.0 / (self.n as f64);
            for c in buf.iter_mut() {
                c.re *= scale;
                c.im *= scale;
            }
        }
    }
}

struct DspContext {
    fft_mel: FftPlan,
    fft_acf: FftPlan,
    hann: [f64; FRAME_LEN],
    hann_power_norm: f64,
    mel_filters: Vec<Vec<(usize, f64)>>,
    samples: Vec<f64>,
    norm_frame: [f64; FRAME_LEN],
    mel_fft_buf: Vec<Complex>,
    acf_fft_buf: Vec<Complex>,
    power_spec: Vec<f64>,
    sq_prefix: Vec<f64>,
    nacf: Vec<f64>,
    frame_meta: Vec<(usize, f64, f64)>, // (offset, mean, rms)
    voiced_f0s: Vec<f64>,
    voiced_centroids: Vec<f64>,
}

impl DspContext {
    fn new() -> Self {
        let mut hann = [0.0_f64; FRAME_LEN];
        let mut hann_sq_sum = 0.0_f64;
        for (i, w) in hann.iter_mut().enumerate() {
            let val = 0.5 * (1.0 - (TAU * (i as f64) / ((FRAME_LEN - 1) as f64)).cos());
            *w = val;
            hann_sq_sum += val * val;
        }

        let mel_filters = build_mel_filterbank();

        Self {
            fft_mel: FftPlan::new(FRAME_LEN),
            fft_acf: FftPlan::new(ACF_FFT_LEN),
            hann,
            hann_power_norm: (FRAME_LEN as f64) * hann_sq_sum,
            mel_filters,
            samples: Vec::with_capacity((CONTEXT_WINDOW_MS as usize) * SAMPLES_PER_MS),
            norm_frame: [0.0; FRAME_LEN],
            mel_fft_buf: vec![Complex::default(); FRAME_LEN],
            acf_fft_buf: vec![Complex::default(); ACF_FFT_LEN],
            power_spec: vec![0.0; HALF_FFT + 1],
            sq_prefix: vec![0.0; FRAME_LEN + 1],
            nacf: vec![0.0; MAX_PITCH_LAG + 2],
            frame_meta: Vec::with_capacity(96),
            voiced_f0s: Vec::with_capacity(96),
            voiced_centroids: Vec::with_capacity(96),
        }
    }

    /// Estimates fundamental frequency `F0` in `75..=350 Hz` via zero-padded FFT
    /// normalized autocorrelation + McLeod Pitch Method first-peak selection + parabolic refinement.
    fn estimate_pitch_f0(&mut self) -> Option<f64> {
        self.sq_prefix[0] = 0.0;
        for i in 0..FRAME_LEN {
            let s = self.norm_frame[i];
            self.sq_prefix[i + 1] = self.sq_prefix[i] + s * s;
            self.acf_fft_buf[i] = Complex { re: s, im: 0.0 };
        }
        for c in &mut self.acf_fft_buf[FRAME_LEN..ACF_FFT_LEN] {
            *c = Complex::default();
        }

        self.fft_acf.transform_in_place(&mut self.acf_fft_buf, false);
        for c in &mut self.acf_fft_buf {
            c.re = c.re * c.re + c.im * c.im;
            c.im = 0.0;
        }
        self.fft_acf.transform_in_place(&mut self.acf_fft_buf, true);

        let total_sq = self.sq_prefix[FRAME_LEN];
        let mut first_zero_crossing: Option<usize> = None;
        for lag in 1..=(MAX_PITCH_LAG + 1) {
            let e1 = self.sq_prefix[FRAME_LEN - lag];
            let e2 = (total_sq - self.sq_prefix[lag]).max(0.0);
            let denom = (e1 * e2).sqrt();
            let val = if denom > 1e-12 {
                (self.acf_fft_buf[lag].re / denom).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.nacf[lag] = val;
            if first_zero_crossing.is_none() && val <= 0.0 {
                first_zero_crossing = Some(lag);
            }
        }

        let search_start = first_zero_crossing.unwrap_or(MIN_PITCH_LAG).max(MIN_PITCH_LAG);
        if search_start > MAX_PITCH_LAG {
            return None;
        }

        let mut max_peak = 0.0_f64;
        for lag in search_start..=MAX_PITCH_LAG {
            let y0 = self.nacf[lag - 1];
            let y1 = self.nacf[lag];
            let y2 = self.nacf[lag + 1];
            if y1 >= y0 && y1 >= y2 && (y1 > y0 || y1 > y2) && y1 > max_peak {
                max_peak = y1;
            }
        }

        if !max_peak.is_finite() || max_peak < VOICING_THRESHOLD {
            return None;
        }

        // First strong local maximum >= 0.90 * max_peak prevents octave-halving errors.
        let peak_cutoff = (max_peak * MPM_PEAK_RATIO).max(VOICING_THRESHOLD);
        let mut chosen_lag: Option<usize> = None;
        for lag in search_start..=MAX_PITCH_LAG {
            let y0 = self.nacf[lag - 1];
            let y1 = self.nacf[lag];
            let y2 = self.nacf[lag + 1];
            if y1 >= y0 && y1 >= y2 && (y1 > y0 || y1 > y2) && y1 >= peak_cutoff {
                chosen_lag = Some(lag);
                break;
            }
        }

        let lag = chosen_lag?;
        let y0 = self.nacf[lag - 1];
        let y1 = self.nacf[lag];
        let y2 = self.nacf[lag + 1];
        let denom = y0 - 2.0 * y1 + y2;
        let delta = if denom.abs() > 1e-12 {
            (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let refined_lag = (lag as f64) + delta;
        if refined_lag <= 0.0 {
            return None;
        }
        let f0_hz = SAMPLE_RATE_F64 / refined_lag;
        if f0_hz.is_finite() && (MIN_F0_HZ..=MAX_F0_HZ).contains(&f0_hz) {
            Some(f0_hz)
        } else {
            None
        }
    }

    /// Computes energy-normalized 16-band Log-Mel vector and spectral centroid (Hz)
    /// for `self.norm_frame`.
    fn compute_mel_and_centroid(&mut self, log_mel_out: &mut [f64; NUM_MEL_BANDS]) -> f64 {
        for i in 0..FRAME_LEN {
            self.mel_fft_buf[i] = Complex {
                re: self.norm_frame[i] * self.hann[i],
                im: 0.0,
            };
        }
        self.fft_mel.transform_in_place(&mut self.mel_fft_buf, false);

        let bin_hz = SAMPLE_RATE_F64 / (FRAME_LEN as f64);
        let mut weighted_freq = 0.0_f64;
        let mut total_mag = 0.0_f64;

        for k in 1..=HALF_FFT {
            let c = self.mel_fft_buf[k];
            let power = (c.re * c.re + c.im * c.im) / self.hann_power_norm;
            self.power_spec[k] = power;
            let mag = power.max(0.0).sqrt();
            weighted_freq += (k as f64) * bin_hz * mag;
            total_mag += mag;
        }

        for (band_idx, filter) in self.mel_filters.iter().enumerate() {
            let mut band_energy = 0.0_f64;
            for &(bin, weight) in filter {
                band_energy += weight * self.power_spec[bin];
            }
            log_mel_out[band_idx] = band_energy.max(MEL_LOG_FLOOR).ln();
        }

        if total_mag > 1e-12 {
            weighted_freq / total_mag
        } else {
            0.0
        }
    }
}

fn hz_to_mel(hz: f64) -> f64 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f64) -> f64 {
    700.0 * (10.0_f64.powf(mel / 2595.0) - 1.0)
}

fn build_mel_filterbank() -> Vec<Vec<(usize, f64)>> {
    let mel_min = hz_to_mel(MEL_MIN_HZ);
    let mel_max = hz_to_mel(MEL_MAX_HZ);
    let num_points = NUM_MEL_BANDS + 2;
    let mut hz_points = Vec::with_capacity(num_points);
    for i in 0..num_points {
        let frac = (i as f64) / ((num_points - 1) as f64);
        hz_points.push(mel_to_hz(mel_min + frac * (mel_max - mel_min)));
    }

    let bin_hz = SAMPLE_RATE_F64 / (FRAME_LEN as f64);
    let mut filters = Vec::with_capacity(NUM_MEL_BANDS);

    for m in 0..NUM_MEL_BANDS {
        let f_left = hz_points[m];
        let f_center = hz_points[m + 1];
        let f_right = hz_points[m + 2];
        let mut weights: Vec<(usize, f64)> = Vec::new();
        let mut weight_sum = 0.0_f64;

        for k in 1..=HALF_FFT {
            let f = (k as f64) * bin_hz;
            let w = if f > f_left && f <= f_center {
                (f - f_left) / (f_center - f_left).max(1e-9)
            } else if f > f_center && f < f_right {
                (f_right - f) / (f_right - f_center).max(1e-9)
            } else {
                0.0
            };
            if w > 0.0 {
                weights.push((k, w));
                weight_sum += w;
            }
        }

        if weights.is_empty() {
            let nearest_bin = ((f_center / bin_hz).round() as usize).clamp(1, HALF_FFT);
            weights.push((nearest_bin, 1.0));
        } else if weight_sum > 1e-12 {
            for (_, w) in &mut weights {
                *w /= weight_sum;
            }
        }
        filters.push(weights);
    }

    filters
}

fn extract_window_features(
    wav: &mut WavPcm16Reader,
    dsp: &mut DspContext,
    start_ms: u64,
    end_ms: u64,
    vad_intervals: &[(u64, u64)],
) -> Result<Option<WindowFeatures>> {
    if end_ms <= start_ms {
        return Ok(None);
    }

    let base_sample = wav.read_samples_ms(start_ms, end_ms, &mut dsp.samples)?;
    if dsp.samples.len() < FRAME_LEN {
        return Ok(None);
    }

    let sample_len = dsp.samples.len();
    dsp.frame_meta.clear();
    let mut peak_rms = 0.0_f64;

    if vad_intervals.is_empty() {
        collect_frame_energies(&dsp.samples, 0, sample_len, &mut dsp.frame_meta, &mut peak_rms);
    } else {
        for &(vs, ve) in vad_intervals {
            let s_ms = vs.max(start_ms);
            let e_ms = ve.min(end_ms);
            if e_ms <= s_ms {
                continue;
            }
            let rel_s = ms_to_sample(s_ms)
                .saturating_sub(base_sample)
                .min(sample_len);
            let rel_e = ms_to_sample(e_ms)
                .saturating_sub(base_sample)
                .min(sample_len);
            if rel_e >= rel_s.saturating_add(FRAME_LEN) {
                collect_frame_energies(
                    &dsp.samples,
                    rel_s,
                    rel_e,
                    &mut dsp.frame_meta,
                    &mut peak_rms,
                );
            }
        }
    }

    if !peak_rms.is_finite() || peak_rms < MIN_ABS_RMS {
        return Ok(None);
    }

    let energy_gate = (peak_rms * REL_RMS_RATIO).max(MIN_ABS_RMS);
    let mut active_count = 0usize;
    let mut voiced_count = 0usize;
    let mut voiced_mel_sum = [0.0_f64; NUM_MEL_BANDS];
    let mut log_mel_frame = [0.0_f64; NUM_MEL_BANDS];
    dsp.voiced_f0s.clear();
    dsp.voiced_centroids.clear();

    let meta_len = dsp.frame_meta.len();
    for idx in 0..meta_len {
        let (off, mean, rms) = dsp.frame_meta[idx];
        if rms < energy_gate || !rms.is_finite() {
            continue;
        }
        active_count += 1;

        let inv_rms = 1.0 / rms;
        for i in 0..FRAME_LEN {
            dsp.norm_frame[i] = (dsp.samples[off + i] - mean) * inv_rms;
        }

        if let Some(f0_hz) = dsp.estimate_pitch_f0() {
            let centroid = dsp.compute_mel_and_centroid(&mut log_mel_frame);
            for (acc, &val) in voiced_mel_sum.iter_mut().zip(log_mel_frame.iter()) {
                *acc += val;
            }
            dsp.voiced_f0s.push(f0_hz);
            if centroid.is_finite() && centroid > 0.0 {
                dsp.voiced_centroids.push(centroid);
            }
            voiced_count += 1;
        }
    }

    if active_count < MIN_ACTIVE_FRAMES || voiced_count < MIN_VOICED_FRAMES {
        return Ok(None);
    }

    let inv_voiced = 1.0 / (voiced_count as f64);
    let mut mel_shape = [0.0_f64; NUM_MEL_BANDS];
    let mut band_mean = 0.0_f64;
    for (dst, &sum) in mel_shape.iter_mut().zip(voiced_mel_sum.iter()) {
        let v = sum * inv_voiced;
        *dst = v;
        band_mean += v;
    }
    band_mean /= NUM_MEL_BANDS as f64;
    for v in &mut mel_shape {
        *v -= band_mean;
    }

    let median_f0_hz = compute_median(&mut dsp.voiced_f0s);
    let median_centroid_hz = compute_median(&mut dsp.voiced_centroids).unwrap_or(0.0);

    Ok(Some(WindowFeatures {
        median_f0_hz,
        mel_shape,
        median_centroid_hz,
        voiced_frames: voiced_count,
    }))
}

fn collect_frame_energies(
    samples: &[f64],
    rel_start: usize,
    rel_end: usize,
    out: &mut Vec<(usize, f64, f64)>,
    peak_rms: &mut f64,
) {
    let mut off = rel_start;
    let inv_n = 1.0 / (FRAME_LEN as f64);
    while off.saturating_add(FRAME_LEN) <= rel_end {
        let slice = &samples[off..off + FRAME_LEN];
        let mut sum = 0.0_f64;
        for &s in slice {
            sum += s;
        }
        let mean = sum * inv_n;
        let mut sq_sum = 0.0_f64;
        for &s in slice {
            let d = s - mean;
            sq_sum += d * d;
        }
        let rms = (sq_sum * inv_n).max(0.0).sqrt();
        if rms.is_finite() {
            if rms > *peak_rms {
                *peak_rms = rms;
            }
            out.push((off, mean, rms));
        }
        off = off.saturating_add(HOP_LEN);
    }
}

fn compute_median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some(0.5 * (values[mid - 1] + values[mid]))
    }
}

fn cosine_distance(a: &[f64; NUM_MEL_BANDS], b: &[f64; NUM_MEL_BANDS]) -> f64 {
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = (norm_a * norm_b).max(0.0).sqrt();
    if denom <= 1e-12 {
        return 0.0;
    }
    let sim = (dot / denom).clamp(-1.0, 1.0);
    (1.0 - sim).clamp(0.0, 2.0)
}

fn score_speaker_change(
    left: &WindowFeatures,
    right: &WindowFeatures,
    gap_ms: u64,
    has_sentence_punct: bool,
) -> f64 {
    let pitch_semitones = match (left.median_f0_hz, right.median_f0_hz) {
        (Some(f_l), Some(f_r)) if f_l > 1.0 && f_r > 1.0 => (12.0 * (f_l / f_r).log2()).abs(),
        _ => 0.0,
    };
    let pitch_norm = ((pitch_semitones - 1.5).max(0.0) / 5.0).min(1.5);

    let mel_dist = cosine_distance(&left.mel_shape, &right.mel_shape);
    let mel_norm = ((mel_dist - 0.06).max(0.0) / 0.28).min(1.6);

    let centroid_oct = if left.median_centroid_hz > 1.0 && right.median_centroid_hz > 1.0 {
        (left.median_centroid_hz / right.median_centroid_hz)
            .log2()
            .abs()
    } else {
        0.0
    };
    let centroid_norm = ((centroid_oct - 0.15).max(0.0) / 0.65).min(1.4);

    let acoustic_score = 0.50 * pitch_norm + 0.50 * mel_norm + 0.25 * centroid_norm;

    // Pause duration and sentence punctuation reinforce genuine acoustic divergence,
    // but NEVER cause a split on their own during a single-speaker dramatic pause.
    let context_bonus = if acoustic_score >= MIN_ACOUSTIC_GATE_FOR_PAUSE_BONUS {
        let pause_bonus = ((gap_ms as f64).min(1_000.0) / 1_000.0) * 0.16;
        let punct_bonus = if has_sentence_punct { 0.06 } else { 0.0 };
        pause_bonus + punct_bonus
    } else {
        0.0
    };

    let total = acoustic_score + context_bonus;
    if total.is_finite() {
        total
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Instant;

    #[derive(Clone, Copy)]
    struct Formant {
        center_hz: f64,
        bandwidth_hz: f64,
        gain: f64,
    }

    #[derive(Clone, Copy)]
    struct VoiceProfile {
        f0_hz: f64,
        vibrato_depth: f64,
        amplitude: f64,
        formants: [Formant; 3],
    }

    const MALE_PROFILE: VoiceProfile = VoiceProfile {
        f0_hz: 120.0,
        vibrato_depth: 0.025,
        amplitude: 0.55,
        formants: [
            Formant {
                center_hz: 450.0,
                bandwidth_hz: 120.0,
                gain: 1.3,
            },
            Formant {
                center_hz: 1_050.0,
                bandwidth_hz: 180.0,
                gain: 0.9,
            },
            Formant {
                center_hz: 2_400.0,
                bandwidth_hz: 250.0,
                gain: 0.35,
            },
        ],
    };

    const FEMALE_PROFILE: VoiceProfile = VoiceProfile {
        f0_hz: 220.0,
        vibrato_depth: 0.025,
        amplitude: 0.52,
        formants: [
            Formant {
                center_hz: 680.0,
                bandwidth_hz: 140.0,
                gain: 0.95,
            },
            Formant {
                center_hz: 1_950.0,
                bandwidth_hz: 220.0,
                gain: 1.3,
            },
            Formant {
                center_hz: 3_100.0,
                bandwidth_hz: 300.0,
                gain: 0.85,
            },
        ],
    };

    const DARK_150HZ_PROFILE: VoiceProfile = VoiceProfile {
        f0_hz: 150.0,
        vibrato_depth: 0.02,
        amplitude: 0.55,
        formants: [
            Formant {
                center_hz: 380.0,
                bandwidth_hz: 110.0,
                gain: 1.5,
            },
            Formant {
                center_hz: 860.0,
                bandwidth_hz: 160.0,
                gain: 0.95,
            },
            Formant {
                center_hz: 2_100.0,
                bandwidth_hz: 240.0,
                gain: 0.2,
            },
        ],
    };

    const BRIGHT_150HZ_PROFILE: VoiceProfile = VoiceProfile {
        f0_hz: 150.0,
        vibrato_depth: 0.02,
        amplitude: 0.55,
        formants: [
            Formant {
                center_hz: 760.0,
                bandwidth_hz: 150.0,
                gain: 0.45,
            },
            Formant {
                center_hz: 2_150.0,
                bandwidth_hz: 240.0,
                gain: 1.5,
            },
            Formant {
                center_hz: 3_400.0,
                bandwidth_hz: 320.0,
                gain: 1.15,
            },
        ],
    };

    /// Synthesizes a multi-harmonic voiced speech segment with Lorentzian vocal-tract formants
    /// and mild natural pitch intonation into `[start_ms, end_ms]` of `pcm`.
    fn render_voiced_segment(
        pcm: &mut [i16],
        start_ms: u64,
        end_ms: u64,
        profile: VoiceProfile,
    ) {
        let s0 = ((start_ms as usize) * SAMPLES_PER_MS).min(pcm.len());
        let s1 = ((end_ms as usize) * SAMPLES_PER_MS).min(pcm.len());
        if s1 <= s0 {
            return;
        }

        // Precompute per-harmonic gains from glottal tilt * Lorentzian formant envelope.
        let max_harm = ((7_200.0 / profile.f0_hz).floor() as usize).max(1);
        let mut harm_gains = Vec::with_capacity(max_harm);
        for k in 1..=max_harm {
            let freq = (k as f64) * profile.f0_hz;
            let tilt = 1.0 / (k as f64).powf(0.85);
            let mut formant_resp = 0.08;
            for f in &profile.formants {
                let d = (freq - f.center_hz) / (0.5 * f.bandwidth_hz);
                formant_resp += f.gain / (1.0 + d * d);
            }
            harm_gains.push(tilt * formant_resp);
        }

        let len = s1 - s0;
        let fade_samples = (15 * SAMPLES_PER_MS).min(len / 4);
        let mut raw = vec![0.0_f64; len];
        let mut phase = 0.0_f64;
        let mut max_abs = 1e-9_f64;

        for (i, slot) in raw.iter_mut().enumerate() {
            let t = (i as f64) / SAMPLE_RATE_F64;
            let inst_f0 = profile.f0_hz * (1.0 + profile.vibrato_depth * (TAU * 3.5 * t).sin());
            phase += TAU * inst_f0 / SAMPLE_RATE_F64;

            let mut val = 0.0_f64;
            for (h_idx, &g) in harm_gains.iter().enumerate() {
                let k = (h_idx + 1) as f64;
                // Deterministic harmonic phase dispersal avoids unrealistic impulse spikes.
                val += g * (k * phase + k * 0.7).sin();
            }

            let env = if fade_samples > 0 && i < fade_samples {
                0.5 * (1.0 - (PI * (i as f64) / (fade_samples as f64)).cos())
            } else if fade_samples > 0 && i + fade_samples >= len {
                let rem = len.saturating_sub(1 + i);
                0.5 * (1.0 - (PI * (rem as f64) / (fade_samples as f64)).cos())
            } else {
                1.0
            };
            let sample = val * env;
            if sample.abs() > max_abs {
                max_abs = sample.abs();
            }
            *slot = sample;
        }

        let scale = (profile.amplitude / max_abs) * 32_767.0;
        for (i, &val) in raw.iter().enumerate() {
            let mixed = f64::from(pcm[s0 + i]) + val * scale;
            pcm[s0 + i] = mixed.round().clamp(-32_768.0, 32_767.0) as i16;
        }
    }

    fn write_wav_with_extra_chunks(
        path: &Path,
        pcm: &[i16],
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
        audio_format: u16,
        extra_chunks: &[(&[u8; 4], &[u8])],
    ) {
        let data_bytes_len = pcm.len() * 2;
        let mut body = Vec::with_capacity(64 + data_bytes_len);

        body.extend_from_slice(b"WAVE");

        // Optional extra chunks (e.g. JUNK, LIST) including odd-sized chunks for alignment testing
        for &(id, payload) in extra_chunks {
            body.extend_from_slice(id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }

        let block_align = channels.saturating_mul(bits_per_sample / 8);
        let byte_rate = sample_rate.saturating_mul(u32::from(block_align));
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&audio_format.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&sample_rate.to_le_bytes());
        body.extend_from_slice(&byte_rate.to_le_bytes());
        body.extend_from_slice(&block_align.to_le_bytes());
        body.extend_from_slice(&bits_per_sample.to_le_bytes());

        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data_bytes_len as u32).to_le_bytes());
        for &s in pcm {
            body.extend_from_slice(&s.to_le_bytes());
        }

        let mut full = Vec::with_capacity(8 + body.len());
        full.extend_from_slice(b"RIFF");
        full.extend_from_slice(&(body.len() as u32).to_le_bytes());
        full.extend_from_slice(&body);
        fs::write(path, full).unwrap();
    }

    fn write_standard_16k_wav(path: &Path, pcm: &[i16]) {
        write_wav_with_extra_chunks(path, pcm, 16_000, 1, 16, 1, &[]);
    }

    fn make_word(start_ms: u64, end_ms: u64, text: &str) -> TimedWord {
        TimedWord {
            start_ms,
            end_ms,
            text: text.to_string(),
            probability: 0.96,
            speaker_change: false,
        }
    }

    fn temp_wav_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("easysrt_diarizer_{tag}_{}.wav", uuid::Uuid::new_v4()))
    }

    #[test]
    fn detects_male_to_female_speaker_turn_at_exact_boundary() {
        let wav_path = temp_wav_path("male_to_female");
        let total_ms = 5_000usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        // Male speaker: 200..2000 ms, 250 ms pause, Female speaker: 2250..4200 ms
        render_voiced_segment(&mut pcm, 200, 2_000, MALE_PROFILE);
        render_voiced_segment(&mut pcm, 2_250, 4_200, FEMALE_PROFILE);

        // Include odd-sized JUNK and LIST chunks to verify defensive RIFF parsing
        write_wav_with_extra_chunks(
            &wav_path,
            &pcm,
            16_000,
            1,
            16,
            1,
            &[(b"JUNK", b"abc"), (b"LIST", b"INFOISFTLavf")],
        );

        let words = vec![
            make_word(200, 620, "Good"),
            make_word(650, 1_150, "evening"),
            make_word(1_200, 2_000, "everyone."),
            make_word(2_250, 2_750, "Thank"),
            make_word(2_800, 3_350, "you"),
            make_word(3_400, 4_200, "host."),
        ];
        let vad = vec![(180, 2_020), (2_230, 4_220)];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &vad).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert_eq!(boundaries, vec![3]);
    }

    #[test]
    fn single_speaker_with_pause_and_volume_shift_does_not_false_split() {
        let wav_path = temp_wav_path("single_speaker_pause");
        let total_ms = 5_000usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        let speaker_part1 = VoiceProfile {
            f0_hz: 130.0,
            vibrato_depth: 0.03,
            amplitude: 0.32,
            formants: [
                Formant {
                    center_hz: 500.0,
                    bandwidth_hz: 130.0,
                    gain: 1.2,
                },
                Formant {
                    center_hz: 1_250.0,
                    bandwidth_hz: 190.0,
                    gain: 0.95,
                },
                Formant {
                    center_hz: 2_550.0,
                    bandwidth_hz: 260.0,
                    gain: 0.45,
                },
            ],
        };
        // Same speaker after a 450 ms mid-sentence pause: louder (+6 dB) and slightly higher intonation (~136 Hz)
        let speaker_part2 = VoiceProfile {
            f0_hz: 136.0,
            amplitude: 0.68,
            ..speaker_part1
        };

        render_voiced_segment(&mut pcm, 150, 2_000, speaker_part1);
        render_voiced_segment(&mut pcm, 2_450, 4_350, speaker_part2);
        write_standard_16k_wav(&wav_path, &pcm);

        let words = vec![
            make_word(150, 650, "We"),
            make_word(680, 1_300, "decided"),
            make_word(1_340, 2_000, "yesterday,"),
            make_word(2_450, 3_050, "after"),
            make_word(3_100, 3_700, "careful"),
            make_word(3_750, 4_350, "thought."),
        ];
        let vad = vec![(140, 2_010), (2_440, 4_360)];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &vad).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert!(
            boundaries.is_empty(),
            "expected no speaker split for single speaker across 450 ms pause, got {boundaries:?}"
        );
    }

    #[test]
    fn detects_same_pitch_different_timbre_speaker_turn() {
        let wav_path = temp_wav_path("same_pitch_timbre");
        let total_ms = 5_000usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        render_voiced_segment(&mut pcm, 200, 2_000, DARK_150HZ_PROFILE);
        render_voiced_segment(&mut pcm, 2_260, 4_100, BRIGHT_150HZ_PROFILE);
        write_standard_16k_wav(&wav_path, &pcm);

        let words = vec![
            make_word(200, 750, "First"),
            make_word(800, 1_350, "speaker"),
            make_word(1_400, 2_000, "speaks."),
            make_word(2_260, 2_850, "Second"),
            make_word(2_900, 3_450, "speaker"),
            make_word(3_500, 4_100, "replies."),
        ];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &[]).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert_eq!(boundaries, vec![3]);
    }

    #[test]
    fn handles_multi_turn_aba_dialogue_and_short_gap_sentence_punct() {
        let wav_path = temp_wav_path("aba_dialogue");
        let total_ms = 6_500usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        // Male [150..1800] -> Female short turn [1880..2750] (only 80 ms gap, sentence punct!)
        // -> Male [3050..5000] (300 ms gap)
        render_voiced_segment(&mut pcm, 150, 1_800, MALE_PROFILE);
        render_voiced_segment(&mut pcm, 1_880, 2_750, FEMALE_PROFILE);
        render_voiced_segment(&mut pcm, 3_050, 5_000, MALE_PROFILE);
        write_standard_16k_wav(&wav_path, &pcm);

        let words = vec![
            make_word(150, 700, "Are"),
            make_word(740, 1_200, "you"),
            make_word(1_240, 1_800, "ready?"),
            make_word(1_880, 2_300, "Yes,"),
            make_word(2_340, 2_750, "always."),
            make_word(3_050, 3_650, "Great,"),
            make_word(3_700, 4_350, "let's"),
            make_word(4_400, 5_000, "begin."),
        ];
        let vad = vec![(140, 1_810), (1_870, 2_760), (3_040, 5_010)];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &vad).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert_eq!(boundaries, vec![3, 5]);
    }

    #[test]
    fn empty_tiny_words_and_silence_wav_return_empty_without_panicking() {
        let silence_wav = temp_wav_path("silence");
        let pcm_silence = vec![0i16; 3_000 * SAMPLES_PER_MS];
        write_standard_16k_wav(&silence_wav, &pcm_silence);

        let words = vec![
            make_word(100, 800, "Hello."),
            make_word(1_200, 2_100, "World."),
        ];

        assert_eq!(
            detect_speaker_change_boundaries(&silence_wav, &[], &[]).unwrap(),
            Vec::<usize>::new()
        );
        assert_eq!(
            detect_speaker_change_boundaries(&silence_wav, &words[..1], &[]).unwrap(),
            Vec::<usize>::new()
        );
        assert_eq!(
            detect_speaker_change_boundaries(&silence_wav, &words, &[]).unwrap(),
            Vec::<usize>::new()
        );
        let _ = fs::remove_file(&silence_wav);

        // Tiny valid 10 ms WAV (< 1 analysis frame) -> Ok(vec![])
        let tiny_wav = temp_wav_path("tiny");
        write_standard_16k_wav(&tiny_wav, &vec![0i16; 10 * SAMPLES_PER_MS]);
        assert_eq!(
            detect_speaker_change_boundaries(&tiny_wav, &words, &[]).unwrap(),
            Vec::<usize>::new()
        );
        let _ = fs::remove_file(&tiny_wav);
    }

    #[test]
    fn rejects_corrupt_or_unsupported_wav_formats_cleanly() {
        let words = vec![
            make_word(100, 700, "One."),
            make_word(1_000, 1_600, "Two."),
        ];

        // 1. Non-RIFF garbage bytes
        let corrupt = temp_wav_path("corrupt");
        fs::write(&corrupt, b"NOT_A_WAV_FILE_AT_ALL").unwrap();
        assert!(detect_speaker_change_boundaries(&corrupt, &words, &[]).is_err());
        let _ = fs::remove_file(&corrupt);

        // 2. Wrong sample rate (44.1 kHz)
        let wrong_sr = temp_wav_path("wrong_sr");
        write_wav_with_extra_chunks(&wrong_sr, &[0i16; 1024], 44_100, 1, 16, 1, &[]);
        let err = detect_speaker_change_boundaries(&wrong_sr, &words, &[]).unwrap_err();
        assert!(err.to_string().contains("sample rate"));
        let _ = fs::remove_file(&wrong_sr);

        // 3. Stereo (2 channels)
        let stereo = temp_wav_path("stereo");
        write_wav_with_extra_chunks(&stereo, &[0i16; 1024], 16_000, 2, 16, 1, &[]);
        let err = detect_speaker_change_boundaries(&stereo, &words, &[]).unwrap_err();
        assert!(err.to_string().contains("channel count"));
        let _ = fs::remove_file(&stereo);

        // 4. IEEE Float format (format = 3)
        let float_wav = temp_wav_path("ieee_float");
        write_wav_with_extra_chunks(&float_wav, &[0i16; 1024], 16_000, 1, 16, 3, &[]);
        let err = detect_speaker_change_boundaries(&float_wav, &words, &[]).unwrap_err();
        assert!(err.to_string().contains("audio format"));
        let _ = fs::remove_file(&float_wav);

        // 5. Malformed chunk with u32::MAX size must terminate immediately with Err
        let adversarial = temp_wav_path("adversarial");
        let mut bad = Vec::new();
        bad.extend_from_slice(b"RIFF");
        bad.extend_from_slice(&32u32.to_le_bytes());
        bad.extend_from_slice(b"WAVE");
        bad.extend_from_slice(b"JUNK");
        bad.extend_from_slice(&u32::MAX.to_le_bytes());
        bad.extend_from_slice(&[0u8; 16]);
        fs::write(&adversarial, &bad).unwrap();
        assert!(detect_speaker_change_boundaries(&adversarial, &words, &[]).is_err());
        let _ = fs::remove_file(&adversarial);
    }

    #[test]
    fn one_minute_multi_speaker_audio_detects_all_turns_quickly() {
        let wav_path = temp_wav_path("one_minute_perf");
        let total_ms = 60_000usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        // 10 turns of 5.5s each separated by 400 ms pauses (alternating Male / Female)
        let mut words = Vec::new();
        let mut vad = Vec::new();
        let mut expected_turns = Vec::new();

        for turn in 0..10usize {
            let start = (turn as u64) * 5_900 + 200;
            let end = start + 5_400;
            let profile = if turn % 2 == 0 {
                MALE_PROFILE
            } else {
                FEMALE_PROFILE
            };
            render_voiced_segment(&mut pcm, start, end, profile);
            vad.push((start, end));

            if turn > 0 {
                expected_turns.push(words.len());
            }
            for w in 0..6u64 {
                let ws = start + w * 880;
                let we = ws + 760;
                let text = if w == 5 { "end." } else { "word" };
                words.push(make_word(ws, we, text));
            }
        }
        write_standard_16k_wav(&wav_path, &pcm);

        let t0 = Instant::now();
        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &vad).unwrap();
        let elapsed_ms = t0.elapsed().as_millis();
        let _ = fs::remove_file(&wav_path);

        assert_eq!(boundaries, expected_turns);
        // Debug builds run unoptimized; release builds target < 50 ms per minute of audio.
        let budget_ms: u128 = if cfg!(debug_assertions) { 2_500 } else { 50 };
        assert!(
            elapsed_ms <= budget_ms,
            "1-minute diarization took {elapsed_ms} ms (budget {budget_ms} ms)"
        );
    }

    #[test]
    fn pitch_estimator_tracks_multi_harmonic_voices_without_octave_errors() {
        let mut dsp = DspContext::new();
        for &target_f0 in &[85.0, 120.0, 150.0, 220.0, 310.0] {
            let mut pcm = vec![0i16; 1_000 * SAMPLES_PER_MS];
            let profile = VoiceProfile {
                f0_hz: target_f0,
                vibrato_depth: 0.0,
                amplitude: 0.55,
                formants: MALE_PROFILE.formants,
            };
            render_voiced_segment(&mut pcm, 100, 900, profile);

            // Sample middle frame at 400 ms (well away from fades)
            let start = 400 * SAMPLES_PER_MS;
            let slice: Vec<f64> = pcm[start..start + FRAME_LEN]
                .iter()
                .map(|&s| f64::from(s) / 32_768.0)
                .collect();
            let mean = slice.iter().sum::<f64>() / (FRAME_LEN as f64);
            let rms = (slice.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (FRAME_LEN as f64))
                .sqrt();
            for (dst, &s) in dsp.norm_frame.iter_mut().zip(slice.iter()) {
                *dst = (s - mean) / rms;
            }
            let est = dsp
                .estimate_pitch_f0()
                .unwrap_or_else(|| panic!("failed to detect F0 for {target_f0} Hz"));
            assert!(
                (est - target_f0).abs() <= 2.5,
                "expected F0 near {target_f0} Hz, got {est:.2} Hz"
            );
        }
    }

    #[test]
    fn unvoiced_noise_wav_returns_no_false_speaker_changes() {
        let wav_path = temp_wav_path("white_noise");
        let total_ms = 4_000usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];

        // Deterministic LCG pseudo-random broadband noise across both halves
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        for sample in &mut pcm {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let centered = ((state >> 48) as i32) - 32_768;
            *sample = (centered / 4) as i16;
        }
        write_standard_16k_wav(&wav_path, &pcm);

        let words = vec![
            make_word(150, 900, "Static"),
            make_word(950, 1_800, "burst."),
            make_word(2_200, 3_000, "More"),
            make_word(3_050, 3_800, "noise."),
        ];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &[]).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert!(
            boundaries.is_empty(),
            "unvoiced noise should not trigger speaker splits, got {boundaries:?}"
        );
    }

    #[test]
    fn parses_wave_format_extensible_pcm16_wav_cleanly() {
        let wav_path = temp_wav_path("extensible_pcm16");
        let total_ms = 4_500usize;
        let mut pcm = vec![0i16; total_ms * SAMPLES_PER_MS];
        render_voiced_segment(&mut pcm, 200, 1_900, MALE_PROFILE);
        render_voiced_segment(&mut pcm, 2_200, 4_000, FEMALE_PROFILE);

        let data_bytes_len = pcm.len() * 2;
        let mut body = Vec::with_capacity(80 + data_bytes_len);
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&40u32.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE fmt size
        body.extend_from_slice(&0xFFFEu16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes()); // mono
        body.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
        body.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate
        body.extend_from_slice(&2u16.to_le_bytes()); // block align
        body.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        body.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        body.extend_from_slice(&16u16.to_le_bytes()); // wValidBitsPerSample
        body.extend_from_slice(&4u32.to_le_bytes()); // dwChannelMask (front center)
        body.extend_from_slice(&KSDATAFORMAT_SUBTYPE_PCM);
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data_bytes_len as u32).to_le_bytes());
        for &s in &pcm {
            body.extend_from_slice(&s.to_le_bytes());
        }

        let mut full = Vec::with_capacity(8 + body.len());
        full.extend_from_slice(b"RIFF");
        full.extend_from_slice(&(body.len() as u32).to_le_bytes());
        full.extend_from_slice(&body);
        fs::write(&wav_path, full).unwrap();

        let words = vec![
            make_word(200, 1_000, "Hello"),
            make_word(1_050, 1_900, "there."),
            make_word(2_200, 3_000, "Hi"),
            make_word(3_050, 4_000, "back."),
        ];

        let boundaries = detect_speaker_change_boundaries(&wav_path, &words, &[]).unwrap();
        let _ = fs::remove_file(&wav_path);

        assert_eq!(boundaries, vec![2]);
    }
}
