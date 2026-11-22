# EasySRT Studio

Offline-first desktop studio for audio/video transcription, subtitle segmentation, Serbian (Latin & Cyrillic) translation and polishing, and multi-format subtitle export (`.srt`, `.vtt`, `.txt`).

Built with **Tauri v2**, **Rust**, **React**, **TypeScript**, **Vite**, and **Tailwind CSS**.

## Architecture Overview

- **Local AI Pipeline**: `ffmpeg` audio normalization, `whisper.cpp` (CUDA / CPU) transcription with Silero VAD, intelligent subtitle segmentation (Standard / Reels / Custom CPS rules), and local `llama.cpp` or cloud OpenAI-compatible (DeepSeek / Custom) translation + Serbian polishing.
- **Storage Modes**:
  - **Installed**: standard OS app data/config directories.
  - **Portable**: triggered when `portable.flag` sits alongside the executable, keeping `bin/`, `models/`, `data/`, and config self-contained.
- **Resumable Checkpoints**: pipeline phases and translation batches checkpoint locally so long jobs can resume cleanly.

## Local Development

```bash
npm install
npm run typecheck
npm run build
npm run tauri dev
```

Backend checks:

```bash
cd src-tauri
cargo check
cargo test
```

## Release Signing & Auto-Updater Setup

`src-tauri/tauri.conf.json` ships with a placeholder updater public key pointing at GitHub Releases (`https://github.com/Gruya13/EasySRT-Studio/releases/latest/download/latest.json`).

Before cutting a signed release:

1. Generate a Tauri signing keypair:
   ```bash
   npx tauri signer generate -w ~/.tauri/easysrt-studio.key
   ```
2. Replace `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` with the generated `.pub` contents.
3. Add repository secrets in GitHub Settings → Secrets and variables → Actions:
   - `TAURI_SIGNING_PRIVATE_KEY` (contents of `~/.tauri/easysrt-studio.key`)
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (if set during key generation)
4. Push a version tag (`git tag v0.1.0 && git push origin v0.1.0`) or trigger the `Release` GitHub Actions workflow manually.
