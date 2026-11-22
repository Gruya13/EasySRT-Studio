# EasySRT Studio — Empirical Benchmarks & Engineering Roadmap

This document records the real-world 10-file performance & translation quality benchmarks conducted on **EasySRT Studio**, summarizes the optimizations already shipped in the codebase, and outlines the prioritized engineering roadmap for future releases.

---

## 1. Benchmark Environment & Corpus

All tests were executed on a constrained laptop GPU target to validate consumer hardware viability:
- **GPU**: NVIDIA GeForce RTX 3050 Laptop (**4,096 MiB VRAM**, `computeCapability 8.6 / sm_86`)
- **Whisper ASR Model**: `ggml-large-v3-turbo-q5_0.bin` (574 MB) + Silero VAD `ggml-silero-v6.2.0.bin`
- **Local LLM**: `gemma-3-4b-it-Q4_K_M.gguf` (2.49 GB) via `llama-server` Vulkan (`b12633`, `-np 1 -fitc 2048 -fitt 128 -ctk q8_0 -ctv q8_0`)
- **Cloud LLM**: DeepSeek API (`deepseek-chat`, `batch_size = 25`)
- **Test Corpus (`/home/gru/Downloads/voice_samples/*.mp3`)**: **10 diverse audio tracks** totaling **1,039.8 seconds (17.3 minutes)** and **234 subtitle cues** across US political speech, BBC news, Shakespearean monologues, humorous verse, Australian/Scottish accents, fables, Serbian Ijekavian poetry, and German poetry.

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

## 4. Optimizations Already Implemented

1. **4 GB VRAM Auto-Fit & Crash Recovery (`pipeline::llama_server`, `appStore`)**: Switched `llama-server` startup from fixed `-ngl 99` to `-np 1 -fitc 2048 -fitt 128 -ctk q8_0 -ctv q8_0` with automatic CPU fallback (`-ngl 0`) and partial checkpoint preservation on mid-batch errors.
2. **CUDA / Tuned Whisper Execution (`pipeline::whisper`)**: Capped threads to physical cores and enabled greedy fast decoding (`-bs 1 -bo 1 -nf -fa`).
3. **Keyed `json_schema` Constrained Decoding (`pipeline::translator`)**: Upgraded `PromptCueItem` responses on `llama-server` to strict keyed `json_schema` (`{"1": "...", "2": "..."}`, `additionalProperties: false`) and `batch_size = 25` (`context_window = 2048`), eliminating dropped-ID recursive subdivision retries on 4B models.
4. **Dangling Function-Word Segmentation Penalty (`pipeline::segmenter`)**: Penalized subtitle splits immediately after short connectors, prepositions, and conjunctions (*and, of, the, to, da, je, su, u, na, i, a, za*) for cleaner phrasing.
5. **Expanded Deterministic Ekavian Normalizer (`pipeline::sr_script`)**: Added 140+ exact words and prefix stems (`grijati→grejati`, `vidjel→videl`, `posjet→poset`, `cvjet→cvet`, `potjer→poter`, `prijevod→prevod`, `savjet→savet`, `izvješt→izvešt`, etc.) applied post-LLM prior to Cyrillic transliteration, while guarding valid Ekavian words (`sjedinjenje`, `Sjedinjene`).
6. **Per-Cue `max_chars` CPS Budget & 1-Click "Auto-Fit CPS" (`CondenseCps`)**: Embedded calculated `max_chars` limits inside LLM payloads and wired the `condense_project_cps` backend command + `"Sažmi preduge (CPS)" / "Auto-Fit CPS"` button in `SubtitleTable.tsx` (skips compliant batches and rewrites only offending cues).
7. **Pipelined Cloud Batch Engine (`appStore`, `voice_samples_benchmark.rs`)**: Overlaps sequential GPU Whisper transcription of queued file `i + 1` with asynchronous DeepSeek/CustomOpenAI translation of file `i`.
8. **3 Smart Workflow Presets (`JobWorkspace.tsx`)**: Added single-click mode switching between **Hybrid Turbo (GPU + DeepSeek)**, **100% Offline (Local AI)**, and **Transcribe Only (No LLM)**.

---

## 5. Planned Engineering Roadmap

### Priority 1 — Linux GPU (`Vulkan` / `CUDA`) `whisper-cli` Distribution (`catalog.json` & CI)
- **Current Gap:** Official `ggml.org/whisper.cpp` releases do not ship prebuilt Linux GPU binaries (`whisper-linux-x64` in `src-tauri/resources/catalog.json` is currently `b5907-cpu`, which takes `73.9 s` vs `3.28 s` on a 89 s clip).
- **Plan:**
  - Build static/portable **Vulkan** (`libvulkan.so.1`) and **CUDA 12.x** Linux `whisper-cli` tarballs in `.github/workflows/release.yml` using `-DGGML_VULKAN=ON` (cross-vendor NVIDIA/AMD/Intel) and `-DGGML_CUDA=ON`.
  - Publish hashed assets on GitHub Releases, register Linux GPU entries in `src-tauri/resources/catalog.json`, and run an `ldd`/driver compatibility probe in `setup_manager.rs` with automatic fallback to CPU if system Vulkan/CUDA libs are missing.

### Priority 2 — Two-Phase Offline Local Batch Execution
- **Current Gap:** In offline multi-file batches (`LocalLlama`), sequential `run_full_pipeline` restarts `llama-server` once per audio file (`10 × ~3.64 s = 36.4 s` wasted on startup overhead across 10 files).
- **Plan:** Refactor offline `batch_size > 1` execution into two clean global phases:
  - **Phase 1:** Run FFmpeg + Whisper across all queued files (`whisper-cli` exits and frees VRAM between/after files), saving all checkpoints.
  - **Phase 2:** Start `LlamaServerGuard` **once**, translate/polish every transcribed checkpoint sequentially against the warm KV/weight cache, and stop `llama-server` once at the end. Expected savings: **~33 s per 10 files** (~10% speedup on offline batches).

### Priority 3 — Dual-Column Contextual Source ASR Correction
- **Current Gap:** Even when DeepSeek accurately infers Whisper's phonetic English errors (`"Aesip"` → `"Ezopove basne"`), the `source_text` column still retains `"Aesip"`, which affects dual-language `.source.srt` exports.
- **Plan:** Add an optional pre-translation or combined-schema `corrected_source` field when running cloud LLMs (especially DeepSeek) so users can inspect/accept contextual spelling fixes on English/German source tracks alongside Serbian translations.

### Priority 4 — Interactive Waveform Timeline & Speaker Diarization
- **Plan:**
  - Add a compact canvas waveform scrubber synced with `SubtitleTable` and WebAudio segment playback for dragging cue `[startMs, endMs]` boundaries visually.
  - Evaluate optional lightweight speaker change detection / diarization markers (`[Govornik 1]`) between Silero VAD speech segments.
