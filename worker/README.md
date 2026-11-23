# EasySRT Telemetry Worker (`easysrt-telemetry`)

Cloudflare Worker that receives privacy-sanitized bug reports and crash reports from EasySRT Studio desktop clients and files deduplicated GitHub Issues in `Gruya13/EasySRT-Studio`.

## Live Deployment

- **Production Base URL:** `https://easysrt-telemetry.ig-systems.rs` (Cloudflare zone: `ig-systems.rs`)
- **Health Check:** `GET https://easysrt-telemetry.ig-systems.rs/health`

## Endpoints

- `GET /health` — Health check (`200 { ok: true, service: "easysrt-telemetry" }`).
- `POST /v1/report/bug` — Validates, sanitizes, and opens a GitHub Issue labeled `bug` (`201`).
- `POST /v1/report/crash` — Validates, sanitizes, searches open issues for `crash-fp:<fingerprint>`, and either appends a comment to an existing issue (`200`, `deduplicated: true`) or opens a new issue labeled `crash-report` (`201`).

## Local Development & Verification

```bash
cd worker
npm install
npm run typecheck
npm test
```

## Deploy with Cloudflare CLI (`cf`)

This project uses `cloudflare.config.ts` with the official Cloudflare CLI (`cf`).

1. Authenticate with Cloudflare (one-time):
   ```bash
   cf auth login
   ```
2. Build and deploy the Worker:
   ```bash
   cd worker
   cf deploy
   ```
3. Configure the GitHub Issues secret token (`GITHUB_ISSUES_TOKEN` — a fine-grained PAT scoped strictly to `Issues: Read & Write` on `Gruya13/EasySRT-Studio`):
   ```bash
   npx wrangler secret put GITHUB_ISSUES_TOKEN --name easysrt-telemetry
   ```
   *(Alternatively, upload secrets during deployment via `cf deploy --secrets-file <path-to-untracked-secrets-file>`.)*
4. Tail live logs if needed:
   ```bash
   npx wrangler tail easysrt-telemetry
   ```

> **Security:** Never commit `.dev.vars`, `.env`, or any GitHub token to version control.
