export interface Env {
  GITHUB_REPO: string;
  GITHUB_ISSUES_TOKEN: string;
}

export interface BugReportPayload {
  appVersion: string;
  title: string;
  description: string;
  steps?: string | null;
  contact?: string | null;
  platform?: string | null;
  gpu?: string | null;
  diagnostics?: string | null;
}

export interface CrashReportPayload {
  appVersion: string;
  crashKind: string;
  fingerprint: string;
  message: string;
  stackTrace?: string | null;
  source?: string | null;
  platform?: string | null;
  gpu?: string | null;
  diagnostics?: string | null;
}

export interface ReportSuccessResponse {
  ok: true;
  deduplicated: boolean;
  issueNumber: number;
  issueUrl: string;
}

export interface RateLimitResult {
  allowed: boolean;
  remaining: number;
  retryAfterSeconds: number;
}

export type ValidationResult<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };

export const SERVICE_NAME = "easysrt-telemetry";
export const MAX_BODY_BYTES = 131_072; // 128 KiB hard cap
const MAX_UPSTREAM_BODY_BYTES = 524_288; // 512 KiB safety cap for GitHub API responses
const MAX_GITHUB_BODY_CHARS = 60_000; // Safe margin below GitHub's 65,536-char issue body limit

export const RATE_LIMIT_WINDOW_SECONDS = 600; // 10 minutes
export const RATE_LIMITS: Readonly<Record<"bug" | "crash", number>> = {
  bug: 5,
  crash: 15,
};

export const GITHUB_USER_AGENT = "EasySRT-Telemetry-Worker";
const GITHUB_API_VERSION = "2022-11-28";

export const CORS_HEADERS: Readonly<Record<string, string>> = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

const FINGERPRINT_RE = /^[A-Za-z0-9]{8,64}$/;
const GITHUB_REPO_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

export class PayloadTooLargeError extends Error {
  readonly maxBytes: number;

  constructor(maxBytes: number) {
    super(`Request body exceeds maximum allowed size of ${maxBytes} bytes`);
    this.name = "PayloadTooLargeError";
    this.maxBytes = maxBytes;
  }
}

export function jsonResponse(
  payload: unknown,
  status = 200,
  extraHeaders?: Record<string, string>,
): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ...CORS_HEADERS,
      ...extraHeaders,
    },
  });
}

/**
 * Extra server-side privacy sanitization: redacts API keys, GitHub tokens,
 * Bearer tokens, and Windows/Linux home directory usernames.
 */
export function sanitizeText(raw: string): string {
  let out = raw;

  // Strip ASCII control chars except tab, LF, CR
  out = out.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, "");

  // Redact OpenAI / DeepSeek / Stripe-style sk- keys
  out = out.replace(/\bsk-[A-Za-z0-9_-]{8,}/g, "[REDACTED_API_KEY]");

  // Redact GitHub personal access tokens (classic & fine-grained) and OAuth/app tokens
  out = out.replace(/\bgithub_pat_[A-Za-z0-9_]+/g, "[REDACTED_GH_TOKEN]");
  out = out.replace(/\bgh[pousr]_[A-Za-z0-9]+/g, "[REDACTED_GH_TOKEN]");

  // Redact HTTP Bearer tokens
  out = out.replace(/\bBearer\s+[A-Za-z0-9._-]+/gi, "Bearer [REDACTED_TOKEN]");

  // Redact generic api_key / api-key / apikey assignments
  out = out.replace(
    /(\bapi[_-]?key\s*[:=]\s*["']?)([^\s"',}]+)/gi,
    "$1[REDACTED]",
  );

  // Redact Windows user paths (both backslash and forward-slash forms)
  out = out.replace(/([A-Za-z]:\\Users\\)([^\\/\s"'<>|]+)/gi, "$1<USER>");
  out = out.replace(/([A-Za-z]:\/Users\/)([^\\/\s"'<>|]+)/gi, "$1<USER>");

  // Redact Linux /home/<name>/ paths
  out = out.replace(/(^|[^A-Za-z0-9_:])(\/home\/)([^\\/\s"'<>|]+)/g, "$1$2<USER>");

  return out;
}

export function truncateText(
  text: string,
  maxLen: number,
  suffix = "\n... [truncated]",
): string {
  if (text.length <= maxLen) {
    return text;
  }
  const sliceLen = Math.max(0, maxLen - suffix.length);
  return `${text.slice(0, sliceLen)}${suffix}`;
}

export function sanitizeSingleLine(raw: string, maxLen: number): string {
  const cleaned = sanitizeText(raw)
    .replace(/[\r\n\t]+/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
  return truncateText(cleaned, maxLen, "...");
}

export function escapeCodeFence(raw: string): string {
  return raw
    .replace(/```/g, "` ` `")
    .replace(/<\/details>/gi, "<\\/details>");
}

/**
 * Reads a Web ReadableStream with a strict byte cap so large/unbounded bodies
 * can never exhaust Worker memory. Aborts and throws PayloadTooLargeError when
 * the Content-Length header or cumulative streamed bytes exceed maxBytes.
 */
export async function readBoundedText(
  body: ReadableStream<Uint8Array> | null,
  maxBytes = MAX_BODY_BYTES,
  contentLengthHeader?: string | null,
): Promise<string> {
  if (contentLengthHeader) {
    const trimmed = contentLengthHeader.trim();
    if (/^\d+$/.test(trimmed)) {
      const declared = Number(trimmed);
      if (Number.isFinite(declared) && declared > maxBytes) {
        if (body) {
          try {
            await body.cancel();
          } catch {
            // Ignore stream cancellation error on early reject
          }
        }
        throw new PayloadTooLargeError(maxBytes);
      }
    }
  }

  if (!body) {
    return "";
  }

  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      if (!value || value.byteLength === 0) {
        continue;
      }
      totalBytes += value.byteLength;
      if (totalBytes > maxBytes) {
        await reader.cancel();
        throw new PayloadTooLargeError(maxBytes);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  if (totalBytes === 0) {
    return "";
  }

  const combined = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.byteLength;
  }

  return new TextDecoder("utf-8").decode(combined);
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validateOptionalString(
  obj: Record<string, unknown>,
  field: string,
): { ok: true; value: string | null } | { ok: false; error: string } {
  const val = obj[field];
  if (val === undefined || val === null) {
    return { ok: true, value: null };
  }
  if (typeof val !== "string") {
    return { ok: false, error: `Field '${field}' must be a string or null` };
  }
  return { ok: true, value: val };
}

export function validateBugReport(raw: unknown): ValidationResult<BugReportPayload> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Request body must be a JSON object" };
  }

  if (typeof raw.appVersion !== "string") {
    return { ok: false, error: "Field 'appVersion' is required and must be a string" };
  }
  const appVersion = raw.appVersion.trim();
  if (appVersion.length < 1 || appVersion.length > 40) {
    return { ok: false, error: "Field 'appVersion' must be between 1 and 40 characters" };
  }

  if (typeof raw.title !== "string") {
    return { ok: false, error: "Field 'title' is required and must be a string" };
  }
  const title = raw.title.replace(/[\r\n\t]+/g, " ").trim();
  if (title.length < 3 || title.length > 180) {
    return { ok: false, error: "Field 'title' must be between 3 and 180 characters" };
  }

  if (typeof raw.description !== "string") {
    return { ok: false, error: "Field 'description' is required and must be a string" };
  }
  const description = raw.description.trim();
  if (description.length < 5 || description.length > 8000) {
    return {
      ok: false,
      error: "Field 'description' must be between 5 and 8000 characters",
    };
  }

  const steps = validateOptionalString(raw, "steps");
  if (!steps.ok) return steps;
  const contact = validateOptionalString(raw, "contact");
  if (!contact.ok) return contact;
  const platform = validateOptionalString(raw, "platform");
  if (!platform.ok) return platform;
  const gpu = validateOptionalString(raw, "gpu");
  if (!gpu.ok) return gpu;
  const diagnostics = validateOptionalString(raw, "diagnostics");
  if (!diagnostics.ok) return diagnostics;

  return {
    ok: true,
    data: {
      appVersion,
      title,
      description,
      steps: steps.value,
      contact: contact.value,
      platform: platform.value,
      gpu: gpu.value,
      diagnostics: diagnostics.value,
    },
  };
}

export function validateCrashReport(raw: unknown): ValidationResult<CrashReportPayload> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Request body must be a JSON object" };
  }

  if (typeof raw.appVersion !== "string") {
    return { ok: false, error: "Field 'appVersion' is required and must be a string" };
  }
  const appVersion = raw.appVersion.trim();
  if (appVersion.length < 1 || appVersion.length > 40) {
    return { ok: false, error: "Field 'appVersion' must be between 1 and 40 characters" };
  }

  if (typeof raw.crashKind !== "string") {
    return { ok: false, error: "Field 'crashKind' is required and must be a string" };
  }
  const crashKind = raw.crashKind.replace(/[\r\n\t]+/g, " ").trim();
  if (crashKind.length < 1 || crashKind.length > 64) {
    return { ok: false, error: "Field 'crashKind' must be between 1 and 64 characters" };
  }

  if (typeof raw.fingerprint !== "string") {
    return { ok: false, error: "Field 'fingerprint' is required and must be a string" };
  }
  const fingerprint = raw.fingerprint.trim();
  if (!FINGERPRINT_RE.test(fingerprint)) {
    return {
      ok: false,
      error: "Field 'fingerprint' must be 8 to 64 alphanumeric characters",
    };
  }

  if (typeof raw.message !== "string") {
    return { ok: false, error: "Field 'message' is required and must be a string" };
  }
  const message = raw.message.trim();
  if (message.length < 1 || message.length > 4000) {
    return { ok: false, error: "Field 'message' must be between 1 and 4000 characters" };
  }

  const stackTrace = validateOptionalString(raw, "stackTrace");
  if (!stackTrace.ok) return stackTrace;
  const source = validateOptionalString(raw, "source");
  if (!source.ok) return source;
  const platform = validateOptionalString(raw, "platform");
  if (!platform.ok) return platform;
  const gpu = validateOptionalString(raw, "gpu");
  if (!gpu.ok) return gpu;
  const diagnostics = validateOptionalString(raw, "diagnostics");
  if (!diagnostics.ok) return diagnostics;

  return {
    ok: true,
    data: {
      appVersion,
      crashKind,
      fingerprint,
      message,
      stackTrace: stackTrace.value,
      source: source.value,
      platform: platform.value,
      gpu: gpu.value,
      diagnostics: diagnostics.value,
    },
  };
}

function buildDiagnosticsDetailsBlock(diagnostics: string | null | undefined, maxChars: number): string {
  if (!diagnostics || diagnostics.trim().length === 0) {
    return "";
  }
  const sanitized = truncateText(sanitizeText(diagnostics.trim()), maxChars);
  const safeContent = escapeCodeFence(sanitized);
  return [
    "<details>",
    "<summary>System Diagnostics & Logs</summary>",
    "",
    "```text",
    safeContent,
    "```",
    "",
    "</details>",
  ].join("\n");
}

export function formatBugIssue(
  payload: BugReportPayload,
  timestampIso = new Date().toISOString(),
): { title: string; body: string; labels: string[] } {
  const cleanVersion = sanitizeSingleLine(payload.appVersion, 40);
  const cleanTitle = sanitizeSingleLine(payload.title, 180);
  const cleanDescription = truncateText(sanitizeText(payload.description.trim()), 8000);
  const cleanSteps = payload.steps?.trim()
    ? truncateText(sanitizeText(payload.steps.trim()), 4000)
    : null;
  const cleanContact = payload.contact?.trim()
    ? sanitizeSingleLine(payload.contact, 200)
    : null;
  const cleanPlatform = payload.platform?.trim()
    ? sanitizeSingleLine(payload.platform, 200)
    : "unknown";
  const cleanGpu = payload.gpu?.trim()
    ? sanitizeSingleLine(payload.gpu, 250)
    : "unknown";

  const sections: string[] = [
    "### Bug Report Metadata",
    "",
    `- **App Version:** \`${cleanVersion}\``,
    `- **Platform:** ${cleanPlatform}`,
    `- **GPU:** ${cleanGpu}`,
    `- **Reported At (UTC):** ${timestampIso}`,
  ];

  if (cleanContact) {
    sections.push(`- **Contact:** ${cleanContact}`);
  }

  sections.push("", "## Description", "", cleanDescription);

  if (cleanSteps) {
    sections.push("", "## Steps to Reproduce", "", cleanSteps);
  }

  const baseBody = sections.join("\n");
  const remainingBudget = Math.max(1000, MAX_GITHUB_BODY_CHARS - baseBody.length - 120);
  const detailsBlock = buildDiagnosticsDetailsBlock(payload.diagnostics, remainingBudget);

  const body = detailsBlock ? `${baseBody}\n\n${detailsBlock}\n` : `${baseBody}\n`;

  return {
    title: `[Bug v${cleanVersion}] ${cleanTitle}`,
    body,
    labels: ["bug"],
  };
}

export function formatCrashIssue(
  payload: CrashReportPayload,
  timestampIso = new Date().toISOString(),
): { title: string; body: string; labels: string[] } {
  const cleanVersion = sanitizeSingleLine(payload.appVersion, 40);
  const cleanKind = sanitizeSingleLine(payload.crashKind, 64);
  const cleanFingerprint = payload.fingerprint.trim();
  const shortMessage = sanitizeSingleLine(payload.message, 120);
  const cleanMessage = truncateText(sanitizeText(payload.message.trim()), 4000);
  const cleanSource = payload.source?.trim()
    ? sanitizeSingleLine(payload.source, 200)
    : null;
  const cleanPlatform = payload.platform?.trim()
    ? sanitizeSingleLine(payload.platform, 200)
    : "unknown";
  const cleanGpu = payload.gpu?.trim()
    ? sanitizeSingleLine(payload.gpu, 250)
    : "unknown";
  const cleanStack = payload.stackTrace?.trim()
    ? escapeCodeFence(truncateText(sanitizeText(payload.stackTrace.trim()), 16_000))
    : null;

  const sections: string[] = [
    `<!-- crash-fp:${cleanFingerprint} -->`,
    "",
    "### Crash Report Metadata",
    "",
    `- **App Version:** \`${cleanVersion}\``,
    `- **Crash Kind:** \`${cleanKind}\``,
    `- **Fingerprint:** \`${cleanFingerprint}\``,
  ];

  if (cleanSource) {
    sections.push(`- **Source:** \`${cleanSource}\``);
  }

  sections.push(
    `- **Platform:** ${cleanPlatform}`,
    `- **GPU:** ${cleanGpu}`,
    `- **Reported At (UTC):** ${timestampIso}`,
    "",
    "## Error Message",
    "",
    "```text",
    escapeCodeFence(cleanMessage),
    "```",
  );

  if (cleanStack) {
    sections.push("", "## Stack Trace", "", "```text", cleanStack, "```");
  }

  const baseBody = sections.join("\n");
  const remainingBudget = Math.max(1000, MAX_GITHUB_BODY_CHARS - baseBody.length - 120);
  const detailsBlock = buildDiagnosticsDetailsBlock(payload.diagnostics, remainingBudget);

  const body = detailsBlock ? `${baseBody}\n\n${detailsBlock}\n` : `${baseBody}\n`;

  return {
    title: `[Crash v${cleanVersion}] ${cleanKind}: ${shortMessage}`,
    body,
    labels: ["crash-report"],
  };
}

export function formatCrashComment(
  payload: CrashReportPayload,
  timestampIso = new Date().toISOString(),
): string {
  const cleanVersion = sanitizeSingleLine(payload.appVersion, 40);
  const cleanKind = sanitizeSingleLine(payload.crashKind, 64);
  const cleanMessage = truncateText(sanitizeText(payload.message.trim()), 4000);
  const cleanSource = payload.source?.trim()
    ? sanitizeSingleLine(payload.source, 200)
    : null;
  const cleanPlatform = payload.platform?.trim()
    ? sanitizeSingleLine(payload.platform, 200)
    : "unknown";
  const cleanGpu = payload.gpu?.trim()
    ? sanitizeSingleLine(payload.gpu, 250)
    : "unknown";
  const cleanStack = payload.stackTrace?.trim()
    ? escapeCodeFence(truncateText(sanitizeText(payload.stackTrace.trim()), 12_000))
    : null;

  const sections: string[] = [
    "### Additional Crash Occurrence",
    "",
    `- **App Version:** \`${cleanVersion}\``,
    `- **Crash Kind:** \`${cleanKind}\``,
  ];

  if (cleanSource) {
    sections.push(`- **Source:** \`${cleanSource}\``);
  }

  sections.push(
    `- **Platform:** ${cleanPlatform}`,
    `- **GPU:** ${cleanGpu}`,
    `- **Timestamp (UTC):** ${timestampIso}`,
    "",
    "#### Message",
    "",
    "```text",
    escapeCodeFence(cleanMessage),
    "```",
  );

  if (cleanStack) {
    sections.push("", "#### Stack Trace", "", "```text", cleanStack, "```");
  }

  const baseBody = sections.join("\n");
  const remainingBudget = Math.max(1000, MAX_GITHUB_BODY_CHARS - baseBody.length - 120);
  const detailsBlock = buildDiagnosticsDetailsBlock(payload.diagnostics, remainingBudget);

  return detailsBlock ? `${baseBody}\n\n${detailsBlock}\n` : `${baseBody}\n`;
}

function getDefaultCache(): Cache | null {
  const maybeGlobal = globalThis as { caches?: { default?: Cache } };
  return maybeGlobal.caches?.default ?? null;
}

export async function checkRateLimit(
  request: Request,
  route: "bug" | "crash",
  nowMs = Date.now(),
): Promise<RateLimitResult> {
  const limit = RATE_LIMITS[route];
  const cache = getDefaultCache();
  if (!cache) {
    return { allowed: true, remaining: limit, retryAfterSeconds: 0 };
  }

  const ip =
    request.headers.get("CF-Connecting-IP")?.trim() ||
    request.headers.get("X-Forwarded-For")?.split(",")[0]?.trim() ||
    "unknown";

  const windowMs = RATE_LIMIT_WINDOW_SECONDS * 1000;
  const bucket = Math.floor(nowMs / windowMs);
  const defaultResetAtMs = (bucket + 1) * windowMs;
  const cacheUrl = `https://rate-limit.easysrt.internal/${route}/${encodeURIComponent(ip)}/${bucket}`;
  const cacheKey = new Request(cacheUrl, { method: "GET" });

  try {
    const existing = await cache.match(cacheKey);
    let count = 0;
    let resetAtMs = defaultResetAtMs;

    if (existing) {
      const raw = await readBoundedText(existing.body, 4096, existing.headers.get("content-length"));
      const parsed = JSON.parse(raw) as { count?: unknown; resetAtMs?: unknown };
      if (typeof parsed.count === "number" && Number.isFinite(parsed.count) && parsed.count >= 0) {
        count = parsed.count;
      }
      if (
        typeof parsed.resetAtMs === "number" &&
        Number.isFinite(parsed.resetAtMs) &&
        parsed.resetAtMs > nowMs
      ) {
        resetAtMs = parsed.resetAtMs;
      }
    }

    const retryAfterSeconds = Math.max(1, Math.ceil((resetAtMs - nowMs) / 1000));
    if (count >= limit) {
      return { allowed: false, remaining: 0, retryAfterSeconds };
    }

    const nextCount = count + 1;
    const updated = new Response(JSON.stringify({ count: nextCount, resetAtMs }), {
      headers: {
        "Content-Type": "application/json",
        "Cache-Control": `public, max-age=${retryAfterSeconds}`,
      },
    });
    await cache.put(cacheKey, updated);

    return {
      allowed: true,
      remaining: Math.max(0, limit - nextCount),
      retryAfterSeconds: 0,
    };
  } catch (err) {
    console.error(
      JSON.stringify({
        event: "rate_limit_cache_error",
        route,
        error: err instanceof Error ? err.message : String(err),
      }),
    );
    return { allowed: true, remaining: limit, retryAfterSeconds: 0 };
  }
}

function githubHeaders(token: string, withJsonBody = false): Record<string, string> {
  const headers: Record<string, string> = {
    "User-Agent": GITHUB_USER_AGENT,
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": GITHUB_API_VERSION,
  };
  if (withJsonBody) {
    headers["Content-Type"] = "application/json";
  }
  return headers;
}

interface GitHubIssueRef {
  number: number;
  html_url: string;
}

async function readUpstreamJson(response: Response): Promise<unknown> {
  const text = await readBoundedText(
    response.body,
    MAX_UPSTREAM_BODY_BYTES,
    response.headers.get("content-length"),
  );
  if (!text) {
    return null;
  }
  return JSON.parse(text) as unknown;
}

async function createGitHubIssue(
  repo: string,
  token: string,
  issue: { title: string; body: string; labels: string[] },
): Promise<{ ok: true; issue: GitHubIssueRef } | { ok: false; status: number }> {
  const resp = await fetch(`https://api.github.com/repos/${repo}/issues`, {
    method: "POST",
    headers: githubHeaders(token, true),
    body: JSON.stringify(issue),
  });

  if (!resp.ok) {
    console.error(
      JSON.stringify({
        event: "github_issue_create_failed",
        status: resp.status,
      }),
    );
    return { ok: false, status: resp.status };
  }

  const parsed = await readUpstreamJson(resp);
  if (
    !isPlainRecord(parsed) ||
    typeof parsed.number !== "number" ||
    typeof parsed.html_url !== "string"
  ) {
    console.error(
      JSON.stringify({
        event: "github_issue_create_invalid_response",
      }),
    );
    return { ok: false, status: 502 };
  }

  return {
    ok: true,
    issue: { number: parsed.number, html_url: parsed.html_url },
  };
}

async function findOpenCrashIssue(
  repo: string,
  token: string,
  fingerprint: string,
): Promise<GitHubIssueRef | null> {
  const searchQuery = `repo:${repo} is:issue is:open in:body "crash-fp:${fingerprint}"`;
  const params = new URLSearchParams({ q: searchQuery });
  const resp = await fetch(`https://api.github.com/search/issues?${params.toString()}`, {
    method: "GET",
    headers: githubHeaders(token, false),
  });

  if (!resp.ok) {
    console.error(
      JSON.stringify({
        event: "github_issue_search_failed",
        status: resp.status,
      }),
    );
    return null;
  }

  const parsed = await readUpstreamJson(resp);
  if (!isPlainRecord(parsed) || !Array.isArray(parsed.items)) {
    return null;
  }

  for (const item of parsed.items) {
    if (
      isPlainRecord(item) &&
      typeof item.number === "number" &&
      typeof item.html_url === "string" &&
      item.pull_request === undefined &&
      (item.state === undefined || item.state === "open")
    ) {
      return { number: item.number, html_url: item.html_url };
    }
  }

  return null;
}

async function addGitHubIssueComment(
  repo: string,
  token: string,
  issueNumber: number,
  body: string,
): Promise<boolean> {
  const resp = await fetch(
    `https://api.github.com/repos/${repo}/issues/${issueNumber}/comments`,
    {
      method: "POST",
      headers: githubHeaders(token, true),
      body: JSON.stringify({ body }),
    },
  );

  if (!resp.ok) {
    console.error(
      JSON.stringify({
        event: "github_comment_create_failed",
        issueNumber,
        status: resp.status,
      }),
    );
    return false;
  }

  return true;
}

function resolveGitHubConfig(
  env: Env,
): { ok: true; repo: string; token: string } | { ok: false } {
  const repo = typeof env.GITHUB_REPO === "string" ? env.GITHUB_REPO.trim() : "";
  const token =
    typeof env.GITHUB_ISSUES_TOKEN === "string" ? env.GITHUB_ISSUES_TOKEN.trim() : "";

  if (!GITHUB_REPO_RE.test(repo) || token.length === 0) {
    return { ok: false };
  }

  return { ok: true, repo, token };
}

async function parseJsonRequestBody(
  request: Request,
): Promise<{ ok: true; data: unknown } | { ok: false; response: Response }> {
  let rawText: string;
  try {
    rawText = await readBoundedText(
      request.body,
      MAX_BODY_BYTES,
      request.headers.get("content-length"),
    );
  } catch (err) {
    if (err instanceof PayloadTooLargeError) {
      return {
        ok: false,
        response: jsonResponse({ ok: false, error: err.message }, 413),
      };
    }
    throw err;
  }

  if (rawText.trim().length === 0) {
    return {
      ok: false,
      response: jsonResponse({ ok: false, error: "Request body is empty" }, 400),
    };
  }

  try {
    return { ok: true, data: JSON.parse(rawText) as unknown };
  } catch {
    return {
      ok: false,
      response: jsonResponse({ ok: false, error: "Invalid JSON payload" }, 400),
    };
  }
}

async function handleBugReport(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "bug");
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request);
  if (!parsedBody.ok) {
    return parsedBody.response;
  }

  const validated = validateBugReport(parsedBody.data);
  if (!validated.ok) {
    return jsonResponse({ ok: false, error: validated.error }, 400);
  }

  const config = resolveGitHubConfig(env);
  if (!config.ok) {
    console.error(JSON.stringify({ event: "misconfigured_env", route: "bug" }));
    return jsonResponse(
      { ok: false, error: "Telemetry service is not configured" },
      500,
    );
  }

  const issue = formatBugIssue(validated.data);
  const created = await createGitHubIssue(config.repo, config.token, issue);
  if (!created.ok) {
    return jsonResponse(
      { ok: false, error: "Failed to record bug report in issue tracker" },
      502,
    );
  }

  console.log(
    JSON.stringify({
      event: "bug_report_created",
      issueNumber: created.issue.number,
    }),
  );

  const responsePayload: ReportSuccessResponse = {
    ok: true,
    deduplicated: false,
    issueNumber: created.issue.number,
    issueUrl: created.issue.html_url,
  };
  return jsonResponse(responsePayload, 201);
}

async function handleCrashReport(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "crash");
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request);
  if (!parsedBody.ok) {
    return parsedBody.response;
  }

  const validated = validateCrashReport(parsedBody.data);
  if (!validated.ok) {
    return jsonResponse({ ok: false, error: validated.error }, 400);
  }

  const config = resolveGitHubConfig(env);
  if (!config.ok) {
    console.error(JSON.stringify({ event: "misconfigured_env", route: "crash" }));
    return jsonResponse(
      { ok: false, error: "Telemetry service is not configured" },
      500,
    );
  }

  const existing = await findOpenCrashIssue(
    config.repo,
    config.token,
    validated.data.fingerprint,
  );

  if (existing) {
    const commentBody = formatCrashComment(validated.data);
    const commented = await addGitHubIssueComment(
      config.repo,
      config.token,
      existing.number,
      commentBody,
    );
    if (!commented) {
      return jsonResponse(
        { ok: false, error: "Failed to append crash occurrence to existing issue" },
        502,
      );
    }

    console.log(
      JSON.stringify({
        event: "crash_report_deduplicated",
        issueNumber: existing.number,
      }),
    );

    const responsePayload: ReportSuccessResponse = {
      ok: true,
      deduplicated: true,
      issueNumber: existing.number,
      issueUrl: existing.html_url,
    };
    return jsonResponse(responsePayload, 200);
  }

  const issue = formatCrashIssue(validated.data);
  const created = await createGitHubIssue(config.repo, config.token, issue);
  if (!created.ok) {
    return jsonResponse(
      { ok: false, error: "Failed to record crash report in issue tracker" },
      502,
    );
  }

  console.log(
    JSON.stringify({
      event: "crash_report_created",
      issueNumber: created.issue.number,
    }),
  );

  const responsePayload: ReportSuccessResponse = {
    ok: true,
    deduplicated: false,
    issueNumber: created.issue.number,
    issueUrl: created.issue.html_url,
  };
  return jsonResponse(responsePayload, 201);
}

export default {
  async fetch(
    request: Request,
    env: Env,
    _ctx?: ExecutionContext,
  ): Promise<Response> {
    try {
      if (request.method === "OPTIONS") {
        return new Response(null, { status: 204, headers: CORS_HEADERS });
      }

      const url = new URL(request.url);
      const pathname =
        url.pathname.length > 1 && url.pathname.endsWith("/")
          ? url.pathname.slice(0, -1)
          : url.pathname;

      if (pathname === "/health") {
        if (request.method !== "GET") {
          return jsonResponse({ ok: false, error: "Method not allowed" }, 405, {
            Allow: "GET, OPTIONS",
          });
        }
        return jsonResponse({ ok: true, service: SERVICE_NAME }, 200);
      }

      if (pathname === "/v1/report/bug") {
        if (request.method !== "POST") {
          return jsonResponse({ ok: false, error: "Method not allowed" }, 405, {
            Allow: "POST, OPTIONS",
          });
        }
        return await handleBugReport(request, env);
      }

      if (pathname === "/v1/report/crash") {
        if (request.method !== "POST") {
          return jsonResponse({ ok: false, error: "Method not allowed" }, 405, {
            Allow: "POST, OPTIONS",
          });
        }
        return await handleCrashReport(request, env);
      }

      return jsonResponse({ ok: false, error: "Not found" }, 404);
    } catch (err) {
      console.error(
        JSON.stringify({
          event: "unhandled_worker_error",
          error: err instanceof Error ? err.message : String(err),
        }),
      );
      return jsonResponse({ ok: false, error: "Internal server error" }, 500);
    }
  },
} satisfies ExportedHandler<Env>;
