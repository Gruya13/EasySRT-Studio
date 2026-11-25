# Changelog

All notable changes to **EasySRT Studio** will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **Release Tooling & Roadmap:** Added the `staging` → `main` release workflow, automated changelog/version tooling (`scripts/release.mjs`), dynamic version propagation, roadmap plans (`v0.5.0`, Serbian linguistics, privacy-first telemetry), and dynamically assembled synthetic test fixtures (`a89198c`, `496da7c`, `eb573be`).

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

[Unreleased]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/Gruya13/EasySRT-Studio/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/Gruya13/EasySRT-Studio/releases/tag/v0.3.2
[0.3.1]: https://github.com/Gruya13/EasySRT-Studio/commit/c1066f6
[0.3.0]: https://github.com/Gruya13/EasySRT-Studio/commit/7b11fc3
[0.2.0]: https://github.com/Gruya13/EasySRT-Studio/commit/49408f9
