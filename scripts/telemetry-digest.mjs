#!/usr/bin/env node

import { pathToFileURL } from "node:url";

export const DEFAULT_TELEMETRY_URL = "https://easysrt-telemetry.ig-systems.rs";
export const DEFAULT_GITHUB_REPO = "Gruya13/EasySRT-Studio";
const GITHUB_REPO_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const FORBIDDEN_TOKEN_FLAGS = new Set([
  "--token",
  "--admin-token",
  "-t",
  "--bearer",
  "--auth",
  "--secret",
]);

/**
 * Strips ANSI CSI, OSC, DCS/PM/APC, 2-byte ESC sequences, and C0/C1 control characters
 * from untrusted remote strings before printing to a terminal (`V05-M4`).
 */
export function stripTerminalControls(input) {
  const str = input === null || input === undefined ? "" : String(input);
  return str
    // OSC sequences: ESC ] ... (BEL or ST)
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
    // DCS / SOS / PM / APC sequences: ESC P/X/^/_ ... ST
    .replace(/\x1b[PX^_][\s\S]*?\x1b\\/g, "")
    // CSI sequences: ESC [ ... final byte
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "")
    // Any remaining 2-byte ESC sequence
    .replace(/\x1b[@-Z\\-_]/g, "")
    // C0 controls (except \t, \n, \r) and C1 controls (0x7F, 0x80..0x9F)
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F\x80-\x9F]/g, "");
}

export function sanitizeMarkdownCell(input) {
  return stripTerminalControls(input)
    .replace(/[\r\n\t]+/g, " ")
    .replace(/\|/g, "\\|")
    .replace(/\s{2,}/g, " ")
    .trim();
}

export function resolveAdminToken(env = process.env) {
  const raw = env?.TELEMETRY_ADMIN_TOKEN;
  if (typeof raw !== "string" || raw.trim().length === 0) {
    throw new Error(
      "Missing TELEMETRY_ADMIN_TOKEN environment variable. Set TELEMETRY_ADMIN_TOKEN in your shell or untracked .env file (never pass tokens as CLI arguments).",
    );
  }
  if (/[\x00-\x20\x7f]/.test(raw)) {
    throw new Error(
      "TELEMETRY_ADMIN_TOKEN contains invalid whitespace or control characters.",
    );
  }
  return raw;
}

export function validateTelemetryBaseUrl(rawUrl) {
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    throw new Error(`Invalid telemetry base URL: ${rawUrl}`);
  }
  const isLoopbackHttp =
    parsed.protocol === "http:" &&
    (parsed.hostname === "localhost" ||
      parsed.hostname === "127.0.0.1" ||
      parsed.hostname === "[::1]");
  if (parsed.protocol !== "https:" && !isLoopbackHttp) {
    throw new Error(
      `Refusing insecure telemetry URL protocol '${parsed.protocol}'. Use https:// (or http://localhost for local testing).`,
    );
  }
  if (parsed.username || parsed.password) {
    throw new Error("Telemetry URL must not contain embedded credentials.");
  }
  return parsed.toString().replace(/\/+$/, "");
}

export function parseDigestArgs(argv, env = process.env) {
  let days = 30;
  let baseUrl = env.EASYSRT_TELEMETRY_URL || DEFAULT_TELEMETRY_URL;
  let repo = env.GITHUB_REPO || DEFAULT_GITHUB_REPO;
  let format = "markdown";
  let help = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const flagName = arg.includes("=") ? arg.slice(0, arg.indexOf("=")) : arg;
    if (FORBIDDEN_TOKEN_FLAGS.has(flagName)) {
      throw new Error(
        `Refusing CLI flag '${flagName}': secrets must be passed via TELEMETRY_ADMIN_TOKEN env var, never via CLI flags.`,
      );
    }

    if (arg === "--help" || arg === "-h") {
      help = true;
    } else if (arg === "--days" || arg.startsWith("--days=")) {
      const val = arg.includes("=") ? arg.slice("--days=".length) : argv[++i];
      if (!val || !/^-?\d+$/.test(val.trim())) {
        throw new Error(`Invalid --days value: ${val ?? ""}`);
      }
      days = Math.min(365, Math.max(1, Number.parseInt(val.trim(), 10)));
    } else if (arg === "--url" || arg.startsWith("--url=")) {
      const val = arg.includes("=") ? arg.slice("--url=".length) : argv[++i];
      if (!val) {
        throw new Error("Missing value for --url");
      }
      baseUrl = val;
    } else if (arg === "--repo" || arg.startsWith("--repo=")) {
      const val = arg.includes("=") ? arg.slice("--repo=".length) : argv[++i];
      if (!val || !GITHUB_REPO_RE.test(val.trim())) {
        throw new Error(`Invalid --repo value: ${val ?? ""}`);
      }
      repo = val.trim();
    } else if (arg === "--format" || arg.startsWith("--format=")) {
      const val = (arg.includes("=") ? arg.slice("--format=".length) : argv[++i] ?? "").trim().toLowerCase();
      if (val !== "markdown" && val !== "json" && val !== "jsonl") {
        throw new Error(`Unsupported --format '${val}'. Expected markdown, json, or jsonl.`);
      }
      format = val;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  return {
    days,
    baseUrl: validateTelemetryBaseUrl(baseUrl),
    repo,
    format,
    help,
  };
}

function formatNum(value, digits = 2) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return "0";
  }
  return Number.isInteger(value) ? String(value) : value.toFixed(digits);
}

function renderMarkdownTable(headers, rows) {
  if (!rows || rows.length === 0) {
    return "_No records in window._\n";
  }
  const head = `| ${headers.map(sanitizeMarkdownCell).join(" | ")} |`;
  const sep = `| ${headers.map(() => "---").join(" | ")} |`;
  const body = rows.map(
    (row) => `| ${row.map((cell) => sanitizeMarkdownCell(cell)).join(" | ")} |`,
  );
  return [head, sep, ...body].join("\n") + "\n";
}

export function formatDigestMarkdown({ digest, releases, releasesError, repo, days }) {
  const generatedAt = sanitizeMarkdownCell(digest?.generatedAt || new Date().toISOString());
  const windowDays = Number(digest?.windowDays ?? days);
  const lines = [
    `# EasySRT Studio Telemetry Digest (${windowDays}d)`,
    "",
    `- **Generated At (UTC):** ${generatedAt}`,
    `- **Repository:** ${sanitizeMarkdownCell(repo)}`,
    "",
    "## GitHub Release Downloads",
    "",
  ];

  if (releasesError) {
    lines.push(`> GitHub release stats unavailable: ${sanitizeMarkdownCell(releasesError)}`, "");
  } else if (!Array.isArray(releases) || releases.length === 0) {
    lines.push("_No GitHub releases found._", "");
  } else {
    let grandTotal = 0;
    const releaseRows = releases.map((rel) => {
      const assets = Array.isArray(rel?.assets) ? rel.assets : [];
      const total = assets.reduce(
        (sum, asset) => sum + (Number.isFinite(Number(asset?.download_count)) ? Number(asset.download_count) : 0),
        0,
      );
      grandTotal += total;
      const published = typeof rel?.published_at === "string" ? rel.published_at.slice(0, 10) : "-";
      return [
        rel?.tag_name || rel?.name || "unknown",
        published,
        String(assets.length),
        String(total),
      ];
    });
    lines.push(
      `Total downloads across recent ${releases.length} release(s): **${grandTotal}**`,
      "",
      renderMarkdownTable(["Release", "Published", "Assets", "Downloads"], releaseRows),
    );
  }

  const dailyTotals = Array.isArray(digest?.dau?.dailyTotals) ? digest.dau.dailyTotals : [];
  const byOs = Array.isArray(digest?.dau?.byOsAndVersion) ? digest.dau.byOsAndVersion : [];
  lines.push("## Daily Active Installations (DAU via 24h Update Checks)", "");
  lines.push(
    renderMarkdownTable(
      ["UTC Date", "Total Checks"],
      dailyTotals.map((r) => [r.utc_date, String(r.total_checks ?? 0)]),
    ),
  );
  if (byOs.length > 0) {
    lines.push(
      "### DAU Breakdown by OS / Arch / Version",
      "",
      renderMarkdownTable(
        ["UTC Date", "Target", "Arch", "Version", "Checks"],
        byOs.map((r) => [
          r.utc_date,
          r.target,
          r.arch,
          r.current_version,
          String(r.check_count ?? 0),
        ]),
      ),
    );
  }

  const perf = Array.isArray(digest?.performance) ? digest.performance : [];
  lines.push("## Pipeline Performance (Level 1)", "");
  lines.push(
    renderMarkdownTable(
      ["GPU Tier", "Whisper", "LLM Provider", "LLM Model", "Jobs", "Avg ASR RTF", "Avg LLM RTF", "OOM Fallbacks", "Batch Retries"],
      perf.map((r) => [
        r.gpu_tier,
        r.whisper_backend,
        r.llm_provider,
        r.llm_model_id,
        String(r.job_count ?? 0),
        formatNum(r.avg_asr_rtf, 3),
        formatNum(r.avg_llm_rtf, 3),
        String(r.oom_fallbacks ?? 0),
        String(r.batch_retries ?? 0),
      ]),
    ),
  );

  const trends = Array.isArray(digest?.editTrends) ? digest.editTrends : [];
  lines.push("## Subtitle Edit Trends & Linguistic Classifiers (Level 2A)", "");
  lines.push(
    renderMarkdownTable(
      ["Version", "Lang", "Style", "Exports", "Edit %", "CPS Before->After", "Enclitics", "Possessives", "Passives", "Conj Comma", "Fillers", "Inflection", "Orphans", "Splits/Merges", "Avg Trim (ms)"],
      trends.map((r) => [
        r.app_version,
        r.target_lang,
        r.translation_style,
        String(r.export_count ?? 0),
        formatNum(r.avg_edit_rate_pct, 1),
        `${formatNum(r.avg_cps_before, 1)} -> ${formatNum(r.avg_cps_after, 1)}`,
        String(r.enclitic_reordered ?? 0),
        String(r.redundant_possessive_removed ?? 0),
        String(r.passive_calque_removed ?? 0),
        String(r.conjunction_comma_fixed ?? 0),
        String(r.missed_filler_deleted ?? 0),
        String(r.inflection_suffix_fix ?? 0),
        String(r.boundary_orphan_shifted ?? 0),
        `${r.split_count ?? 0}/${r.merge_count ?? 0}`,
        `${formatNum(r.avg_start_delta_ms, 0)} / ${formatNum(r.avg_end_delta_ms, 0)}`,
      ]),
    ),
  );

  const ngrams = Array.isArray(digest?.promotedNgrams) ? digest.promotedNgrams : [];
  const minCohorts = Number(digest?.kAnonymityMinCohorts ?? 5);
  const minSubnets = Number(digest?.kAnonymityMinSubnets ?? 3);
  lines.push(
    `## Promoted K-Anonymous Lexical Fixes (k >= ${minCohorts}, subnets >= ${minSubnets})`,
    "",
  );
  lines.push(
    renderMarkdownTable(
      ["Lang Pair", "AI Output", "Human Fix", "Cohorts", "Subnets", "Updated"],
      ngrams.map((r) => [
        r.lang_pair,
        r.old_span,
        r.new_span === "" ? "(deleted)" : r.new_span,
        String(r.cohort_count ?? 0),
        String(r.subnet_count ?? 0),
        typeof r.updated_at === "string" ? r.updated_at.slice(0, 10) : "-",
      ]),
    ),
  );

  const trainingTotal = Number(digest?.trainingPairsSummary?.totalCount ?? 0);
  const trainingRows = Array.isArray(digest?.trainingPairsSummary?.byLanguageAndStyle)
    ? digest.trainingPairsSummary.byLanguageAndStyle
    : [];
  lines.push("## Donated Training Pairs (Level 3)", "");
  lines.push(`Total sanitized cue pairs in window: **${trainingTotal}**`, "");
  if (trainingRows.length > 0) {
    lines.push(
      renderMarkdownTable(
        ["Source", "Target", "Style", "Address Form", "Pairs"],
        trainingRows.map((r) => [
          r.source_lang,
          r.target_lang,
          r.style,
          r.address_form,
          String(r.pair_count ?? 0),
        ]),
      ),
    );
  }

  return lines.join("\n").trimEnd() + "\n";
}

export async function runTelemetryDigest({
  argv = process.argv.slice(2),
  env = process.env,
  fetchImpl = globalThis.fetch,
} = {}) {
  const opts = parseDigestArgs(argv, env);
  if (opts.help) {
    return [
      "Usage: TELEMETRY_ADMIN_TOKEN=... node scripts/telemetry-digest.mjs [options]",
      "",
      "Options:",
      "  --days <1..365>           Lookback window in days (default: 30)",
      `  --url <https://...>       Worker base URL (default: ${DEFAULT_TELEMETRY_URL})`,
      `  --repo <owner/name>       GitHub repository (default: ${DEFAULT_GITHUB_REPO})`,
      "  --format <markdown|json|jsonl>  Output format (default: markdown)",
      "  --help, -h                Show this help message",
      "",
      "Security: TELEMETRY_ADMIN_TOKEN must be passed strictly via environment variable.",
    ].join("\n") + "\n";
  }

  const token = resolveAdminToken(env);
  const workerFormat = opts.format === "jsonl" ? "jsonl" : "json";
  const digestUrl = `${opts.baseUrl}/v1/admin/digest?days=${opts.days}&format=${workerFormat}`;

  const digestRes = await fetchImpl(digestUrl, {
    method: "GET",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: workerFormat === "jsonl" ? "application/x-ndjson" : "application/json",
    },
  });

  if (!digestRes.ok) {
    const errText = stripTerminalControls(await digestRes.text()).slice(0, 300);
    throw new Error(`Telemetry digest request failed (${digestRes.status}): ${errText}`);
  }

  if (opts.format === "jsonl") {
    const rawJsonl = await digestRes.text();
    return stripTerminalControls(rawJsonl);
  }

  const digest = await digestRes.json();
  if (opts.format === "json") {
    return `${stripTerminalControls(JSON.stringify(digest, null, 2))}\n`;
  }

  let releases = [];
  let releasesError = null;
  try {
    const ghHeaders = {
      "User-Agent": "EasySRT-Telemetry-Digest",
      Accept: "application/vnd.github+json",
    };
    if (typeof env.GITHUB_TOKEN === "string" && env.GITHUB_TOKEN.trim().length > 0) {
      ghHeaders.Authorization = `Bearer ${env.GITHUB_TOKEN.trim()}`;
    }
    const ghRes = await fetchImpl(
      `https://api.github.com/repos/${opts.repo}/releases?per_page=10`,
      { method: "GET", headers: ghHeaders },
    );
    if (!ghRes.ok) {
      releasesError = `HTTP ${ghRes.status}`;
    } else {
      const parsed = await ghRes.json();
      releases = Array.isArray(parsed) ? parsed : [];
    }
  } catch (err) {
    releasesError = err instanceof Error ? err.message : String(err);
  }

  return formatDigestMarkdown({
    digest,
    releases,
    releasesError,
    repo: opts.repo,
    days: opts.days,
  });
}

async function main() {
  try {
    const output = await runTelemetryDigest();
    process.stdout.write(output);
  } catch (err) {
    const msg = stripTerminalControls(err instanceof Error ? err.message : String(err));
    process.stderr.write(`Error: ${msg}\n`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
