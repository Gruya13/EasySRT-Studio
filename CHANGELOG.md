# Changelog

All notable changes to **EasySRT Studio** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-11-28

### Added
- **Serbian Linguistic Engine & Phonetic ASR Recovery:** Separated `AddressForm` (`Auto`, `Polite 'Vi'`, `Informal 'ti'`) from tone style, added pre-translation domain mini-glossaries (`contextBrief`), domain-neutral Serbian anti-calque rules, Serbian *Pravopis* conjunction-comma hygiene, Whisper topic-context `--prompt` seeding, and automatic source ASR correction when switching to Cloud AI providers (`cb1d1cf`).
- **Timeline `Follow Playhead` & Gap-Hold Selection:** Added a persisted `Follow Playhead` toggle (`#tl-btn-follow`) with smooth viewport auto-scrolling and gap-hold subtitle display so editor panels stay active across inter-cue breathing pauses (`cb1d1cf`).
- **Privacy-First GDPR / ZZPL Telemetry & 24h Auto-Updater:** Added explicit opt-in consent toggles (all defaults off, with a live JSON payload inspector in Setup Wizard and Settings, `v0.4.0` upgrade migration prompt, and `DO_NOT_TRACK=1` / `EASYSRT_TELEMETRY_URL=off` kill-switches), once-per-24h Rust-driven update checks via our cached Cloudflare proxy, zero-click export-time structural diff analysis with optional per-export opt-out pill, and `k >= 5` / subnet-guarded D1 learning analytics (`f7af1fa`, `b52fcf9`, `7578b80`, `5bed69c`, `7f573ce`).

### Fixed
- **Windows Frameless Window & Drag Ergonomics:** Removed the duplicate native Windows OS titlebar (`decorations: false` with startup maximize) and refined `data-tauri-drag-region`, double-click maximize, and resize state syncing (`6f2e42c`).
- **Exclusive `Space` Transport & WebKitGTK Chrome Selection:** Fixed `Space` / `Shift+Space` reopening focused toolbar buttons or file dialogs by adding capture-phase transport guards, and prevented accidental blue text selection on studio headers during timeline scrubbing (`6f2e42c`).
- **Phantom Background Audio Fix & Whisper UTF-8 BPE Decoding:** Replaced eager hidden `<audio>` attachment with lazy fallback loading, `AudioContext.suspend()` on pause, and `MediaSession` neutralization so external OS events never resume paused audio, while fixing split UTF-8 BPE decoding and bounding Whisper JSON size (`cb1d1cf`, `fcbb2f8`).

### Security
- **CSP-Preserving Updater, Opt-In Gates & PII Sanitization:** Routed update checks through Rust without widening browser CSP `connect-src`, enforced opt-in consent on crash/panic paths, bounded/sanitized glossary & Whisper prompt inputs, added sidecar `<job_id>.baseline.json` storage, and enforced sentence-initial `[NAME]` / `[NUM]` redaction, Zero-IP rate limiting, and `k >= 5` / subnet Sybil protection in the telemetry backend (`b52fcf9`, `7578b80`, `5bed69c`, `cb1d1cf`).

### Documentation
- **Privacy Policy & Updated EN/SR Guides:** Added [`PRIVACY.md`](PRIVACY.md) and updated English/Serbian READMEs, user guides, troubleshooting guides, and `ARCHITECTURE.md` with full Wave 1 & Wave 2 coverage (`7e9d74c`, `569d950`, `54e0b9f`).

## [0.4.0] - 2026-11-25

### Added
- **Cinema Dual-Pane Editor & Streaming Timeline:** Redesigned workspace with simultaneous Source & Translation editing, independent font-size controls, `600 ms` debounced checkpoint autosave, and background media intake probing (`db259f7`, `02afec6`, `8858726`).
- **Dual Serbian Translation Style (`Natural` vs. `Formal`):** Added instant 0 ms style switching and dual-style cue caching with distinct idiomatic Ekavian register prompts and expanded lexical normalization (`693a806`, `3e9eca9`, `eefadf9`, `db259f7`, `8858726`).
- **Pure-Rust Acoustic Speaker-Turn Diarizer & Media Probe:** Added acoustic MFCC/pitch speaker-turn detection and sparse background media probing for speaker-count and topic-context classification (`7145450`, `a240dd2`, `02afec6`, `8858726`).
- **Linguistic Segmentation & Line Wrapping:** Added comma-first splitting, leading-conjunction attachment, configurable spoken filler removal, orphan-starter rollback/shift, `80 ms` inter-cue breathing gaps, and linguistic `1–4` line balanced wrapping (`a240dd2`, `3806392`, `4a2d642`).
- **Scoped Audio Playback & Batch Controls:** Added zero-base64 Web Audio + blob media streaming for Linux WebKitGTK and cross-platform codecs, plus clean batch queue cancellation and WebM/Opus file support (`02afec6`, `f980f5c`, `8858726`).

### Fixed
- **Audio Playback:** Restored zero-base64 Web Audio and blob streaming playback for Linux WebKitGTK and cross-platform audio/video codecs (`f980f5c`).
- **Pipeline & Export:** Preserved safe single dots in export filename stems, attached leading discourse fillers cleanly, and improved dual-style batch reuse and cancellation handling (`134c982`, `8858726`).

### Security
- **Tauri & WebView Hardening:** Enabled strict Content Security Policy (CSP), removed unused `fs`/`shell` plugins, and restricted `assetProtocol` scope with dynamic media allowlisting (`02afec6`).
- **Storage, Subprocess & Network Guards:** Hardened credential storage (OS Keyring primary, RAM session fallback, opt-in encrypted local vault/DPAPI), `CustomOpenAi` SSRF/TLS URL validation with zero redirects, ephemeral `llama-server` API keys and private logs, strict `nvidia-smi` paths/timeouts, SHA-256 + HTTPS allowlist enforcement for downloads, and export path-traversal protection (`0d579fa`, `4a2d642`).
- **Dependencies & CI/CD Supply Chain:** Upgraded `zip` to `9.8.1` and `cookie` to `0.18.3` to resolve RustSec advisories, pinned GitHub Actions to commit SHAs and LunarG Vulkan SDK with SHA-256 verification, added `cargo audit` in CI, and hardened telemetry worker CORS/sanitization and UI link safety (`5e4ddbe`, `aa11c33`).

### Maintenance
- **Release Tooling & Automated Quality Gates:** Added the `staging` → `main` release workflow, automated changelog and manifest version synchronization (`scripts/release.mjs`), dynamic runtime version propagation, and sanitized test fixtures (`a89198c`, `eb573be`).

## [0.3.2] - 2026-11-24

### Added
- Timeline mouse-wheel zoom expanded up to 2000% for precision cue-edge editing (`5e26b38`).
- English default UI language on fresh installs with instant EN/SR switching (`5e26b38`).
- Apache License 2.0 adoption and refreshed landing README (`5e26b38`).

### Maintenance
- Cleaned repository documentation for the debut public release (`7c7ca28`).

## [0.3.1] - 2026-11-24

### Added
- Accent-resilient English/Serbian auto-detection (`c1066f6`).
- 1000% timeline wheel zoom and footer bug-report shortcut (`c1066f6`).

## [0.3.0] - 2026-11-23

### Added
- Built-in bug reporting and privacy-safe automatic crash diagnostics (`7b11fc3`).
- Universal Whisper Large V3 Turbo Q5 default and CUDA runtime hardening (`7b11fc3`).

## [0.2.0] - 2026-11-23

### Added
- Automatic one-step setup for core AI runtime (FFmpeg, Whisper, llama.cpp) and Windows VC++ runtime DLLs (`49408f9`).
- Dynamic runtime version display and GitHub signed in-app updater (`c3fa2f8`).

[Unreleased]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/Gruya13/EasySRT-Studio/releases/tag/v0.3.2
[0.3.1]: https://github.com/Gruya13/EasySRT-Studio/commit/c1066f6
[0.3.0]: https://github.com/Gruya13/EasySRT-Studio/commit/7b11fc3
[0.2.0]: https://github.com/Gruya13/EasySRT-Studio/commit/49408f9
