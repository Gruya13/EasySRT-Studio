<div align="center">

# EasySRT Studio

**Local-First AI Subtitle Studio for Video Editors, Podcasters & Creators**

*Transcribe, segment, translate, polish, and fine-tune broadcast-ready subtitles on an interactive 3-track waveform timeline — up to **28.4x faster than real-time** with zero Python, Docker, or manual CUDA setup.*

[![Release v0.3.1](https://img.shields.io/badge/release-v0.3.1-38bdf8?style=for-the-badge&logo=github)](https://github.com/Gruya13/EasySRT-Studio/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-10b981?style=for-the-badge)](LICENSE)
[![Platforms](https://img.shields.io/badge/platform-Windows%2010%2F11%20%7C%20Linux-6366f1?style=for-the-badge)](#1-click-installation)
[![Stack](https://img.shields.io/badge/built%20with-Tauri%20v2%20%E2%80%A2%20Rust%20%E2%80%A2%20React-1f2937?style=for-the-badge)](docs/ARCHITECTURE.md)

[Download Latest Release (v0.3.1)](https://github.com/Gruya13/EasySRT-Studio/releases/latest) · [Architecture](docs/ARCHITECTURE.md) · [Benchmarks & Roadmap](docs/ROADMAP.md) · [Report a Bug](https://github.com/Gruya13/EasySRT-Studio/issues)

</div>

---

## Why Creators Choose EasySRT Studio

Most subtitle tools force a trade-off between slow cloud queues, recurring per-minute subscriptions, broken line timing, and unnatural AI translations. **EasySRT Studio** puts a native Rust + CUDA/Vulkan AI pipeline directly on your desktop:

- **1-Click Smart Installer — Zero Technical Setup**
  The Windows installer (`EasySRT Studio_0.3.1_x64-setup.exe`) automatically detects your hardware and provisions **FFmpeg**, **CUDA 12.4** (or CPU) binaries, **Whisper Large V3 Turbo (Q5_0)**, **llama.cpp**, **Silero VAD v6.2.0**, and required MSVC runtime libraries out of the box. Just open the app, pick your offline translation model or enter a Cloud API key, and drop in your media.
- **Run 100% Offline or Hybrid Turbo (`28.4x` Real-Time)**
  - **🔒 100% Offline (`Local AI`)**: Full local privacy powered by `whisper.cpp` + `llama-server` with sequential VRAM isolation and 4 GB auto-fit (`-np 1 -fitc 2048`). Multi-file batches keep the local LLM warm across all files (`Two-Phase Batch`).
  - **⚡ Hybrid Turbo (`GPU + DeepSeek / Custom OpenAI`)**: Run local GPU Whisper transcription overlapped with pipelined cloud batch translation — transcribing, segmenting, and translating **17.3 minutes of audio in `36.64 s` wall clock** (`0.04x` RTF).
  - **🎙️ Transcribe Only (`No LLM`)**: Ultra-fast Whisper ASR + Silero VAD source caption export at ~3 seconds per minute of audio on RTX GPUs.
- **Smart EN / SR Auto-Detection (Accent-Resilient)**
  Under the default **`Auto (EN / SR)`** mode, EasySRT Studio constrains language auto-detection strictly to **English (`en`)** and **Serbian (`sr`)** — normalizing `hr`/`bs` -> `sr` and comparing EN vs SR confidence if Whisper tentatively picks a third language on heavily accented speech (such as Greek-accented English) — while keeping full explicit support for German (`de`), Spanish (`es`), French (`fr`), Russian (`ru`), Italian (`it`), Greek (`el`), and bilingual Serbian-intro + English-guest clips.
- **Interactive 3-Track Waveform Timeline (`100%–1000%` Cursor-Anchored Zoom)**
  Review and tighten every caption frame-by-frame with a HiDPI acoustic waveform (`MP3`), synchronized `Original` and `Translation` lanes, draggable start/end trim handles, mouse-wheel zoom up to **1000%** anchored at your cursor, and NLE-grade keyboard shortcuts.
- **Broadcast-Ready Timing & 1-Click `Auto-Fit CPS`**
  Word-level segmentation enforces clean reading speeds (**Standard TV/Film `2×42`**, **Shorts/Reels `1×20`**, or **Custom rules**), avoids splitting right after dangling connectors/prepositions, embeds per-cue `max_chars` budgets inside LLM prompts, and offers a 1-click **Auto-Fit CPS** condenser for any overly fast lines.
- **Studio-Grade Serbian Ekavian & Deterministic Script Engine**
  Post-processes Serbian subtitles through 140+ deterministic Ekavian lexical/stem rules (*mlijeko → mleko*, *grijati → grejati*, *prijevod → prevod*), optional Dual-Column Source ASR correction (`correctSourceAsr`), and lossless **Latin ↔ Cyrillic** conversion with accurate digraph casing (`Љ`/`Њ`/`Џ`) and morphological prefix-boundary guards (*injekcija → инјекција*, *Tanjug → Танјуг*).

---

## 1-Click Installation

Download the latest release from **[GitHub Releases](https://github.com/Gruya13/EasySRT-Studio/releases/latest)**:

| Platform | Package | What Happens Automatically |
| :--- | :--- | :--- |
| **Windows 10 / 11 (x64)** | `EasySRT Studio_0.3.1_x64-setup.exe` | Smart NSIS installer installs the app under `%LOCALAPPDATA%\com.easysrt.studio`, provisions MSVC runtime DLLs, detects your NVIDIA driver, and installs FFmpeg, CUDA 12.4 / CPU binaries, Silero VAD v6.2.0, and **Whisper Large V3 Turbo (Q5_0)** cleanly via headless bootstrap. |
| **Linux (x64)** | `EasySRT Studio_0.3.1_amd64.AppImage` | Self-contained desktop bundle with automatic Vulkan / CUDA / CPU Whisper detection and pre-flight shared-library fallback. |

> **NVIDIA GPU Note:** No NVIDIA CUDA Toolkit SDK installation is required — a standard NVIDIA Game Ready or Studio display driver supporting CUDA 12.4+ (`>= 550.54`) is all you need.

On first launch, **Setup & Models** lets you finish your translation setup in one click:
- Choose an **Offline Local LLM** matched to your GPU VRAM (SHA-256 verified with resumable HTTP Range downloads), **or**
- Switch to **DeepSeek API** / **Custom OpenAI-compatible API** (`OpenRouter`, `Ollama`, `vLLM`, `OpenAI`) and save your key directly to your OS credential vault.

---

## Hardware & Model Tiers

**Whisper Large V3 Turbo (Q5_0)** (`574 MB`) is provisioned by default across setups so transcription quality starts high everywhere, while local LLM translators scale cleanly to your detected GPU VRAM (`nvidia-smi`):

| Hardware Tier | Detected VRAM | Default / Optional Whisper ASR | Recommended Local Translation LLM |
| :--- | :--- | :--- | :--- |
| **Studio Pro (`12+ GB`)** | `>= 11,000 MiB` (RTX 3060 12GB, 4070 Ti, 3090, 4090) | `Whisper Large V3 Turbo (Q5_0)` (`574 MB`) or `FP16` (`1.62 GB`) | `Gemma 4 12B IT (QAT Q4_0)` (`6.98 GB`) or `Gemma 3 12B IT (Q4_K_M)` (`7.30 GB`) |
| **Balanced (`8 GB`)** | `5,500 – 10,999 MiB` (RTX 3060 Ti, 3070, 4060 8GB) | `Whisper Large V3 Turbo (Q5_0)` (`574 MB`) | `Gemma 4 E4B IT (QAT Q4_0)` (`5.15 GB`) or `Gemma 3 4B IT (Q8_0)` (`4.13 GB`) |
| **Compact (`4 GB`)** | `3,500 – 5,499 MiB` (RTX 3050 4GB Laptop, GTX 1650) | `Whisper Large V3 Turbo (Q5_0)` (`574 MB`) | `Gemma 3 4B IT (Q4_K_M)` (`2.49 GB`) or `Qwen 3 4B Instruct (Q4_K_M)` (`2.50 GB`) |
| **CPU / Cloud Mode** | `< 3,500 MiB` or No NVIDIA GPU | `Whisper Large V3 Turbo (Q5_0)` / `Whisper Medium (Q5_0)` (`539 MB`) | `DeepSeek Cloud API` (recommended for speed) or `Gemma 3 4B IT` CPU fallback |

> You can also import your own local **`.bin`** (Whisper) or **`.gguf`** (LLM) models from **Settings → Modeli** or point EasySRT Studio to an existing models folder.

---

## Real-World Benchmark Performance

Measured on an everyday consumer laptop (**NVIDIA RTX 3050 4 GB VRAM**) across **10 diverse MP3 tracks totaling `1,039.8 s` (`17.3 min`) and `234` subtitle cues** (political speeches, BBC radio news, Shakespearean monologues, Scottish/Australian accents, Serbian poetry, and German verse):

| Mode & Stack | Whisper ASR | LLM Inference | End-to-End Wall Time (RTF) | Speedup |
| :--- | :---: | :---: | :---: | :---: |
| **100% Offline Baseline** (`Gemma 3 4B`, JSON array) | `30.3 s` | `570.7 s` | `602.6 s` (`0.58x`) | `1.0x` |
| **100% Offline Optimized** (`Gemma 3 4B`, keyed `json_schema` + Ekavian) | `30.2 s` | `261.2 s` | `329.2 s` (`0.32x`) | `1.83x` |
| **Hybrid Sequential** (`CUDA Whisper` + `DeepSeek API`) | `29.5 s` | `44.4 s` | `75.2 s` (`0.07x`) | `8.01x` |
| **⚡ Hybrid Turbo Pipelined** (`CUDA Whisper` overlapped + `DeepSeek` + CPS budget) | `30.2 s` | `41.7 s` *(overlapped)* | **`36.64 s` (`0.04x`)** | **`16.4x` (28.4x real-time)** |

Full per-file timings and linguistic comparisons are documented in [`docs/ROADMAP.md`](docs/ROADMAP.md).

---

## Studio Workflow & Keyboard Shortcuts

- **Supported Media Input**: `.mp3`, `.wav`, `.m4a`, `.flac`, `.ogg`, `.aac`, `.opus`, `.mp4`, `.mkv`, `.webm`
- **Direct Subtitle Import & Export**: Import `.srt` or `.vtt` for instant AI translation, Serbian polishing, script conversion, and timeline trimming; export `.srt`, `.vtt`, and `.txt` (with optional dual source+translated `.srt` and UTF-8 BOM for Premiere/legacy Windows NLEs).
- **Atomic Checkpoints**: Every transcription stage and translation batch is saved atomically to `data/checkpoints/` so interrupted batch jobs resume cleanly without losing completed work.

| Shortcut / Gesture | Action |
| :--- | :--- |
| **`Mouse Wheel Up / Down`** *(over Timeline)* | Zoom timeline smoothly from **`100%` up to `1000%`**, anchored at cursor position |
| **`Shift + Mouse Wheel`** *(over Timeline)* | Pan timeline horizontally |
| **`Space`** / **`Shift + Space`** | Play / Pause from playhead · Play from `00:00` |
| **`C`** | Split active subtitle cue cleanly at the current playhead timestamp |
| **`←` / `→`** · **`Shift + ← / →`** | Step playhead 1 frame (`40 ms`) · Jump `1 second` |
| **`↑` / `↓`** | Jump to and audition next / previous subtitle cue |
| **`Alt + 1`** / **`Alt + 2`** | Jump straight into editing **Original transcript** / **Translation** text |
| **`Ctrl + F`** / **`Ctrl + H`** | Quick search · Search & Replace (`Source`, `Translation`, or `Both` with match-case toggle) |
| **`Ctrl + Z`** / **`Ctrl + Y`** | Undo / Redo timeline & text edits (50-step history) |

---

## Privacy, Auto-Updates & Built-In Diagnostics

- **Local-First by Design**: Audio and video decoding stays **100% on your machine**. When using Cloud API translation (`DeepSeek` or `Custom OpenAI`), EasySRT Studio asks for confirmation before sending subtitle text only, and stores API keys in **Windows Credential Manager / Linux Keyutils** (or `0600` masked fallback storage) — never in `config.json`.
- **Signed In-App Updates**: Built-in **Updater** verifies signed GitHub Release manifests (`latest.json` via `tauri-plugin-updater`) with one-click updates.
- **Bug Reporting & Automatic Crash Recovery**: Click **Prijavi bug / Report Bug** in the TopBar, Settings, or the **bug icon in the footer** next to `IG-Systems` to submit feedback or feature suggestions directly — or save diagnostics locally offline. Crash and bug payloads pass through strict Rust/TypeScript redaction so **API keys, tokens, subtitle text, and local usernames/paths are never transmitted**. Automatic crash reporting can be toggled off anytime in **Settings**.

---

## Development & Building From Source

### Prerequisites
- **Node.js** 22+ and `npm`
- **Rust** (`stable` toolchain) and `cargo`
- **Linux (Ubuntu/Debian)**: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `ffmpeg`, `libvulkan-dev`

```bash
# Install frontend dependencies
npm ci

# Frontend typecheck, unit tests & production build
npm run typecheck
npm test
npm run build

# Rust backend checks & full test suite
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# Run desktop app in development mode
npm run tauri dev
```

For telemetry worker checks, see [`worker/`](worker/) (`npm run typecheck && npm test`).

---

## License

Released under the **[MIT License](LICENSE)** — Copyright © 2026 **IG-Systems**.
