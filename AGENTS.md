# EasySRT Studio — Agent Operating Rules

Follow these rules on every task in this repository.

## 1. Branching Model (`staging` → `main`)
- **`staging` is the working branch.** Always verify `git branch --show-current` is `staging` (or a short-lived `feat/...` / `fix/...` branch cut from `staging`) before making edits.
- **Never commit or push directly to `main`.** `main` is protected and represents production releases and the clean public showcase only.
- All changes reach `main` via a Pull Request from `staging` with passing CI.

## 2. Commit Message Convention (Required for User-Facing Changelog)
Use [Conventional Commits](https://www.conventionalcommits.org/) on `staging` so `scripts/release.mjs` generates clean, end-user-focused release notes automatically:
- `feat(scope): ...` → **Added**
- `fix(scope): ...` → **Fixed**
- `security(scope): ...` / `fix(security): ...` → **Security**
- `perf(scope): ...` → **Performance**
- `refactor(scope): ...` → **Changed**
- `docs(user): ...` → **Documentation** *(user-visible documentation improvements)*
- `chore(scope): ...` / `build:` → **Maintenance**
- **Omitted from public `CHANGELOG.md` automatically:** `docs(planning):`, `docs(dev):`, `chore(planning):`, `chore(docs):`, `test:`, and `ci:` (keeps `CHANGELOG.md` and GitHub Releases free of internal engineering noise).
- Add `!` after type/scope (e.g. `refactor(api)!: ...`) or a `BREAKING CHANGE:` footer for breaking changes.

## 3. Version Single Source of Truth
- **Never hardcode version strings in application source code.**
  - Frontend uses `APP_VERSION` from `src/lib/version.ts` (injected from `package.json` via Vite).
  - Rust backend uses `env!("CARGO_PKG_VERSION")`.
- Never bump manifest or README versions by hand; always use `npm run release:prepare` (syncs `package.json`, lockfiles, `tauri.conf.json`, `Cargo.toml`, `Cargo.lock`, `worker/package*.json`, `README.md`, and `README.sr.md`).
- CI enforces synchronization via `npm run version:check`.

## 4. Documentation Policy & Definition of Done (3-Layer Model)
- **Layer 1 — Public Marketing & User Docs (`EN + SR` 1:1 parity on `main` & `staging`):**
  - `README.md` + `README.sr.md` and `docs/user/en/{guide,troubleshooting}.md` + `docs/user/sr/{guide,troubleshooting}.md`.
  - Both languages **must** keep identical filenames, identical `##` section counts, and be updated together in the same commit/PR (enforced by `npm run docs:check`).
- **Layer 2 — Permanent Engineering Docs (`docs/dev/`, English only):**
  - `docs/dev/{ARCHITECTURE,BENCHMARKS,RELEASE_WORKFLOW,CONTRIBUTING}.md` must reflect the **currently shipped/committed** codebase without stale roadmap wishlists.
- **Layer 3 — Ephemeral Sprint Plans (`docs/planning/`, `staging` ONLY):**
  - Active sprint plans (e.g. `docs/planning/V0.5.0_PLAN.md`) live exclusively on `staging` during development.
  - **`docs/planning/` must never reach `main`** (enforced by `npm run docs:check -- --strict-main` in CI). Before opening a `staging` → `main` PR, distill completed outcomes into `CHANGELOG.md`, `docs/user/*`, and `docs/dev/*`, then `git rm docs/planning/*`.
  - Private business notes, rough drafts, and cross-session agent memory belong in `.wiki/`, never in Git.

| Code / Repo Change | Mandatory Documentation Update in Same Commit / PR |
| :--- | :--- |
| User-facing UI, workflow, format, or shortcut change | `docs/user/en/guide.md` **and** `docs/user/sr/guide.md` (+ `README.md` & `README.sr.md` if headline feature) |
| GPU/driver, API vault/endpoint, or diagnostic behavior change | `docs/user/en/troubleshooting.md` **and** `docs/user/sr/troubleshooting.md` |
| Pipeline, module, security, storage, or VRAM architecture change | `docs/dev/ARCHITECTURE.md` |
| New empirical benchmark or major pipeline speedup | `docs/dev/BENCHMARKS.md` |
| CI, release script, or branching workflow change | `docs/dev/RELEASE_WORKFLOW.md` + `AGENTS.md` |
| Starting a new sprint / release plan on `staging` | `docs/planning/V0.X.Y_PLAN.md` (`docs(planning): ...`) |

## 5. Mandatory Local Verification Before Pushing
```bash
npm run version:check
npm run docs:check
npm audit --audit-level=high
npm run typecheck
npm test
npm run build
(cd worker && npm audit --audit-level=high && npm run typecheck && npm test)
(cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings)
```

## 6. Staging Pre-release Builds (Safe Testing)
To build Windows NSIS + Linux AppImage installers from `staging` **without** touching the production Tauri auto-updater (`latest.json`):
```bash
gh workflow run release.yml --ref staging -f prerelease=true -f tag_suffix=rc.1
```
(Omit `-f tag_suffix=...` to default to `v<version>-staging.<shortsha>`.)

## 7. Production Release Playbook (`staging` → `main`)
1. Ensure you are on `staging`, up to date with `origin/staging` and `origin/main`, and all feature work is committed.
2. Distill and remove any active sprint plans (`git rm -f docs/planning/*` if present) after updating `docs/user/*` and `docs/dev/*`.
3. Prepare version bump + `CHANGELOG.md`:
   ```bash
   npm run release:prepare -- patch   # or: minor | major | 0.4.1
   ```
4. Review the generated section in `CHANGELOG.md` (edit wording if helpful for end users), then run full verification (Section 5) **plus** `npm run docs:check -- --strict-main`.
5. Commit and push `staging`:
   ```bash
   git add -A
   git commit -m "chore(release): prepare v$(node -p "require('./package.json').version")"
   git push origin staging
   ```
6. Open a PR from `staging` to `main` and wait for `Frontend & Rust Backend CI` to pass:
   ```bash
   gh pr create --base main --head staging --title "release: v$(node -p "require('./package.json').version")" --body-file <(npm run --silent release:notes -- "$(node -p "require('./package.json').version")")
   ```
7. Merge the PR with a regular merge commit (`gh pr merge --merge`).
   - `.github/workflows/release.yml` automatically detects the new untagged version on `main`, creates tag `vX.Y.Z`, builds Windows NSIS + Linux AppImage + Linux Vulkan tarball, and publishes the GitHub Release with the exact `CHANGELOG.md` notes + `latest.json` updater manifest. (Documentation-only PRs with no version bump skip binary builds automatically.)
8. Sync `staging` back with `main`:
   ```bash
   git checkout staging && git pull origin staging && git merge origin/main && git push origin staging
   ```

See [docs/dev/RELEASE_WORKFLOW.md](docs/dev/RELEASE_WORKFLOW.md) for complete architecture and troubleshooting details.
