<div align="center">

🌐 **English** | [Srpski](README.sr.md)

# EasySRT Studio

**Fast, Private AI Subtitle Studio for Video Editors, Podcasters & Creators**

*Turn any audio or video into clean, broadcast-ready subtitles in a dual-pane cinema workspace — with 1-click setup, 100% offline AI privacy, or up to **28x real-time** hybrid speed.*

[![Release v0.4.0](https://img.shields.io/badge/release-v0.4.0-38bdf8?style=for-the-badge&logo=github)](https://github.com/Gruya13/EasySRT-Studio/releases/latest)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-10b981?style=for-the-badge)](LICENSE)
[![Platforms](https://img.shields.io/badge/platform-Windows%2010%2F11%20%7C%20Linux-6366f1?style=for-the-badge)](https://github.com/Gruya13/EasySRT-Studio/releases/latest)

### [Download EasySRT Studio v0.4.0](https://github.com/Gruya13/EasySRT-Studio/releases/latest)

[User Guide](docs/user/en/guide.md) · [Troubleshooting & FAQ](docs/user/en/troubleshooting.md) · [Privacy Policy](PRIVACY.md) · [Changelog](CHANGELOG.md)

</div>

---

## Why Creators Choose EasySRT Studio

- **Zero Setup Hassle**: The Windows installer sets up FFmpeg, GPU acceleration, and speech recognition automatically in one step — launch the app, choose a free offline translator or plug in a Cloud API key, and start captioning.
- **Cinema Dual-Pane Editor & 2000% Streaming Timeline**: Edit Source & Translation side-by-side with independent font sizing and automatic checkpoint saving, scrub instant streaming audio/video waveforms, drag cue edges, and zoom smoothly up to **2000%**.
- **Studio-Grade Serbian (`Natural` & `Formal` in 0 ms)**: Switch instantly between natural spoken and formal Ekavian styles without re-translating, clean spoken fillers automatically, tighten reading speed with 1-click **Auto-Fit CPS**, and toggle **Latin ↔ Cyrillic** deterministically.
- **Smart Speaker Splits & Linguistic Wrapping**: Pure-Rust acoustic speaker-turn detection starts fresh cues when speakers change, while linguistic rules split cleanly on commas and wrap **1–4 balanced lines** for TV/Film or Shorts/Reels.
- **Smart Intake Probe & Accent-Resilient Auto-Detect**: Background media probing estimates speaker count and topic context on drop, while `Auto (EN / SR)` prevents non-native accents from drifting into unrelated languages (with manual selection for DE, ES, FR, IT, RU, EL, and more).

---

## Choose Your Workflow Mode

| Mode | Privacy | Speed | Hardware Needed |
| :--- | :--- | :--- | :--- |
| **100% Offline (Local AI)** | Zero cloud traffic — everything stays on disk | ~3x real-time | 4 GB+ NVIDIA GPU (or Vulkan / CPU fallback) |
| **Hybrid Turbo (GPU + Cloud API)** | Audio stays local; only text cues go to DeepSeek / OpenAI | Up to **28x real-time** | Any GPU/CPU + DeepSeek or OpenAI-compatible API key |
| **Transcribe Only (No LLM)** | 100% local speech-to-text & timing | ~27–34x real-time on GPU | Any modern Windows 10/11 or Linux PC |

---

## Get Started in 3 Steps

1. **Install**: Download the **Windows Setup `.exe`** or **Linux `.AppImage`** from **[GitHub Releases](https://github.com/Gruya13/EasySRT-Studio/releases/latest)**.
2. **Pick Your Mode**: Select a free local offline AI model matched to your graphics card, or connect a **DeepSeek / OpenAI-compatible** API key.
3. **Drop Media & Export**: Drag in `.mp3`, `.mp4`, `.mkv`, `.webm`, `.wav`, `.m4a`, `.opus`, `.flac`, or existing `.srt`/`.vtt` subtitles, fine-tune on the timeline, and export studio-ready **`.SRT`**, **`.VTT`**, or **`.TXT`**.

Need keyboard shortcuts, GPU tips, or setup help? Read the **[User Guide](docs/user/en/guide.md)** and **[Troubleshooting & FAQ](docs/user/en/troubleshooting.md)**.

---

## Privacy, Security & Updates

- **Zero-PII Offline-First Core:** Audio, video, raw transcripts, and API keys never leave your computer to IG-Systems. API keys are protected via your OS keyring (or an optional encrypted local vault).
- **Explicit GDPR & Serbian ZZPL Opt-In Telemetry:** Crash reporting, performance/structural metrics, K-anonymous (`k >= 5`) lowercase lexical diffs, and `[NAME]`/`[NUM]`-masked edit-pair donations are **off by default** and include a live **"View Exact JSON Sample"** inspector in the Setup Wizard and Settings (plus `DO_NOT_TRACK=1` / `EASYSRT_TELEMETRY_URL=off` kill-switches).
- **Signed 24h Auto-Updater:** Built-in `minisign`-verified updates check once every 24 hours via our zero-IP proxy (`easysrt-telemetry.ig-systems.rs` with GitHub Releases fallback) and can be toggled anytime in Settings.

Read the full **[Privacy Policy (`PRIVACY.md`)](PRIVACY.md)** for exact schemas, retention periods, and opt-out controls.

---

## License

Licensed under the **[Apache License 2.0](LICENSE)** — Copyright © 2026 **IG-Systems**.

<p><sub>Engineering documentation: <a href="docs/dev/ARCHITECTURE.md">System Architecture</a> · <a href="docs/dev/BENCHMARKS.md">Empirical Benchmarks</a> · <a href="docs/dev/CONTRIBUTING.md">Contributing</a></sub></p>
