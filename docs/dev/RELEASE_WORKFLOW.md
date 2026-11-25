# EasySRT Studio — Staging & Release Workflow

This document describes the branch strategy, 3-layer documentation gate, CI pipeline, staging pre-release builds, and automated production release workflow for **EasySRT Studio**.

## Branch Topology & Documentation Layers

| Branch | Role | Protection | Documentation Scope |
| :--- | :--- | :--- | :--- |
| `main` | Production / latest shipped release | Protected: PR required, `Frontend & Rust Backend CI` required, no force-push, no delete | **Clean Showcase:** `README.md`, `README.sr.md`, `CHANGELOG.md`, `docs/user/{en,sr}/*`, and permanent `docs/dev/*`. **`docs/planning/` is forbidden.** |
| `staging` | Active development & integration branch | Open for commits & feature PRs | Everything on `main` **plus** ephemeral active sprint plans in `docs/planning/*` (distilled and removed before opening a PR to `main`). |
| `feat/*`, `fix/*` | Optional short-lived branches off `staging` | Merged back into `staging` | Same as `staging`. |

## Release & Documentation Tooling (`scripts/`)

Zero-dependency Node ESM tools wired into `package.json` and covered by unit tests (`scripts/__tests__/*.test.mjs`):

| Command | Purpose |
| :--- | :--- |
| `npm run version:check` | Verifies `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `worker/package.json`, `worker/package-lock.json`, `README.md`, and `README.sr.md` all declare the exact same version. |
| `npm run docs:check` | Verifies `README.md` ↔ `README.sr.md` parity, `docs/user/en/` ↔ `docs/user/sr/` filename & `##` section parity, relative link/image integrity across all Markdown files, no `docs/planning/` leaks in user/marketing docs, `--strict-main` gate, and `--base <ref>` EN/SR co-modification parity. |
| `npm run release:prepare -- <patch\|minor\|major\|X.Y.Z> [--dry-run]` | Parses Conventional Commits since the last stable `vX.Y.Z` tag (automatically omitting internal `docs(planning)`, `docs(dev)`, `test`, and `ci` noise), upserts the new version section in `CHANGELOG.md`, and syncs all manifest files + `README.md` + `README.sr.md`. |
| `npm run release:notes -- <version> [--with-footer] [--prerelease]` | Extracts the exact release notes for `<version>` from `CHANGELOG.md` (or generates commit-based notes for `--prerelease`) for GitHub Releases. |

## Runtime Version Propagation

Source files never hardcode version strings:
- **Frontend (`src/`):** imports `APP_VERSION` from `src/lib/version.ts`, injected at build/dev time by `vite.config.ts` (`__APP_VERSION__`) from `package.json`.
- **Rust backend (`src-tauri/`):** uses `env!("CARGO_PKG_VERSION")` (synced with `src-tauri/Cargo.toml`).

## How GitHub Actions Automates CI & Releases

### 1. Continuous Integration (`.github/workflows/ci.yml`)
Runs on every push to `main` or `staging` and on every Pull Request (all third-party Actions pinned to full commit SHAs):
1. **Fail-fast metadata & docs gate (~2s):** `npm run version:check` + `npm run docs:check` (adds `--strict-main` when targeting `main` and `--base origin/<base_ref>` on PRs).
2. `npm ci` + `npm audit --audit-level=high`
3. `npm run typecheck` + `npm test` + `npm run build`
4. `cd worker && npm ci && npm audit --audit-level=high && npm run typecheck && npm test`
5. `cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings && cargo audit`

### 2. Staging Pre-release Builds (Safe for Production Users)
Trigger from `staging` at any time:
```bash
gh workflow run release.yml --ref staging -f prerelease=true -f tag_suffix=rc.1
```
- Creates tag `v<version>-rc.1` (or `v<version>-staging.<shortsha>` if `tag_suffix` is omitted).
- Publishes a **GitHub Pre-release** (`prerelease: true`) with Windows NSIS installer, Linux AppImage, and Linux Vulkan `whisper-cli` tarball.
- Sets `includeUpdaterJson: false` so `/releases/latest/download/latest.json` (read by Tauri's in-app updater) is **never** modified.

### 3. Production Release (`staging` → `main`)
1. On `staging`, distill any completed sprint plans in `docs/planning/` into `CHANGELOG.md`, `docs/user/{en,sr}/*`, and `docs/dev/*`, then remove active planning files (`git rm docs/planning/*`).
2. Run `npm run release:prepare -- patch` (or `minor` / `major` / explicit `X.Y.Z`).
3. Review `CHANGELOG.md` for user-friendly clarity, run all local verification checks (including `npm run docs:check -- --strict-main`), commit (`chore(release): prepare vX.Y.Z`), and push `staging`.
4. Open a PR `staging` → `main` and merge with `--merge` once `Frontend & Rust Backend CI` is green.
5. On push to `main`, `.github/workflows/release.yml`:
   - Checks whether git tag `vX.Y.Z` already exists.
   - If it **already exists** (e.g., documentation or CI maintenance PR with no version bump), exits cleanly without rebuilding installers or modifying releases.
   - If it **does not exist**, creates and pushes annotated tag `vX.Y.Z`, extracts the exact `CHANGELOG.md` section as the GitHub Release body, and builds/publishes signed Windows NSIS, Linux AppImage, Linux Vulkan tarball, and `latest.json`.
6. Sync `staging` back with `main` (`git checkout staging && git pull origin staging && git merge origin/main && git push origin staging`).

## Emergency Hotfix Procedure
If production requires an urgent fix before `staging` is ready:
1. Branch `fix/<slug>` from `main`.
2. Commit the fix, run `npm run release:prepare -- patch` and `npm run docs:check -- --strict-main`, open PR `fix/<slug>` → `main`, and merge after CI passes (auto-releases `vX.Y.(Z+1)`).
3. Merge `origin/main` back into `staging` so the fix and changelog entry stay synchronized.
