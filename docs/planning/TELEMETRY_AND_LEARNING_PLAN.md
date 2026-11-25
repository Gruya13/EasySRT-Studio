# EasySRT Studio — Privacy-First Telemetry & Automated AI-vs-Human Learning Plan

**Status:** Planned (post-`v0.4.0` roadmap)  
**Target Releases:** `v0.4.x` / `v0.5.0`  
**Origin:** Product, legal-compliance (GDPR / Serbian ZZPL), and NLP feedback-loop design session.

This document specifies a **100% legal, transparent, Zero-PII telemetry and automated subtitle-learning architecture** for **EasySRT Studio**. Its core goal is to capture hardware performance, segmentation/timing friction, and recurring AI translation mistakes **automatically at Export time — with zero extra clicks during editing** — while upholding strict offline-first user trust, **GDPR** (EU), **ZZPL** (*Zakon o zaštiti podataka o ličnosti*, Serbia), and **Apache 2.0** open-source standards.

---

## Core Design Principles

1. **Explicit Opt-In Consent (Never Silent Default-On)**: No analytics or crash payloads leave the user's machine unless explicitly enabled in `SetupWizard` or `SettingsPanel`, with a live **"Inspect Exact JSON Payload"** preview right beside the toggles and 1-click revocation at any time.
2. **Zero Workflow Interruption (Export-Triggered Diffing)**: Users never click "report bad subtitle" buttons while editing. Instead, the app snapshots initial AI cues when the pipeline finishes and compares `AI Baseline ↔ Final Exported Cues` locally in `< 5 ms` when the user clicks **Export**.
3. **Zero-PII Edge & 1st-Party Infrastructure (`easysrt-telemetry.ig-systems.rs`)**: All telemetry goes exclusively to our 1st-party Cloudflare Worker + **Cloudflare D1** SQLite database (no Google Analytics, Mixpanel, Sentry SaaS, or third-party trackers). Client IP addresses (`CF-Connecting-IP`) are never stored, and no persistent `device_id`, username, or file path is ever transmitted.
4. **K-Anonymity (`k >= 5`) for Automated Lexical Learning**: Short lowercase word/phrase corrections (`1–3` words, stripped of all uppercase proper nouns, digits, URLs, and glossary terms) only become visible to maintainers once at least **5 independent user cohorts** perform the exact same fix, mathematically preventing individual content leakage while automatically surfacing systematic AI translation errors.

---

## Summary of Planned Workstreams

| # | Workstream | Target Module(s) |
| :-: | :--- | :--- |
| **1** | **GDPR / ZZPL Explicit Opt-In Consent UI, Payload Inspector & `PRIVACY.md`** | `SetupWizard.tsx`, `SettingsPanel.tsx`, `models.rs`, `models.ts`, `appStore.ts`, `PRIVACY.md` *(new)* |
| **2** | **Level 1 — Hardware, Runtime & Pipeline RTF Performance Telemetry** | `diagnostics.rs`, `pipeline/mod.rs`, `worker/src/index.ts` |
| **3** | **AI Baseline Snapshot & Local Export-Triggered Diff Engine** | `models.rs`, `models.ts`, `pipeline/telemetry_diff.rs` *(new)*, `pipeline/exporter.rs`, `commands.rs` |
| **4** | **Level 2A — Automated Structural, Grammatical & Timing Edit Classifiers (Zero Text)** | `pipeline/telemetry_diff.rs`, `diagnostics.rs` |
| **5** | **Level 2B — K-Anonymous (`k >= 5`) Lowercase Lexical & Phrasing Diff Extraction** | `pipeline/telemetry_diff.rs`, `worker/src/index.ts`, `worker/schema.sql` *(new)* |
| **6** | **Level 3 — Optional Anonymized Edit-Pair Corpus Donation on Export (*AI Training Opt-In*)** | `pipeline/telemetry_diff.rs`, `diagnostics.rs`, `ExportModal.tsx`, `worker/src/index.ts` |
| **7** | **Cloudflare D1 Telemetry Backend, TTL Cron Cleanup & Maintainer Digest CLI** | `worker/src/index.ts`, `worker/cloudflare.config.ts`, `scripts/telemetry-digest.mjs` *(new)* |

---

## Workstream 1 — GDPR / ZZPL Explicit Opt-In Consent UI, Payload Inspector & `PRIVACY.md`

### Current Behavior & Gap
- In [`src-tauri/src/models.rs`](../../src-tauri/src/models.rs) and [`src/stores/appStore.ts`](../../src/stores/appStore.ts), `auto_send_crash_reports` currently defaults to `true` without asking during `SetupWizard`.
- Under **GDPR Art. 6(1)(a) / Art. 7**, **ePrivacy Art. 5(3)**, and **Serbian ZZPL Art. 12 / Art. 15**, pre-ticked or silent default-on telemetry is not valid consent — and in an open-source app marketed around *Total Offline Privacy*, silent outbound requests risk community distrust.

### Planned Changes
1. **Three Clear, Granular Consent Flags on `AppConfig` (`models.rs` & `models.ts`)**:
   - `auto_send_crash_reports: bool` — **default `false`** on fresh installs until explicitly chosen in `SetupWizard` (sends sanitized panic/crash diagnostics only on crashes).
   - `share_anonymous_usage_metrics: bool` — **default `false`** until opted in (enables Level 1 performance metrics + Level 2A structural edit classifiers + Level 2B `k >= 5` K-anonymous lowercase n-gram diffs on Export).
   - `share_anonymized_translation_edits: bool` — **default `false`** until explicitly opted in (enables Level 3 PII-scrubbed `(source, initial_ai, final_human)` cue pairs on Export for improving Serbian AI translation models).
   - Also honor environment overrides `DO_NOT_TRACK=1` and `EASYSRT_TELEMETRY_URL=off` as hard global kill-switches.
2. **Transparent Consent Step in `SetupWizard.tsx` & `SettingsPanel.tsx` (`General / Opšte`)**:
   - Add a clean, non-nagging privacy card on the final step of `SetupWizard.tsx` explaining in plain Serbian/English that audio, video, and API keys never leave the device, with independent toggles and a **"View Exact JSON Sample" (*Pogledaj tačan primer JSON-a*)** expandable preview.
   - Mirror all three toggles + the live JSON preview in `SettingsPanel.tsx` (`Opšte` / `General` tab) for instant 1-click opt-in or revocation.
3. **Rotating Monthly Cohort Salt (No Persistent `device_id`)**:
   - Store only a local random `telemetry_cohort_salt` + `telemetry_cohort_month` (`"YYYY-MM"`) in `config.json` (regenerated automatically every calendar month, never linked to hardware/OS identifiers). When computing a K-anonymity cohort token for a specific word pair, send `SHA-256(monthly_salt + ":" + pair_key)[..16]` so the server can count distinct monthly cohorts without even being able to link two different word-pair events to the same user!
4. **Repository `PRIVACY.md`**:
   - Document data controller (*IG-Systems*), exact JSON schemas per tier, zero-IP policy, Cloudflare D1 retention/TTL rules, and how to disable via UI or `EASYSRT_TELEMETRY_URL=off`.

---

## Workstream 2 — Level 1: Hardware, Runtime & Pipeline RTF Performance Telemetry

### Payload & Purpose (`~400 bytes`, Zero User Text)
When `share_anonymous_usage_metrics == true`, emit a compact summary event upon pipeline completion (`POST /v1/telemetry/job-metrics`):
- **Hardware & Stack**: `appVersion`, `os` (`"windows"` | `"linux"`), `gpuTier` (`"cpuOnly"` | `"vram4Gb"` | `"vram8Gb"` | `"vram12GbPlus"`), `whisperBackend` (`"cuda"` | `"vulkan"` | `"cpu"`), `whisperModelId` (catalog ID or `"custom"`), `llmProvider` (`"localLlama"` | `"deepSeek"` | `"customOpenAi"`), `llmModelId` (catalog ID, `"deepseek-chat"`, or `"custom"`).
- **Real-World Speed & Reliability**:
  - `audioDurationMs`, `whisperElapsedMs`, `diarizerElapsedMs`, `llmElapsedMs`
  - `asrRtf` (`whisperElapsedMs / audioDurationMs`), `llmRtf`
  - `oomCpuFallback: bool`, `batchSubdivisionRetries: u32`
  - `cueCount: u32`, `sourceLang`, `targetLang`, `translationStyle`, `addressForm`, `preset` (`"standard"` | `"reels"` | `"custom"`).

### Engineering Value
Immediately highlights real-world bottlenecks across consumer GPUs (e.g., if a specific local GGUF model triggers frequent `batchSubdivisionRetries` or `oomCpuFallback` on 4 GB cards).

---

## Workstream 3 — AI Baseline Snapshot & Local Export-Triggered Diff Engine

### Architecture (`src-tauri/src/pipeline/telemetry_diff.rs`)
1. **Capturing the AI Baseline (`SubtitleProject.ai_baseline_cues`)**:
   - Define a compact struct in `models.rs` / `models.ts`:
     ```rust
     #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
     #[serde(rename_all = "camelCase")]
     pub struct AiBaselineCue {
         pub id: u32,
         pub start_ms: u64,
         pub end_ms: u64,
         pub source_text: String,
         pub translated_text: String,
     }
     ```
   - Populate `project.ai_baseline_cues` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`) automatically when `transcribe_audio` (if no LLM stage runs) or `run_batch_operation` (`Translate` / `PolishSerbian`) finishes.
2. **Zero-Click Trigger at Export (`pipeline::exporter` / `commands::export_subtitles`)**:
   - When the user exports subtitles (`.srt`, `.vtt`, or `.txt`), if either `share_anonymous_usage_metrics` or `share_anonymized_translation_edits` is enabled and `project.ai_baseline_cues` is non-empty:
     - Run `telemetry_diff::analyze_export_diff(&project.ai_baseline_cues, &project.cues, &project.options)` synchronously in `< 5 ms`.
     - Dispatch the resulting sanitized payload asynchronously on a background Tokio task (`tokio::spawn` with a 5-second timeout, silently ignoring network errors when offline so Export never blocks or fails!).
     - Deduplicate per `(job_id, exported_hash)` so exporting `.srt` and `.vtt` back-to-back does not double-count the same edits.
3. **Interval-Aware Alignment (`AI Baseline ↔ Final Cues`)**:
   - Match baseline cues `B[i]` to final cues `F[j]` via temporal overlap:
     - **1-to-1 match**: single baseline cue overlaps single final cue (`IoU >= 0.5`) → inspect text & timing delta.
     - **1-to-Many (`Split` detected)**: one baseline cue `B[i]` overlaps `F[j..=j+k]` (`k >= 1`) → classify exact split point (punctuation, conjunction, pause).
     - **Many-to-1 (`Merge` detected)**: baseline cues `B[i..=i+m]` (`m >= 1`) map into one final cue `F[j]` → record pre-merge durations and inter-cue gap.
     - **Adjacent Boundary Shift**: `B[i]` + `B[i+1]` vs `F[j]` + `F[j+1]` where the last token of `B[i]` moved to the start of `F[j+1]` (or first token of `B[i+1]` moved to the end of `F[j]`).

---

## Workstream 4 — Level 2A: Automated Structural, Grammatical & Timing Edit Classifiers (Zero Text)

Computed 100% locally inside `telemetry_diff.rs` for `share_anonymous_usage_metrics == true` users — **transmitting only integer counters and millisecond averages, never subtitle words**:

1. **Overall Subtitle Quality KPIs**:
   - `totalBaselineCues`, `totalFinalCues`, `unchangedCuesCount`, `textEditedCuesCount`, `editRatePct`
   - `initialHighCpsCount` → `finalHighCpsCount`, `initialLineTooLongCount` → `finalLineTooLongCount`
   - `avgCpsBefore` → `avgCpsAfter`, `condenseCpsUsed: bool`.
2. **Linguistic Error Classifiers (Serbian & Cross-Lingual)**:
   - `encliticReorderedCount`: cue words match as a normalized lowercase set except one or more Serbian enclitics (`je, sam, si, smo, ste, su, bih, bi, bismo, biste, se, mi, ti, mu, joj, ga, ih`) changed position.
   - `redundantPossessiveRemovedCount`: possessive (`moj, tvoj, njegov, njen, naš, vaš, njihov` + inflections) deleted or replaced with reflexive `svoj`.
   - `passiveCalqueRemovedCount`: literal `"od strane"` or passive participle calque rewritten.
   - `conjunctionCommaFixedCount`: spurious English-calque comma removed after sentence-initial connector (`"I, "` → `"I "`, `"Tako da, "` → `"Tako da "`, `"Ali, "` → `"Ali "`).
   - `missedFillerDeletedCount`: single word deleted from AI cue (tracks residual filler leakage).
   - `inflectionSuffixFixCount`: single-word substitution where both words are lowercase and share a common prefix of `>= 4` chars (captures gender/case/aspect endings like `najstranije → najstrašnijih`).
   - `boundaryOrphanShiftedCount`: trailing 1-word starter moved across consecutive cues.
3. **Segmentation & Timeline Ergonomics**:
   - **Splits**: `splitCount`, `splitAfterCommaCount`, `splitAfterConjunctionCount`, `avgPreSplitChars`, `avgPreSplitDurationMs`.
   - **Merges**: `mergeCount`, `avgMergedShortCueDurationMs`, `avgMergedShortCueChars`, `avgMergedGapMs`.
   - **Timeline Trims** (on 1:1 matched cues where `|delta| >= 40 ms`): `startTrimCount`, `avgStartDeltaMs` (signed), `endTrimCount`, `avgEndDeltaMs` (signed — e.g., `+145 ms` directly tells us whether `POST_SPEECH_HOLD_MS` needs tuning).

---

## Workstream 5 — Level 2B: K-Anonymous (`k >= 5`) Lowercase Lexical & Phrasing Diff Extraction

### Problem Solved
Numeric classifiers tell us *what category* of mistake happened, while **K-anonymous n-gram diffing** automatically reveals **the exact missed words and idioms** (e.g., `"tijekom" → "tokom"`, `"napraviti odluku" → "doneti odluku"`, `"u pogledu" → "što se tiče"`, or deleted filler `"zapravo"`) across the user base — without ever exposing private names, numbers, or unique sentences.

### Two-Stage Privacy Barrier
1. **Stage A — Strict Client-Side Extractor (`telemetry_diff::extract_safe_ngram_diffs`)**:
   - Operates only on 1-to-1 matched cues where at least 50% of the cue's tokens remained unchanged (localized lexical edit, not a full rewrite).
   - Extracts minimal token-level diff spans `(old_span, new_span)` where `0 <= new_words <= 3` and `1 <= old_words <= 3` (covering 1–3 word substitutions and 1–2 word filler deletions, normalized to Latin for Serbian uniformity).
   - **Hard Local Privacy Filter — immediately discards the diff if `old_span` or `new_span`**:
     - Contains **any uppercase character** (`c.is_uppercase()` — eliminates all personal names, surnames, cities, streets, companies, organizations, and acronyms).
     - Contains **any digit** (`c.is_numeric()`), `@`, `/`, `\`, `.com`, or non-alphabetic symbols (other than spaces/hyphens/apostrophes).
     - Contains any token shorter than 2 chars (unless paired with another word) or longer than 24 chars.
     - Matches any key or value in `project.options.glossary` (case-insensitive).
   - Computes an unlinkable per-pair cohort proof `cohortProof = HMAC/SHA256(monthly_cohort_salt, lang_pair + "|" + old_span + "->" + new_span)[..24]` so the server cannot even correlate two n-gram diffs sent from the same export!
2. **Stage B — Server-Side K-Anonymity Vault (`worker/` + Cloudflare D1)**:
   - Stores incoming tuples in D1 table `ngram_candidates`:
     - Increments `cohort_count` **only** when `(pair_hash, cohort_proof)` has not been seen before.
   - **Hard Visibility Gate (`K_ANONYMITY_MIN_COHORTS = 5`)**:
     - Maintainer digest / API queries execute `SELECT lang_pair, old_span, new_span, cohort_count FROM ngram_candidates WHERE cohort_count >= 5 ORDER BY cohort_count DESC`.
     - Any rare or user-specific phrase (`cohort_count < 5`) is **never returned by any API** and is **permanently deleted after 30 days** by a daily Cloudflare Worker Cron Trigger.

---

## Workstream 6 — Level 3: Optional Anonymized Edit-Pair Donation on Export (*AI Training Opt-In*)

For power users, beta testers, and maintainers who explicitly enable `share_anonymized_translation_edits == true` (default `false`):
1. **Automatic Collection at Export**:
   - Collects up to `25` modified cues per exported project where `initial_ai_text != final_human_text`:
     ```json
     {
       "sourceLang": "en",
       "targetLang": "sr",
       "style": "natural",
       "addressForm": "auto",
       "sourceText": "We spoke with [NAME] yesterday, what do you think?",
       "initialAiText": "Razgovarali smo sa [NAME] juče. Šta",
       "finalHumanText": "Juče smo razgovarali sa [NAME]."
     }
     ```
2. **Local PII & Proper-Noun Sanitizer (`diagnostics::sanitize_training_cue_pair`)**:
   - Strips emails, phone numbers (`+381...`, grouped digits), URLs, API tokens, file paths, and replaces multi-digit numbers with `[NUM]`.
   - Replaces glossary entries and mid-sentence capitalized proper nouns (matchingacross `sourceText` and `translatedText`) with `[NAME]` so personal identities are masked before transmission.
3. **Transparent Zero-Click Notice in `ExportModal.tsx`**:
   - When `share_anonymized_translation_edits == true` and modified cues exist (`N > 0`), `ExportModal.tsx` shows a subtle footer note:
     > *„N izmenjenih titlova biće anonimno poslato radi unapređenja AI prevoda • [Preskoči za ovaj izvoz]“*
   - Normal export requires **zero extra clicks**, while confidential projects can bypass sharing for that export with one click.

---

## Workstream 7 — Cloudflare D1 Telemetry Backend, TTL Cron & Maintainer Digest CLI

### Extending `worker/` (`easysrt-telemetry.ig-systems.rs`)
1. **Keep Existing Bug & Crash Endpoints Untouched**:
   - `/v1/report/bug` and `/v1/report/crash` continue deduplicating into GitHub Issues as today.
2. **Add D1 Database (`easysrt-telemetry-db`) & New Routes**:
   - `POST /v1/telemetry/export` — validates schema + size (`<= 32 KiB`), enforces zero-IP rate limiting (ephemeral in-memory `SHA-256(ip + daily_salt)`), inserts aggregated row into `job_and_edit_metrics`, and upserts K-anonymous `ngram_candidates` + `ngram_cohorts`.
   - `POST /v1/telemetry/training-pairs` — validates and stores sanitized Level 3 `(source, initialAi, finalHuman)` pairs in `training_edit_pairs` (90-day retention or exportable JSONL).
   - `GET /v1/admin/digest` (protected by `TELEMETRY_ADMIN_TOKEN` secret in Cloudflare Worker): returns aggregated RTF percentiles by GPU/model, edit-rate trends by `app_version`, structural error rankings, promoted K-anonymous lexical diffs (`cohort_count >= 5`), and downloadable JSONL training pairs.
   - `scheduled()` Cron Trigger (`0 3 * * *`): purges `ngram_candidates` where `cohort_count < 5` and `updated_at < now - 30 days`.
3. **Maintainer One-Command CLI (`scripts/telemetry-digest.mjs`)**:
   - `npm run telemetry:digest` fetches `/v1/admin/digest` and prints a clean markdown terminal report so we can directly feed real-world findings into `sr_script.rs`, `segmenter.rs`, and `translator.rs` before each release.

---

## Suggested Implementation Order (`staging`)

1. **Phase 1 — Consent, `PRIVACY.md` & Baseline Snapshot (`Workstreams 1 & 3`)**:
   - Add `shareAnonymousUsageMetrics`, `shareAnonymizedTranslationEdits`, rotating `telemetryCohortSalt`, and `SetupWizard` / `SettingsPanel` consent UI + live JSON payload preview + `PRIVACY.md`.
   - Add `SubtitleProject.ai_baseline_cues` snapshot population after ASR/LLM stages.
2. **Phase 2 — Local Diff Engine & Classifiers (`Workstreams 2, 4, 5-Client & 6-Client`)**:
   - Implement `src-tauri/src/pipeline/telemetry_diff.rs` (interval alignment, enclitic/possessive/passive/orphan/split/merge/trim classifiers, uppercase-free `1–3` word n-gram extractor, and Level 3 `[NAME]`/`[NUM]` sanitizer) + comprehensive Rust unit tests + `ExportModal.tsx` skip pill.
3. **Phase 3 — Cloudflare Worker + D1 Schema & Digest CLI (`Workstreams 5-Server & 7`)**:
   - Add `worker/schema.sql`, D1 bindings in `worker/cloudflare.config.ts`, `/v1/telemetry/export`, `/v1/telemetry/training-pairs`, `/v1/admin/digest`, daily K-anonymity TTL Cron cleanup, Worker unit tests (`worker/src/__tests__/`), and `scripts/telemetry-digest.mjs`.
4. **Phase 4 — Verification**:
   ```bash
   npm run version:check
   npm run typecheck
   npm test
   npm run build
   cd worker && npm test
   cd src-tauri && cargo check --all-targets && cargo test && cargo clippy --all-targets -- -D warnings
   ```
