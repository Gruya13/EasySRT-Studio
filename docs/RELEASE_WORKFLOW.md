# EasySRT Studio — Staging & Release Workflow

This document describes the branch strategy, CI gates, staging pre-release builds, and automated release/changelog pipeline for **EasySRT Studio**.

## Branch Topology

| Branch | Role | Protection | Triggers |
| :--- | :--- | :--- | :--- |
| `main` | Production / last shipped release | Protected: PR required, `Frontend & Rust Backend CI` required, no force-push, no delete | `CI` + automatic `Release` if `package.json` version is not yet tagged |
| `staging` | Active development & integration branch | Open for commits & feature PRs | `CI` on every push; manual `workflow_dispatch` for staging pre-releases |
| `feat/*`, `fix/*` | Optional short-lived branches off `staging` | Merged back into `staging` | `CI` on PR |

## Release Tooling (`scripts/release.mjs`)

Zero-dependency Node ESM tool wired into `package.json` scripts and covered by unit tests (`scripts/__tests__/release.test.mjs`):

| Command | Purpose |
| :--- | :--- |
| `npm run version:check` | Verifies `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `worker/package.json`, `worker/package-lock.json`, and `README.md` all declare the exact same version. Runs in CI. |
| `npm run release:prepare -- <patch\|minor\|major\|X.Y.Z> [--dry-run]` | Parses Conventional Commits since the last stable `vX.Y.Z` tag, upserts the new version section in `CHANGELOG.md` (Keep a Changelog format), and syncs all manifest files + `README.md`. Idempotent if re-run for the same version. |
| `npm run release:notes -- <version> [--with-footer] [--prerelease]` | Extracts the exact release notes for `<version>` from `CHANGELOG.md` (or generates commit-based notes for `--prerelease`) for GitHub Releases. |

## Runtime Version Propagation

Source files do not hardcode version strings:
- **Frontend (`src/`):** imports `APP_VERSION` from `src/lib/version.ts`, injected at build/dev time by `vite.config.ts` (`__APP_VERSION__`) from `package.json`.
- **Rust backend (`src-tauri/`):** uses `env!("CARGO_PKG_VERSION")` (automatically synced with `src-tauri/Cargo.toml`).

## How GitHub Actions Automates Releases

### 1. Continuous Integration (`.github/workflows/ci.yml`)
Runs on every push to `main` or `staging` and on every Pull Request:
1. `npm run version:check`
2. `npm run typecheck`
3. `npm test` (frontend + `scripts/release.mjs` unit tests)
4. `npm run build`
5. `cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings`

### 2. Staging Pre-release Builds (Safe for Production Users)
Trigger from `staging` at any time:
```bash
gh workflow run release.yml --ref staging -f prerelease=true -f tag_suffix=rc.1
```
- Creates tag `v<version>-rc.1` (or `v<version>-staging.<shortsha>` if `tag_suffix` is omitted).
- Publishes a **GitHub Pre-release** (`prerelease: true`) with Windows NSIS installer, Linux AppImage, and Linux Vulkan `whisper-cli` tarball.
- Sets `includeUpdaterJson: false` so `/releases/latest/download/latest.json` (read by Tauri's in-app updater in `src-tauri/tauri.conf.json`) is **never** modified.

### 3. Production Release (Merge `staging` → `main`)
1. On `staging`, run `npm run release:prepare -- patch` (or `minor` / `major` / explicit `X.Y.Z`).
2. Review `CHANGELOG.md`, run all local checks, commit (`chore(release): prepare vX.Y.Z`), and push `staging`.
3. Open a PR `staging` → `main` and merge once `Frontend & Rust Backend CI` is green.
4. On push to `main`, `.github/workflows/release.yml`:
   - Checks whether git tag `vX.Y.Z` already exists.
   - If it **already exists** (e.g., docs/CI maintenance PR with no version bump), exits cleanly without rebuilding or publishing.
   - If it **does not exist**, creates and pushes annotated tag `vX.Y.Z`, extracts the exact `CHANGELOG.md` section as the GitHub Release body, and builds/publishes Windows NSIS, Linux AppImage, Linux Vulkan tarball, and `latest.json`.
5. Sync `staging` with `main` (`git checkout staging && git pull && git merge origin/main && git push`).

## Emergency Hotfix Procedure
If production requires a fix before `staging` is ready:
1. Branch `fix/<slug>` from `main`.
2. Commit the fix, run `npm run release:prepare -- patch`, open PR `fix/<slug>` → `main`, and merge after CI passes (auto-releases `vX.Y.(Z+1)`).
3. Merge `origin/main` back into `staging` so the fix and changelog entry are never lost.
