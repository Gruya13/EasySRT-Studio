# EasySRT Studio — Privacy Policy (`PRIVACY.md`)

**Data Controller:** **IG-Systems**  
**Application:** EasySRT Studio (licensed under **[Apache License 2.0](LICENSE)**)  
**Telemetry Endpoint:** `https://easysrt-telemetry.ig-systems.rs` (1st-party Cloudflare Worker & Cloudflare D1)

EasySRT Studio is built on an **offline-first, Zero-PII** architecture. This document explains exactly what never leaves your computer, how optional telemetry tiers work when you explicitly opt in, how the 24-hour signed update check operates, and how to disable all network telemetry completely.

---

## 1. Zero-PII & Offline-First Guarantees

By default, EasySRT Studio operates as a local desktop application. The following data **never leaves your device to IG-Systems servers** under any telemetry setting:

- **Audio & video files** (processed locally via FFmpeg and `whisper.cpp`)
- **Full subtitle transcripts or `.srt` / `.vtt` / `.txt` files**
- **API keys, bearer tokens, and OS keyring credentials**
- **Usernames, home directory paths, and hardware serials / machine IDs**
- **Persistent tracking identifiers, cookies, or third-party analytics SDKs** (no Google Analytics, Mixpanel, or Sentry SaaS)

> **Note on Optional Cloud Translation:** If you explicitly configure **DeepSeek API** or a **Custom OpenAI-compatible** endpoint in Settings, subtitle cue text is sent directly from your computer to that provider under your own API key and that provider's terms. IG-Systems never proxies or sees your Cloud translation traffic.

---

## 2. Explicit Opt-In Consent Model (GDPR & Serbian ZZPL)

EasySRT Studio complies with **GDPR** (Art. 6(1)(a), Art. 7), **ePrivacy** (Art. 5(3)), and the **Serbian Law on Personal Data Protection (*Zakon o zaštiti podataka o ličnosti*, ZZPL Art. 12 & Art. 15)**:

1. **Three Independent Defaults-Off Toggles:** In both the **Setup Wizard** and **Settings → General**, all diagnostic and telemetry toggles start **`false` (unchecked)**:
   - **Crash Reports (`autoSendCrashReports`)** — sends redacted Rust panic / subprocess crash diagnostics (never subtitle text) to `POST /v1/report/crash`.
   - **Anonymous Usage & Quality Metrics (`shareAnonymousUsageMetrics`)** — enables **Level 1**, **Level 2A**, and **Level 2B** telemetry.
   - **Anonymized Translation Edit Pairs (`shareAnonymizedTranslationEdits`)** — enables **Level 3** `[NAME]`/`[NUM]`-masked edit pair donation on Export.
2. **Consent Versioning (`privacyConsentVersion`) & `v0.4.0` Upgrade Migration:**
   - Explicit consent is recorded locally in `config.json` via `privacyConsentVersion` (`1` in `v0.5.0+`).
   - In `v0.4.0` and earlier, `autoSendCrashReports` defaulted to `true` without a consent version check. When upgrading from `v0.4.0` (`privacyConsentVersion: 0`), **crash report uploads are automatically suppressed** until you explicitly review and confirm your choices in Settings (`privacyConsentVersion >= 1` AND toggle enabled).
3. **Live JSON Payload Inspector (*"View Exact JSON Sample"*):**
   - Right beside the toggles in both the **Setup Wizard** and **Settings → General**, an expandable inspector calls the local Rust backend (`get_telemetry_sample_payloads`) to display the exact sanitized JSON structure for crash reports, Level 1/2A/2B metrics, and Level 3 training pairs—and shows immediately whether environment kill-switches are active.
4. **1-Click Revocation:** You may disable any toggle at any time in **Settings → General** with immediate effect.

---

## 3. Data Collected by Tier

### Level 0 — 24-Hour Signed Auto-Update Check & Anonymous DAU Counter
- **Default:** Enabled (`autoCheckUpdates: true`, standard functional update delivery under GDPR Art. 6(1)(f) / ZZPL; can be toggled off in **Settings → General**).
- **How it works:** At most once every **24 hours** (`86,400` seconds, tracked locally via `lastUpdateCheckEpochSec`), the Rust backend checks for a new signed release via:
  - Primary proxy: `https://easysrt-telemetry.ig-systems.rs/v1/update/latest.json?target={{target}}&arch={{arch}}&current={{current_version}}`
  - Automatic fallback: `https://github.com/Gruya13/EasySRT-Studio/releases/latest/download/latest.json`
- **Cryptographic Verification:** All update manifests and installers are verified end-to-end using Tauri's embedded `minisign` public key; the Worker returns upstream manifest bytes verbatim.
- **What is counted:** The Worker reads only the UTC date (`YYYY-MM-DD`), allowlisted OS target (`windows`, `linux`, `darwin`), architecture (`x86_64`, `aarch64`, `i686`, `armv7`), and semver `current_version` to increment an aggregated daily counter (`daily_update_checks` in Cloudflare D1).
- **Zero IP Storage & Worker Observability:** `CF-Connecting-IP` is **never** written to D1 or persistent logs. Ephemeral rate limiting hashes client IPs in memory/Cache API using `SHA-256(ip + ":" + utcDate + ":" + dailySalt)`, and Cloudflare Worker observability configuration ([`worker/cloudflare.config.ts`](worker/cloudflare.config.ts)) is aligned with zero-PII logging.

### Level 1 — Hardware, Runtime & Pipeline RTF Performance (`POST /v1/telemetry/job-metrics`)
- **Requires:** Explicit opt-in (`shareAnonymousUsageMetrics: true` and `privacyConsentVersion >= 1`).
- **Payload (`~400 bytes`, `<= 32 KiB` cap, zero text):** `appVersion`, `os`, `gpuTier` (`cpuOnly`, `vram4Gb`, `vram8Gb`, `vram12GbPlus`), `whisperBackend` (`cuda`, `vulkan`, `cpu`), `whisperModelId`, `llmProvider`, `llmModelId`, `audioDurationMs`, `whisperElapsedMs`, `diarizerElapsedMs`, `llmElapsedMs`, `asrRtf`, `llmRtf`, `oomCpuFallback`, `batchSubdivisionRetries`, `cueCount`, `sourceLang`, `targetLang`, `translationStyle`, `addressForm`, `preset`.

### Level 2A — Structural, Grammatical & Timing Edit Classifiers (`POST /v1/telemetry/export`)
- **Requires:** Explicit opt-in (`shareAnonymousUsageMetrics: true` and `privacyConsentVersion >= 1`).
- **How it works:** When you export `.srt`, `.vtt`, or `.txt`, the Rust diff engine (`telemetry_diff.rs`) compares the initial AI baseline sidecar (`<job_id>.baseline.json`) against your final cues locally in `< 5 ms` (deduplicated per `(job_id, exported_hash)`).
- **Payload (`<= 32 KiB` cap, zero subtitle words):** Only integer counters and millisecond averages:
  - Cue totals & edit rate (`totalBaselineCues`, `totalFinalCues`, `unchangedCuesCount`, `textEditedCuesCount`, `editRatePct`)
  - Reading speed & line-length deltas (`initialHighCpsCount`, `finalHighCpsCount`, `initialLineTooLongCount`, `finalLineTooLongCount`, `avgCpsBefore`, `avgCpsAfter`, `condenseCpsUsed`)
  - Linguistic & Serbian grammar classifiers (`encliticReorderedCount`, `redundantPossessiveRemovedCount`, `passiveCalqueRemovedCount`, `conjunctionCommaFixedCount`, `missedFillerDeletedCount`, `inflectionSuffixFixCount`, `boundaryOrphanShiftedCount`)
  - Segmentation & timeline trim ergonomics (`splitCount`, `splitAfterCommaCount`, `splitAfterConjunctionCount`, `avgPreSplitChars`, `avgPreSplitDurationMs`, `mergeCount`, `avgMergedShortCueDurationMs`, `avgMergedShortCueChars`, `avgMergedGapMs`, `startTrimCount`, `avgStartDeltaMs`, `endTrimCount`, `avgEndDeltaMs`).

### Level 2B — K-Anonymous (`k >= 5`) Lowercase Lexical Diffs (`POST /v1/telemetry/export`)
- **Requires:** Explicit opt-in (`shareAnonymousUsageMetrics: true` and `privacyConsentVersion >= 1`).
- **Purpose:** Automatically detects recurring AI phrasing mistakes (e.g., `"tijekom" -> "tokom"`) without exposing names, numbers, or unique sentences.
- **Strict Client-Side Privacy Filter:** Operates only on `1:1` matched cues where `>= 50%` of tokens stayed unchanged, extracting minimal `1–3` word diffs (`1 <= old_words <= 3`, `0 <= new_words <= 3`) normalized to Serbian Latin. A diff is **immediately discarded locally** if either span contains:
  - **Any uppercase character** (`c.is_uppercase()` — blocks all personal names, surnames, places, brands, and acronyms)
  - **Any digit** (`c.is_numeric()`), `@`, `/`, `\`, `.com`, or non-alphabetic symbols (except spaces, hyphens, and apostrophes)
  - Any token shorter than 2 characters (unless paired) or longer than 24 characters
  - Any key or value from your project glossary
- **Unlinkable Monthly Cohort Salt:** Client generates a random `telemetryCohortSalt` rotated every UTC calendar month (`telemetryCohortMonth`), never tied to hardware IDs, and sends `cohortProof = HMAC/SHA256(monthly_salt, lang_pair + "|" + old_span + "->" + new_span)[..24]`.
- **Sybil-Resistant K-Anonymity Gate & 30-Day TTL Purge:**
  - The Worker stores `(pair_hash, cohort_proof, subnet_hash)` in `ngram_cohorts` using an HMAC of the `/24` IPv4 or `/48` IPv6 prefix (`SUBNET_HMAC_SECRET`), never storing raw IPs.
  - A candidate phrase is **hidden from all maintainer queries** unless it has been independently observed by **`k >= 5` distinct cohorts across `>= 3` distinct blinded subnets**.
  - Any unpromoted candidate (`cohort_count < 5 OR subnet_count < 3`) is **permanently purged after 30 days** by the daily Cloudflare Worker Cron Trigger (`0 3 * * *`).

### Level 3 — Optional Anonymized Edit-Pair Donation (`POST /v1/telemetry/training-pairs`)
- **Requires:** Explicit opt-in (`shareAnonymizedTranslationEdits: true` and `privacyConsentVersion >= 1`).
- **Payload (`<= 64 KiB` cap):** Up to **25** modified `(sourceText, initialAiText, finalHumanText)` pairs per export along with `appVersion`, `sourceLang`, `targetLang`, `style`, and `addressForm`.
- **Local PII & Proper-Noun Sanitizer:** Before leaving your PC, the Rust sanitizer strips emails, phone numbers, URLs, API tokens, and file paths, replaces multi-digit numbers with `[NUM]`, and replaces glossary names and capitalized proper nouns (matched project-wide across both sentence-initial and mid-sentence positions) with `[NAME]`. The Worker runs an additional server-side sanitization pass before storing in `training_edit_pairs`.
- **Per-Export Skip Pill:** Whenever Level 3 is enabled and modified cues exist (`N > 0`), the Export modal displays a non-blocking footer notice with a 1-click **"Skip for this export"** pill so confidential jobs can bypass sharing immediately.
- **90-Day Retention:** Stored rows in `training_edit_pairs` are **automatically purged after 90 days** by the daily Cron Trigger (`0 3 * * *`).

### Manual Bug Reports & Crash Reports (`POST /v1/report/bug`, `POST /v1/report/crash`)
- Sent only when you explicitly submit a bug/feedback form or explicitly opt in to crash reporting (`privacyConsentVersion >= 1` and `autoSendCrashReports: true`).
- Client-side and server-side sanitizers redact usernames, home paths, URL credentials, API keys, and bearer tokens before filing deduplicated issues in `Gruya13/EasySRT-Studio`. When offline or disabled via environment kill-switch, reports can be exported locally for inspection.

---

## 4. Global Kill-Switches (`DO_NOT_TRACK=1` & `EASYSRT_TELEMETRY_URL=off`)

You can block **all** telemetry traffic at the process/OS level using either standard environment variable:

- **`DO_NOT_TRACK=1`**
- **`EASYSRT_TELEMETRY_URL=off`** (also accepts `""`, `none`, or `disabled`)

When either kill-switch is active:
- All telemetry uploads (crash reports, Level 1, Level 2A/2B, Level 3) are immediately disabled in `diagnostics.rs`.
- Automatic background 24-hour update checks are suppressed.
- If you manually click **Check for Updates**, EasySRT Studio bypasses `easysrt-telemetry.ig-systems.rs` and queries GitHub Releases directly.

---

## 5. Retention Summary & Your Rights

| Data Category | Consent Required | Max Retention | Storage |
| :--- | :--- | :--- | :--- |
| Audio, video, raw transcripts, API keys | Never uploaded to IG-Systems | Local only | Local disk / OS Keyring |
| Level 0 (`daily_update_checks`) | Toggleable (`autoCheckUpdates`, default on) | Aggregated daily counters (zero IP/device ID) | Cloudflare D1 |
| Level 1 & 2A (`job_and_edit_metrics`) | Explicit opt-in (`shareAnonymousUsageMetrics`) | Aggregated performance & counter metrics | Cloudflare D1 |
| Level 2B unpromoted `ngram_candidates` | Explicit opt-in (`shareAnonymousUsageMetrics`) | **30 days** (cron purge if `cohort_count < 5` or `subnet_count < 3`) | Cloudflare D1 |
| Level 3 (`training_edit_pairs`) | Explicit opt-in (`shareAnonymizedTranslationEdits`) | **90 days** (daily cron purge) | Cloudflare D1 |

Under **GDPR** and **Serbian ZZPL**, you have the right to withdraw consent at any time, request information about processing, or report a privacy concern via [GitHub Issues](https://github.com/Gruya13/EasySRT-Studio/issues). Because Level 0–2B telemetry contains no personal identifiers or device IDs, data cannot be tied back to an individual user after submission.

---

## 6. Related Documentation

- [English User Guide](docs/user/en/guide.md) · [Srpski korisnički vodič](docs/user/sr/guide.md)
- [Troubleshooting & FAQ (EN)](docs/user/en/troubleshooting.md) · [Rešavanje problema i FAQ (SR)](docs/user/sr/troubleshooting.md)
- [System Architecture](docs/dev/ARCHITECTURE.md)
