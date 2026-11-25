# EasySRT Studio — Empirical Benchmarks & Engineering Summary

This document records the real-world 10-file performance and translation quality benchmarks conducted on **EasySRT Studio**, alongside a summary of the core performance, linguistic, and security capabilities shipped through `v0.4.0`.

---

## 1. Benchmark Environment & Corpus

All tests were executed on a constrained laptop GPU target to validate consumer hardware viability:
- **GPU**: NVIDIA GeForce RTX 3050 Laptop (**4,096 MiB VRAM**, `computeCapability 8.6 / sm_86`)
- **Whisper ASR Model**: `ggml-large-v3-turbo-q5_0.bin` (574 MB) + Silero VAD `ggml-silero-v6.2.0.bin`
- **Local LLM**: `gemma-3-4b-it-Q4_K_M.gguf` (2.49 GB) via `llama-server` Vulkan (`b12633`, `-np 1 -fitc 2048 -fitt 128 -ctk q8_0 -ctv q8_0`)
- **Cloud LLM**: DeepSeek API (`deepseek-chat`, `batch_size = 25`)
- **Test Corpus (`~/Downloads/voice_samples/*.mp3`)**: **10 diverse audio tracks** totaling **1,039.8 seconds (17.3 minutes)** and **234 subtitle cues** across US political speech, BBC news, Shakespearean monologues, humorous verse, Australian/Scottish accents, fables, Serbian Ijekavian poetry, and German poetry.

---

## 2. Empirical Benchmark Results

### 2.1 ASR Engine Optimization (Single Clip: `01_political_speech_en_us.mp3`, `89.2 s`)

| Whisper Configuration | Execution Time | Speedup vs. Untuned CPU | Notes |
| :--- | :---: | :---: | :--- |
| Prebuilt Linux CPU `whisper-cli` (default flags) | `73.90 s` | `1.0x` | Hyperthreading contention + beam search (`-bs 5`) overhead |
| CPU `whisper-cli` tuned (`-t 8 -bs 1 -bo 1 -nf`) | `43.63 s` | `1.69x` | Greedy decoding + physical core cap (`pipeline::whisper`) |
| Locally compiled CUDA `whisper-cli` (`sm_86`, tuned) | `3.28 s` | **`22.5x`** | `~27–34x` real-time ASR on RTX 3050 4 GB |

### 2.2 End-to-End 10-File Pipeline Progression (`1,039.8 s` Audio, `234` Cues)

| Run | Mode & Stack | Whisper ASR | LLM Inference | Warm Time (RTF) | Cold Wall Time (RTF) | Warning Cues |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: |
| **Run 1** | **Offline Local Baseline** (`Gemma 3 4B`, JSON array, untuned) | `30.3 s` | `570.7 s` | — | **`602.6 s` (`0.58x`)** | — |
| **Run 2** | **Offline Local Optimized** (`Gemma 3 4B`, keyed `json_schema` + `normalize_ekavian`) | `30.2 s` | `261.2 s` (`297.6 s` w/ startup) | `291.4 s` (`0.28x`) | **`329.2 s` (`0.32x`)** | `61 / 234` (`26.1%`) |
| **Run 3** | **Hybrid Sequential** (CUDA Whisper + `DeepSeek API`) | `29.5 s` | `44.4 s` | `73.8 s` (`0.07x`) | **`75.2 s` (`0.07x`)** | `46 / 234` (`19.7%`) |
| **Run 4** | **Hybrid Pipelined + CPS Budget** (`BENCH_PIPELINED=1`, GPU `i+1` overlaps DeepSeek `i`) | `30.2 s` | `41.7 s` (`73.5 s` per-file sum) | `72.0 s` (`0.07x`) | **`36.64 s` (`0.04x`)** | **`39 / 234` (`16.7%`)** |

> **Key Takeaway:** Enabling keyed `json_schema` + GPU layer auto-fit cut offline local latency nearly in half (`602.6 s → 329.2 s`, saving `273.4 s`). Switching translation to DeepSeek with the **Pipelined Cloud Batch Engine** compressed all 17.3 minutes of audio into **36.64 seconds wall clock (`28.4x` faster than real-time)** — an **`8.98x` speedup** over Run 2 and **`16.4x` speedup** over Run 1 — while per-cue `max_chars` budgets reduced high-CPS warnings by **15.2%** (`46 → 39`) prior to running Auto-Fit CPS.

### 2.3 Per-File Breakdown: Local Optimized (Run 2) vs. DeepSeek Pipelined (Run 4)

| File | Audio (s) | Lang / Op | Cues | Local Run 2 Cold (s) | DeepSeek Run 4 Per-File (s) | Local → DeepSeek Warnings |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| `01_political_speech_en_us.mp3` | `89.2` | `en` → `sr` | `17` | `22.66` (`0.25x`) | `6.59` (`0.07x`) | `3` → `4` |
| `02_bbc_radio_news_en_uk.mp3` | `117.8` | `en` → `sr` | `29` | `37.57` (`0.32x`) | `8.70` (`0.07x`) | `12` → `8` |
| `03_dramatic_monologue_male_en.mp3` | `88.8` | `en` → `sr` | `19` | `27.33` (`0.31x`) | `5.92` (`0.07x`) | `7` → `1` |
| `04_dramatic_monologue_female_en.mp3` | `113.1` | `en` → `sr` | `19` | `23.24` (`0.21x`) | `5.74` (`0.05x`) | `4` → `1` |
| `05_humorous_nonsense_en.mp3` | `93.4` | `en` → `sr` | `18` | `27.91` (`0.30x`) | `6.22` (`0.07x`) | `2` → `2` |
| `06_australian_verse_en_au.mp3` | `112.6` | `en` → `sr` | `29` | `42.00` (`0.37x`) | `9.82` (`0.09x`) | `11` → `10` |
| `07_scottish_dialect_en_scots.mp3` | `69.4` | `en` → `sr` | `16` | `37.19` (`0.54x`) | `4.91` (`0.07x`) | `6` → `5` |
| `08_calm_prose_fable_en.mp3` | `118.0` | `en` → `sr` | `34` | `44.06` (`0.37x`) | `10.17` (`0.09x`) | `14` → `7` |
| `09_serbian_poetry_sr.mp3` | `121.9` | `hr/sr` polish | `25` | `34.07` (`0.28x`) | `7.23` (`0.06x`) | `0` → `0` |
| `10_german_poetry_de.mp3` | `115.6` | `de` → `sr` | `28` | `33.15` (`0.29x`) | `8.17` (`0.07x`) | `2` → `1` |
| **Total** | **`1,039.8`** | — | **`234`** | **`329.2` (`0.32x`)** | **`73.5` sum / `36.64 s` wall (`0.04x`)** | **`61` → `39`** |

---

## 3. Translation & Linguistic Quality Comparison (`Gemma 3 4B` vs. `DeepSeek API`)

Spot inspection across the generated `.sr.srt` tracks revealed clear behavioral differences:

| Dimension | Local `gemma-3-4b-it-Q4_K_M` (4 GB Offline) | Cloud `deepseek-chat` (Online) |
| :--- | :--- | :--- |
| **Whisper ASR Homophone Recovery** | Translates phonetic mistakes literally (`08` Cue 1: *"Section 37 of the Aesip for Children"* → `"Sekcija 37 Aesipa za Decu"`). | Infers intended meaning contextually (`08` Cue 1 → **`"Odeljak 37 Ezopovih basni za decu"`**; `09` Cue 5 *"Grki su tamo zaloga"* → **`"Gorki su tamo zalogaj"`**). |
| **Natural Serbian Grammar & Cases** | Occasional gender/case agreement errors (`01` Cue 1: `"Moja časna građanima"`, `02` Cue 2: `"jedna od najstranije stvari"`). | Consistently natural grammar and vocative/genitive inflection (`01` Cue 1: **`"Moji sugrađani,"`**, `02` Cue 1: **`"Ova potera za Nemcima koji u zbrci beže s obala Rajne"`**). |
| **Archaic / Poetic English (`04`)** | Struggles with Early Modern English, repeating source fragments verbatim (`"O Romeo, Romeo, o romeo, \| wherefore art thou romeo?"`, Cue 19 hallucinated line). | Translates accurately and idiomatically (`"O Romeo, Romeo"` / `"zašto si"` / `"druga. Ovaj snimak je u javnom domenu"`). |
| **Ekavian Standard Enforcement** | Pre-dictionary runs leaked Ijekavian words such as `potjeralo`, `grijati`, `posjetite`, `cvjetala`. Deterministic `normalize_ekavian()` now normalizes these automatically. | Follows strict Ekavian strongly by default; deterministic `normalize_ekavian()` guarantees uniformity on remaining slips (e.g., `09` Cue 4 `"grejati"`). |
| **German → Serbian (`10`)** | Mixed literal errors (`"pročeti na nemačkom"`). | Fluent phrasing while occasionally keeping song/poem titles in original German (`"Walküren von Heinrich Heine"`). |

---

## 4. Shipped Architecture & Quality Capabilities (`v0.3.2` → `v0.4.0`)

1. **4 GB VRAM Auto-Fit & Crash Recovery (`pipeline::llama_server`, `appStore`)**: `llama-server` launches with `-np 1 -fitc 2048 -fitt 128 -ctk q8_0 -ctv q8_0`, falls back automatically to CPU (`-ngl 0`) on GPU OOM, and checkpoints progress per batch.
2. **CUDA / Tuned Whisper Execution (`pipeline::whisper`)**: Caps threads to physical CPU cores and runs greedy fast decoding (`-bs 1 -bo 1 -nf -fa` or DTW preset alignment) with concurrent stderr pipe draining.
3. **Keyed `json_schema` Constrained Decoding (`pipeline::translator`)**: Enforces strict keyed `json_schema` (`{"1": "...", "2": "..."}`, `additionalProperties: false`) on local `llama-server` to guarantee 1:1 cue ID alignment and eliminate dropped-ID subdivision retries.
4. **Linguistic Segmentation & `1–4` Line Balanced Wrapping (`pipeline::segmenter`)**: Combines comma-first long-sentence lookahead, leading conjunction and discourse-filler attachment, configurable spoken filler stripping, orphan-starter rollback/shift, `80 ms` inter-cue breathing gaps, and balanced `1–4` line wrapping with negation/preposition/auxiliary guards.
5. **Pure-Rust Acoustic Speaker-Turn Diarizer & Background Media Probe (`pipeline::diarizer`, `pipeline::probe`)**: Zero-VRAM MFCC + MPM pitch autocorrelation speaker-change boundary detector (`0 MB` extra model download) paired with a preemptible 3-stage background intake probe estimating speaker count, language, and topic context.
6. **Dual Serbian Translation Style (`Natural` vs. `Formal`) & Ekavian Normalizer (`pipeline::translator`, `pipeline::sr_script`)**: Instant `0 ms` dual-style switching with independent per-style edit preservation, idiomatic Ekavian register prompts, 140+ exact/stem deterministic Ijekavian→Ekavian normalizations, and smart Cyrillic ↔ Latin conversion.
7. **Per-Cue `max_chars` CPS Budget & 1-Click "Auto-Fit CPS" (`CondenseCps`)**: Embeds calculated per-cue `max_chars` limits inside LLM payloads and provides targeted rewriting (`condense_project_cps`) that skips compliant batches.
8. **Pipelined Cloud Batch & Two-Phase Local Offline Batch (`appStore`, `pipeline::run_local_llm_batch`)**: Overlaps GPU Whisper transcription of file `i + 1` with asynchronous Cloud API translation of file `i` (`28.4x` real-time), while offline batches transcribe all queued files first and reuse a single warm `LlamaServerGuard`.
9. **Dual-Column Source ASR Correction (`JobOptions.correct_source_asr`, `pipeline::translator`)**: Single-pass structured translation mode (`{"1": {"source": "...", "text": "..."}}`) fixing obvious phonetic/homophone speech-to-text slips in `cue.source_text` alongside `cue.translated_text`.
10. **Cinema Dual-Pane Editor & `100%–2000%` Streaming Timeline (`DualEditorBand`, `WaveformTimeline`)**: Side-by-side Source & Translation editing with independent font-size controls, `600 ms` debounced checkpoint autosave, zero-base64 Web Audio + scoped `assetProtocol` / blob streaming playback across Linux WebKitGTK and Windows WebView2, and cursor-anchored wheel zoom up to `2000%`.
11. **Defense-in-Depth Security & Supply-Chain Hardening (`storage.rs`, `setup_manager.rs`, `ci.yml`, `release.yml`)**: Strict Tauri v2 CSP, OS Keyring + RAM session / opt-in DPAPI & encrypted local vault, `CustomOpenAi` HTTPS/SSRF validation with zero redirects, ephemeral `llama-server` bearer auth, SHA-256 + HTTPS download allowlists, SHA-pinned GitHub Actions, and `npm audit` + `cargo audit` CI gates.
