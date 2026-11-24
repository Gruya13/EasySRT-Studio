# EasySRT Studio — Agent Operating Rules

Follow these rules on every task in this repository.

## 1. Branching Model (`staging` → `main`)
- **`staging` is the working branch.** Always verify `git branch --show-current` is `staging` (or a short-lived `feat/...` / `fix/...` branch cut from `staging`) before making edits.
- **Never commit or push directly to `main`.** `main` is protected and represents production releases only.
- All changes reach `main` via a Pull Request from `staging` with passing CI.

## 2. Commit Message Convention (Required for Changelog)
Use [Conventional Commits](https://www.conventionalcommits.org/) on `staging` so `scripts/release.mjs` generates accurate release notes automatically:
- `feat(scope): ...` → **Added**
- `fix(scope): ...` → **Fixed**
- `perf(scope): ...` → **Performance**
- `refactor(scope): ...` → **Changed**
- `docs(scope): ...` → **Documentation**
- `chore(scope): ...` / `ci:` / `test:` / `build:` → **Maintenance**
- Add `!` after type/scope (e.g. `refactor(api)!: ...`) or a `BREAKING CHANGE:` footer for breaking changes.

## 3. Version Single Source of Truth
- **Never hardcode version strings in application source code.**
  - Frontend uses `APP_VERSION` from `src/lib/version.ts` (injected from `package.json` via Vite).
  - Rust backend uses `env!("CARGO_PKG_VERSION")`.
- Never bump manifest files by hand; always use `npm run release:prepare`.
- CI enforces synchronization via `npm run version:check`.

## 4. Mandatory Local Verification Before Pushing
```bash
npm run version:check
npm run typecheck
npm test
npm run build
cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings
```

## 5. Staging Pre-release Builds (Safe Testing)
To build Windows NSIS + Linux AppImage installers from `staging` **without** touching the production Tauri auto-updater (`latest.json`):
```bash
gh workflow run release.yml --ref staging -f prerelease=true -f tag_suffix=rc.1
```
(Omit `-f tag_suffix=...` to default to `v<version>-staging.<shortsha>`.)

## 6. Production Release Playbook (`staging` → `main`)
1. Ensure you are on `staging`, up to date with `origin/staging` and `origin/main`, and all feature work is committed.
2. Prepare version bump + `CHANGELOG.md`:
   ```bash
   npm run release:prepare -- patch   # or: minor | major | 0.3.3
   ```
3. Review the generated section in `CHANGELOG.md` (edit wording if helpful for end users), then run full verification (Section 4).
4. Commit and push `staging`:
   ```bash
   git add -A
   git commit -m "chore(release): prepare v$(node -p "require('./package.json').version")"
   git push origin staging
   ```
5. Open a PR from `staging` to `main` and wait for `Frontend & Rust Backend CI` to pass:
   ```bash
   gh pr create --base main --head staging --title "release: v$(node -p "require('./package.json').version")" --body-file <(npm run --silent release:notes -- "$(node -p "require('./package.json').version")")
   ```
6. Merge the PR with a regular merge commit (`gh pr merge --merge`).
   - `.github/workflows/release.yml` automatically detects the new untagged version on `main`, creates tag `vX.Y.Z`, builds Windows NSIS + Linux AppImage + Linux Vulkan tarball, and publishes the GitHub Release with the exact `CHANGELOG.md` notes + `latest.json` updater manifest.
7. Sync `staging` back with `main`:
   ```bash
   git checkout staging && git pull origin staging && git merge origin/main && git push origin staging
   ```

See [docs/RELEASE_WORKFLOW.md](docs/RELEASE_WORKFLOW.md) for complete architecture and troubleshooting details.
