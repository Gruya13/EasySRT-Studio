# Contributing to EasySRT Studio

Thank you for contributing to **EasySRT Studio**! This guide covers local development prerequisites, project layout, and verification steps.

---

## 1. Prerequisites

- **Node.js 22+** and **npm**
- **Rust stable toolchain** (`rustup`, `cargo`, `clippy`)
- **FFmpeg** installed on `PATH` for local media testing
- **Platform WebView dependencies:**
  - **Windows 10/11:** Microsoft Edge WebView2 Runtime (preinstalled on modern Windows) + Visual Studio C++ Build Tools
  - **Ubuntu / Debian Linux:**
    ```bash
    sudo apt-get update && sudo apt-get install -y \
      libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf ffmpeg
    ```
  - **Arch Linux:**
    ```bash
    sudo pacman -S --needed webkit2gtk-4.1 libayatana-appindicator librsvg patchelf ffmpeg
    ```

---

## 2. Running Locally

```bash
# Install frontend dependencies
npm ci

# Start Vite frontend + Tauri desktop window in development mode
npx tauri dev
```

---

## 3. Repository Layout

```text
EasySRT Studio/
├── src/                      # React 19 + TypeScript + Zustand frontend UI
├── src-tauri/                # Tauri v2 + Rust backend (FFmpeg, Whisper, diarizer, LLM, vault)
├── worker/                   # Cloudflare Worker for sanitized crash/bug report deduplication
├── scripts/                  # Zero-dependency release.mjs & docs-check.mjs tooling + unit tests
├── docs/
│   ├── user/{en,sr}/         # Bilingual end-user guides (1:1 structural parity)
│   ├── dev/                  # Permanent engineering docs (ARCHITECTURE, BENCHMARKS, RELEASE_WORKFLOW)
│   └── planning/             # Ephemeral sprint plans (staging branch only; never merged to main)
├── README.md / README.sr.md  # Bilingual public landing pages
├── CHANGELOG.md              # Keep a Changelog user-facing release history
└── AGENTS.md                 # Canonical branching, commit, and documentation rules
```

---

## 4. Branching, Commits & Verification

1. **Target `staging`:** Cut feature/fix branches from `staging` and open Pull Requests against `staging` (`main` is reserved exclusively for release and showcase sync PRs from `staging`).
2. **Conventional Commits:** Use `feat(scope):`, `fix(scope):`, `perf(scope):`, `security(scope):`, `refactor(scope):`, `docs(scope):`, `test(scope):`, or `ci(scope):`.
3. **Bilingual User Docs:** Whenever changing end-user UI or behavior, update both `docs/user/en/*.md` and `docs/user/sr/*.md` (and both `README.md` / `README.sr.md` if applicable).
4. **Run local checks before opening a PR:**
   ```bash
   npm run version:check
   npm run docs:check
   npm audit --audit-level=high
   npm run typecheck
   npm test
   npm run build
   (cd worker && npm ci && npm audit --audit-level=high && npm run typecheck && npm test)
   (cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings)
   ```

For architecture deep-dives and CI/release details, see [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`RELEASE_WORKFLOW.md`](RELEASE_WORKFLOW.md).
