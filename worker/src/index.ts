export interface Env {
  GITHUB_REPO: string;
  GITHUB_ISSUES_TOKEN: string;
  TELEMETRY_ADMIN_TOKEN?: string;
  SUBNET_HMAC_SECRET?: string;
  DB?: D1Database;
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

export interface JobMetricsPayload {
  appVersion: string;
  os: string;
  gpuTier: string;
  whisperBackend: string;
  whisperModelId: string;
  llmProvider: string;
  llmModelId: string;
  audioDurationMs: number;
  whisperElapsedMs: number;
  diarizerElapsedMs: number;
  llmElapsedMs: number;
  asrRtf: number;
  llmRtf: number;
  oomCpuFallback: boolean;
  batchSubdivisionRetries: number;
  cueCount: number;
  sourceLang: string;
  targetLang: string;
  translationStyle: string;
  addressForm: string;
  preset: string;
}

export interface ExportDiffMetrics {
  totalBaselineCues: number;
  totalFinalCues: number;
  unchangedCuesCount: number;
  textEditedCuesCount: number;
  editRatePct: number;
  initialHighCpsCount: number;
  finalHighCpsCount: number;
  initialLineTooLongCount: number;
  finalLineTooLongCount: number;
  avgCpsBefore: number;
  avgCpsAfter: number;
  condenseCpsUsed: boolean;
  encliticReorderedCount: number;
  redundantPossessiveRemovedCount: number;
  passiveCalqueRemovedCount: number;
  conjunctionCommaFixedCount: number;
  missedFillerDeletedCount: number;
  inflectionSuffixFixCount: number;
  boundaryOrphanShiftedCount: number;
  splitCount: number;
  splitAfterCommaCount: number;
  splitAfterConjunctionCount: number;
  avgPreSplitChars: number;
  avgPreSplitDurationMs: number;
  mergeCount: number;
  avgMergedShortCueDurationMs: number;
  avgMergedShortCueChars: number;
  avgMergedGapMs: number;
  startTrimCount: number;
  avgStartDeltaMs: number;
  endTrimCount: number;
  avgEndDeltaMs: number;
}

export interface SafeNgramDiff {
  langPair: string;
  oldSpan: string;
  newSpan: string;
  cohortProof: string;
}

export interface ExportTelemetryPayload {
  appVersion: string;
  os: string;
  sourceLang: string;
  targetLang: string;
  translationStyle: string;
  addressForm: string;
  metrics: ExportDiffMetrics;
  ngramDiffs: SafeNgramDiff[];
}

export interface TrainingCuePair {
  sourceLang: string;
  targetLang: string;
  style: string;
  addressForm: string;
  sourceText: string;
  initialAiText: string;
  finalHumanText: string;
}

export interface TrainingPairsPayload {
  appVersion: string;
  pairs: TrainingCuePair[];
}

export type RateLimitRoute =
  | "bug"
  | "crash"
  | "update"
  | "jobMetrics"
  | "export"
  | "trainingPairs";

export const SERVICE_NAME = "easysrt-telemetry";
export const MAX_BODY_BYTES = 131_072; // 128 KiB hard cap for bug/crash reports
export const MAX_TELEMETRY_BODY_BYTES = 32_768; // 32 KiB cap for job-metrics & export telemetry
export const MAX_TRAINING_PAIRS_BODY_BYTES = 65_536; // 64 KiB cap for training-pairs
export const MAX_UPDATE_MANIFEST_BYTES = 65_536; // 64 KiB cap for GitHub latest.json
export const UPDATE_CACHE_TTL_SECONDS = 300;
export const K_ANONYMITY_MIN_COHORTS = 5;
export const K_ANONYMITY_MIN_SUBNETS = 3;
export const NGRAM_PURGE_DAYS = 30;
export const TRAINING_PAIRS_PURGE_DAYS = 90;

const MAX_UPSTREAM_BODY_BYTES = 524_288; // 512 KiB safety cap for GitHub API responses
const MAX_GITHUB_BODY_CHARS = 60_000; // Safe margin below GitHub's 65,536-char issue body limit
const MAX_NGRAM_DIFFS_PER_EXPORT = 100;
const MAX_TRAINING_PAIRS_PER_REQUEST = 25;

const FIXED_UPDATE_CACHE_URL = "https://update-cache.easysrt.internal/v1/update/latest.json";

export const RATE_LIMIT_WINDOW_SECONDS = 600; // 10 minutes
export const RATE_LIMITS: Readonly<Record<RateLimitRoute, number>> = {
  bug: 5,
  crash: 15,
  update: 60,
  jobMetrics: 30,
  export: 30,
  trainingPairs: 20,
};

export const GITHUB_USER_AGENT = "EasySRT-Telemetry-Worker";
const GITHUB_API_VERSION = "2022-11-28";

const FINGERPRINT_RE = /^[A-Za-z0-9]{8,64}$/;
const GITHUB_REPO_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const SEMVER_RE = /^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const LANG_CODE_RE = /^[A-Za-z0-9_-]{1,24}$/;
const MODEL_ID_RE = /^[A-Za-z0-9_.:/ -]{1,80}$/;
const LANG_PAIR_RE = /^[a-z]{2,8}(?:->|-)[a-z]{2,8}$/;
const COHORT_PROOF_RE = /^[a-f0-9]{16,64}$/i;
const NGRAM_ALLOWED_CHARS_RE = /^[\p{Ll}\s'-]+$/u;
const UPPERCASE_CHAR_RE = /\p{Lu}/u;
const DIGIT_CHAR_RE = /\p{Nd}/u;
const LOWERCASE_LETTER_RE = /\p{Ll}/u;

const ALLOWED_UPDATE_TARGETS = new Set(["windows", "linux", "darwin"]);
const ALLOWED_UPDATE_ARCHES = new Set(["x86_64", "aarch64", "i686", "armv7"]);
const ALLOWED_OS_VALUES = new Set(["windows", "linux", "darwin", "macos"]);
const ALLOWED_GPU_TIERS = new Set(["cpuOnly", "vram4Gb", "vram8Gb", "vram12GbPlus"]);
const ALLOWED_WHISPER_BACKENDS = new Set(["cuda", "vulkan", "cpu"]);
const ALLOWED_LLM_PROVIDERS = new Set(["localLlama", "deepSeek", "customOpenAi"]);
const ALLOWED_TRANSLATION_STYLES = new Set(["natural", "formal"]);
const ALLOWED_ADDRESS_FORMS = new Set(["auto", "politeVi", "informalTi"]);
const ALLOWED_PRESETS = new Set(["standard", "reels", "custom"]);

// Ephemeral per-isolate daily salt rotated by UTC date for Zero-IP rate-limit hashing.
let cachedDailySaltDate = "";
let cachedDailySaltValue = "";

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
      ...extraHeaders,
    },
  });
}

function bytesToHex(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i++) {
    out += bytes[i].toString(16).padStart(2, "0");
  }
  return out;
}

export async function sha256Bytes(input: string): Promise<Uint8Array> {
  const encoded = new TextEncoder().encode(input);
  const digest = await crypto.subtle.digest("SHA-256", encoded);
  return new Uint8Array(digest);
}

export async function sha256Hex(input: string): Promise<string> {
  return bytesToHex(await sha256Bytes(input));
}

export async function hmacSha256Hex(secret: string, data: string): Promise<string> {
  const enc = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    enc.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, enc.encode(data));
  return bytesToHex(new Uint8Array(sig));
}

export async function timingSafeEqualStrings(a: string, b: string): Promise<boolean> {
  if (!a || !b) {
    return false;
  }
  const [hashA, hashB] = await Promise.all([sha256Bytes(a), sha256Bytes(b)]);
  const subtle = crypto.subtle as SubtleCrypto & {
    timingSafeEqual?: (x: ArrayBufferView, y: ArrayBufferView) => boolean;
  };
  if (typeof subtle.timingSafeEqual === "function") {
    return subtle.timingSafeEqual(hashA, hashB);
  }
  let diff = 0;
  for (let i = 0; i < hashA.length; i += 1) {
    diff |= hashA[i] ^ hashB[i];
  }
  return diff === 0;
}

function getDailyRateLimitSalt(utcDate: string): string {
  if (cachedDailySaltDate === utcDate && cachedDailySaltValue.length > 0) {
    return cachedDailySaltValue;
  }
  const randomBytes = new Uint8Array(32);
  crypto.getRandomValues(randomBytes);
  cachedDailySaltValue = bytesToHex(randomBytes);
  cachedDailySaltDate = utcDate;
  return cachedDailySaltValue;
}

function extractClientIp(request: Request): string {
  return (
    request.headers.get("CF-Connecting-IP")?.trim() ||
    request.headers.get("X-Forwarded-For")?.split(",")[0]?.trim() ||
    "unknown"
  );
}

function parseIpv4Octets(ip: string): number[] | null {
  const parts = ip.split(".");
  if (parts.length !== 4) {
    return null;
  }
  const octets: number[] = [];
  for (const part of parts) {
    if (!/^\d{1,3}$/.test(part)) {
      return null;
    }
    const n = Number(part);
    if (!Number.isInteger(n) || n < 0 || n > 255) {
      return null;
    }
    octets.push(n);
  }
  return octets;
}

function expandIpv6Groups(ip: string): string[] | null {
  let normalized = ip.toLowerCase();
  const zoneIdx = normalized.indexOf("%");
  if (zoneIdx !== -1) {
    normalized = normalized.slice(0, zoneIdx);
  }
  if (!normalized.includes(":")) {
    return null;
  }

  // Handle IPv4-mapped suffix like ::ffff:192.0.2.128
  const lastColon = normalized.lastIndexOf(":");
  const tail = normalized.slice(lastColon + 1);
  if (tail.includes(".")) {
    const v4 = parseIpv4Octets(tail);
    if (!v4) {
      return null;
    }
    const hi = ((v4[0] << 8) | v4[1]).toString(16).padStart(4, "0");
    const lo = ((v4[2] << 8) | v4[3]).toString(16).padStart(4, "0");
    normalized = `${normalized.slice(0, lastColon + 1)}${hi}:${lo}`;
  }

  const doubleColonParts = normalized.split("::");
  if (doubleColonParts.length > 2) {
    return null;
  }

  const parseSegment = (segment: string): string[] | null => {
    if (segment === "") {
      return [];
    }
    const groups = segment.split(":");
    for (const g of groups) {
      if (!/^[0-9a-f]{1,4}$/.test(g)) {
        return null;
      }
    }
    return groups.map((g) => g.padStart(4, "0"));
  };

  if (doubleColonParts.length === 1) {
    const groups = parseSegment(doubleColonParts[0]);
    return groups && groups.length === 8 ? groups : null;
  }

  const left = parseSegment(doubleColonParts[0]);
  const right = parseSegment(doubleColonParts[1]);
  if (!left || !right) {
    return null;
  }
  const missing = 8 - (left.length + right.length);
  if (missing < 1) {
    return null;
  }
  return [...left, ...Array<string>(missing).fill("0000"), ...right];
}

/**
 * Extracts a coarse network prefix (/24 for IPv4, /48 for IPv6) for Sybil resistance
 * without ever persisting raw IP addresses.
 */
export function extractSubnetPrefix(rawIp: string): string {
  let ip = rawIp.trim();
  if (!ip || ip === "unknown") {
    return "unknown";
  }
  if (ip.startsWith("[") && ip.includes("]")) {
    ip = ip.slice(1, ip.indexOf("]"));
  }

  // Handle IPv4 with optional port (e.g. 203.0.113.5:12345)
  if (!ip.includes("::") && ip.includes(".") && ip.includes(":")) {
    const colonIdx = ip.indexOf(":");
    if (/^\d+$/.test(ip.slice(colonIdx + 1))) {
      ip = ip.slice(0, colonIdx);
    }
  }

  const directV4 = parseIpv4Octets(ip);
  if (directV4) {
    return `ipv4:${directV4[0]}.${directV4[1]}.${directV4[2]}.0/24`;
  }

  const v6Groups = expandIpv6Groups(ip);
  if (v6Groups) {
    // Check IPv4-mapped IPv6 (::ffff:a.b.c.d -> 0000:0000:0000:0000:0000:ffff:xxxx:yyyy)
    if (
      v6Groups[0] === "0000" &&
      v6Groups[1] === "0000" &&
      v6Groups[2] === "0000" &&
      v6Groups[3] === "0000" &&
      v6Groups[4] === "0000" &&
      v6Groups[5] === "ffff"
    ) {
      const hi = Number.parseInt(v6Groups[6], 16);
      const lo = Number.parseInt(v6Groups[7], 16);
      return `ipv4:${(hi >> 8) & 0xff}.${hi & 0xff}.${(lo >> 8) & 0xff}.0/24`;
    }
    return `ipv6:${v6Groups[0]}:${v6Groups[1]}:${v6Groups[2]}::/48`;
  }

  return "unknown";
}

async function computeSubnetHash(rawIp: string, env: Env, utcDate: string): Promise<string> {
  const prefix = extractSubnetPrefix(rawIp);
  const secret = typeof env.SUBNET_HMAC_SECRET === "string" ? env.SUBNET_HMAC_SECRET.trim() : "";
  if (secret.length > 0) {
    return (await hmacSha256Hex(secret, prefix)).slice(0, 32);
  }
  // Test/dev fallback when SUBNET_HMAC_SECRET is not configured: ephemeral daily-salted hash.
  return (await sha256Hex(`${prefix}:${utcDate}:${getDailyRateLimitSalt(utcDate)}`)).slice(0, 32);
}

/**
 * Extra server-side privacy sanitization: redacts URL userinfo credentials,
 * API keys, HuggingFace/Gemini/GitHub tokens, Bearer tokens, x-api-key headers,
 * and Windows/Linux/macOS home directory usernames.
 */
export function sanitizeText(raw: string): string {
  let out = raw;

  // Strip ASCII control chars except tab, LF, CR
  out = out.replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, "");

  // Redact URL userinfo credentials (e.g. https://user:pass@host -> https://<redacted>@host)
  // Word-bounded + capped scheme/userinfo quantifiers guarantee linear-time execution (no ReDoS).
  out = out.replace(
    /\b([A-Za-z][A-Za-z0-9+.-]{0,15}:\/\/)[^/\s\\?#"'<>]{1,512}@([A-Za-z0-9\[])/g,
    "$1<redacted>@$2",
  );

  // Redact OpenAI / DeepSeek / Stripe-style sk- keys
  out = out.replace(/\bsk-[A-Za-z0-9_-]{8,}/g, "[REDACTED_API_KEY]");

  // Redact HuggingFace access tokens (hf_...)
  out = out.replace(/\bhf_[A-Za-z0-9]{10,}/g, "[REDACTED_HF_TOKEN]");

  // Redact Google / Gemini API keys (AIza...)
  out = out.replace(/\bAIza[0-9A-Za-z_-]{20,}/g, "[REDACTED_API_KEY]");

  // Redact GitHub personal access tokens (classic & fine-grained) and OAuth/app tokens
  out = out.replace(/\bgithub_pat_[A-Za-z0-9_]+/g, "[REDACTED_GH_TOKEN]");
  out = out.replace(/\bgh[pousr]_[A-Za-z0-9]+/g, "[REDACTED_GH_TOKEN]");

  // Redact HTTP Bearer tokens
  out = out.replace(/\bBearer\s+[A-Za-z0-9._-]+/gi, "Bearer [REDACTED_TOKEN]");

  // Redact x-api-key headers/params and generic api_key / api-key / apikey assignments
  out = out.replace(
    /(\b(?:x[_-])?api[_-]?key["']?\s*[:=]\s*["']?)(?:\[REDACTED(?:_[A-Z_]+)?\]|[^\s"',}&;\]\)]+)/gi,
    "$1[REDACTED]",
  );

  // Redact Windows user paths (both backslash and forward-slash forms)
  out = out.replace(/([A-Za-z]:\\Users\\)([^\\/\s"'<>|]+)/gi, "$1<USER>");
  out = out.replace(/([A-Za-z]:\/Users\/)([^\\/\s"'<>|]+)/gi, "$1<USER>");

  // Redact Linux /home/<name>/ paths
  out = out.replace(/(^|[^A-Za-z0-9_:])(\/home\/)([^\\/\s"'<>|]+)/g, "$1$2<USER>");

  // Redact macOS /Users/<name>/ paths (excludes drive-letter C:/Users/ handled above)
  out = out.replace(/(^|[^A-Za-z0-9_:])(\/Users\/)([^\\/\s"'<>|]+)/g, "$1$2<USER>");

  return out;
}

/**
 * Neutralize GitHub `@username` and `@org/team` mentions by inserting a zero-width
 * space (`U+200B`) directly after `@` so untrusted telemetry text cannot trigger
 * GitHub mention notifications, while preserving email addresses (`user@example.com`)
 * and normal non-mention text.
 */
export function neutralizeGitHubMentions(raw: string): string {
  return raw.replace(/(^|[^A-Za-z0-9_])@([A-Za-z0-9])/g, "$1@\u200B$2");
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
  const cleaned = neutralizeGitHubMentions(sanitizeText(raw))
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
 * Reads a Web ReadableStream into raw bytes with a strict byte cap so large/unbounded
 * bodies can never exhaust Worker memory.
 */
export async function readBoundedBytes(
  body: ReadableStream<Uint8Array> | null,
  maxBytes = MAX_BODY_BYTES,
  contentLengthHeader?: string | null,
): Promise<Uint8Array> {
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
    return new Uint8Array(0);
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
    return new Uint8Array(0);
  }

  const combined = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    combined.set(chunk, offset);
    offset += chunk.byteLength;
  }

  return combined;
}

/**
 * Reads a Web ReadableStream as UTF-8 text with a strict byte cap.
 */
export async function readBoundedText(
  body: ReadableStream<Uint8Array> | null,
  maxBytes = MAX_BODY_BYTES,
  contentLengthHeader?: string | null,
): Promise<string> {
  const bytes = await readBoundedBytes(body, maxBytes, contentLengthHeader);
  if (bytes.byteLength === 0) {
    return "";
  }
  return new TextDecoder("utf-8").decode(bytes);
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

function validateBoundedInt(
  obj: Record<string, unknown>,
  field: string,
  min: number,
  max: number,
  defaultValue?: number,
): ValidationResult<number> {
  const raw = obj[field];
  if (raw === undefined && defaultValue !== undefined) {
    return { ok: true, data: defaultValue };
  }
  if (typeof raw !== "number" || !Number.isFinite(raw) || !Number.isInteger(raw)) {
    return { ok: false, error: `Field '${field}' must be an integer` };
  }
  if (raw < min || raw > max) {
    return { ok: false, error: `Field '${field}' must be between ${min} and ${max}` };
  }
  return { ok: true, data: raw };
}

function validateBoundedNumber(
  obj: Record<string, unknown>,
  field: string,
  min: number,
  max: number,
  defaultValue?: number,
): ValidationResult<number> {
  const raw = obj[field];
  if (raw === undefined && defaultValue !== undefined) {
    return { ok: true, data: defaultValue };
  }
  if (typeof raw !== "number" || !Number.isFinite(raw)) {
    return { ok: false, error: `Field '${field}' must be a finite number` };
  }
  if (raw < min || raw > max) {
    return { ok: false, error: `Field '${field}' must be between ${min} and ${max}` };
  }
  return { ok: true, data: raw };
}

function validateEnumString(
  obj: Record<string, unknown>,
  field: string,
  allowed: ReadonlySet<string>,
  defaultValue?: string,
): ValidationResult<string> {
  const raw = obj[field];
  if (raw === undefined && defaultValue !== undefined) {
    return { ok: true, data: defaultValue };
  }
  if (typeof raw !== "string") {
    return { ok: false, error: `Field '${field}' must be a string` };
  }
  const trimmed = raw.trim();
  if (!allowed.has(trimmed)) {
    return { ok: false, error: `Field '${field}' has unsupported value` };
  }
  return { ok: true, data: trimmed };
}

function validateRegexString(
  obj: Record<string, unknown>,
  field: string,
  pattern: RegExp,
  maxLen: number,
  defaultValue?: string,
): ValidationResult<string> {
  const raw = obj[field];
  if (raw === undefined && defaultValue !== undefined) {
    return { ok: true, data: defaultValue };
  }
  if (typeof raw !== "string") {
    return { ok: false, error: `Field '${field}' must be a string` };
  }
  const trimmed = raw.trim();
  if (trimmed.length < 1 || trimmed.length > maxLen || !pattern.test(trimmed)) {
    return { ok: false, error: `Field '${field}' is invalid or exceeds ${maxLen} characters` };
  }
  return { ok: true, data: trimmed };
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

export function validateJobMetricsPayload(raw: unknown): ValidationResult<JobMetricsPayload> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Request body must be a JSON object" };
  }

  const appVersion = validateRegexString(raw, "appVersion", SEMVER_RE, 40);
  if (!appVersion.ok) return appVersion;
  const os = validateEnumString(raw, "os", ALLOWED_OS_VALUES);
  if (!os.ok) return os;
  const gpuTier = validateEnumString(raw, "gpuTier", ALLOWED_GPU_TIERS);
  if (!gpuTier.ok) return gpuTier;
  const whisperBackend = validateEnumString(raw, "whisperBackend", ALLOWED_WHISPER_BACKENDS);
  if (!whisperBackend.ok) return whisperBackend;
  const whisperModelId = validateRegexString(raw, "whisperModelId", MODEL_ID_RE, 80);
  if (!whisperModelId.ok) return whisperModelId;
  const llmProvider = validateEnumString(raw, "llmProvider", ALLOWED_LLM_PROVIDERS);
  if (!llmProvider.ok) return llmProvider;
  const llmModelId = validateRegexString(raw, "llmModelId", MODEL_ID_RE, 80);
  if (!llmModelId.ok) return llmModelId;

  const audioDurationMs = validateBoundedInt(raw, "audioDurationMs", 0, 86_400_000);
  if (!audioDurationMs.ok) return audioDurationMs;
  const whisperElapsedMs = validateBoundedInt(raw, "whisperElapsedMs", 0, 86_400_000);
  if (!whisperElapsedMs.ok) return whisperElapsedMs;
  const diarizerElapsedMs = validateBoundedInt(raw, "diarizerElapsedMs", 0, 86_400_000, 0);
  if (!diarizerElapsedMs.ok) return diarizerElapsedMs;
  const llmElapsedMs = validateBoundedInt(raw, "llmElapsedMs", 0, 86_400_000, 0);
  if (!llmElapsedMs.ok) return llmElapsedMs;
  const asrRtf = validateBoundedNumber(raw, "asrRtf", 0, 1000);
  if (!asrRtf.ok) return asrRtf;
  const llmRtf = validateBoundedNumber(raw, "llmRtf", 0, 1000, 0);
  if (!llmRtf.ok) return llmRtf;

  if (typeof raw.oomCpuFallback !== "boolean") {
    return { ok: false, error: "Field 'oomCpuFallback' must be a boolean" };
  }
  const batchSubdivisionRetries = validateBoundedInt(raw, "batchSubdivisionRetries", 0, 10_000, 0);
  if (!batchSubdivisionRetries.ok) return batchSubdivisionRetries;
  const cueCount = validateBoundedInt(raw, "cueCount", 0, 100_000);
  if (!cueCount.ok) return cueCount;

  const sourceLang = validateRegexString(raw, "sourceLang", LANG_CODE_RE, 24);
  if (!sourceLang.ok) return sourceLang;
  const targetLang = validateRegexString(raw, "targetLang", LANG_CODE_RE, 24);
  if (!targetLang.ok) return targetLang;
  const translationStyle = validateEnumString(
    raw,
    "translationStyle",
    ALLOWED_TRANSLATION_STYLES,
    "natural",
  );
  if (!translationStyle.ok) return translationStyle;
  const addressForm = validateEnumString(raw, "addressForm", ALLOWED_ADDRESS_FORMS, "auto");
  if (!addressForm.ok) return addressForm;
  const preset = validateEnumString(raw, "preset", ALLOWED_PRESETS, "standard");
  if (!preset.ok) return preset;

  return {
    ok: true,
    data: {
      appVersion: appVersion.data.replace(/^v/, ""),
      os: os.data,
      gpuTier: gpuTier.data,
      whisperBackend: whisperBackend.data,
      whisperModelId: whisperModelId.data,
      llmProvider: llmProvider.data,
      llmModelId: llmModelId.data,
      audioDurationMs: audioDurationMs.data,
      whisperElapsedMs: whisperElapsedMs.data,
      diarizerElapsedMs: diarizerElapsedMs.data,
      llmElapsedMs: llmElapsedMs.data,
      asrRtf: asrRtf.data,
      llmRtf: llmRtf.data,
      oomCpuFallback: raw.oomCpuFallback,
      batchSubdivisionRetries: batchSubdivisionRetries.data,
      cueCount: cueCount.data,
      sourceLang: sourceLang.data,
      targetLang: targetLang.data,
      translationStyle: translationStyle.data,
      addressForm: addressForm.data,
      preset: preset.data,
    },
  };
}

function validateExportDiffMetrics(raw: unknown): ValidationResult<ExportDiffMetrics> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Field 'metrics' must be a JSON object" };
  }

  const totalBaselineCues = validateBoundedInt(raw, "totalBaselineCues", 0, 100_000);
  if (!totalBaselineCues.ok) return totalBaselineCues;
  const totalFinalCues = validateBoundedInt(raw, "totalFinalCues", 0, 100_000);
  if (!totalFinalCues.ok) return totalFinalCues;
  const unchangedCuesCount = validateBoundedInt(raw, "unchangedCuesCount", 0, 100_000);
  if (!unchangedCuesCount.ok) return unchangedCuesCount;
  const textEditedCuesCount = validateBoundedInt(raw, "textEditedCuesCount", 0, 100_000);
  if (!textEditedCuesCount.ok) return textEditedCuesCount;
  const editRatePct = validateBoundedNumber(raw, "editRatePct", 0, 100);
  if (!editRatePct.ok) return editRatePct;

  const initialHighCpsCount = validateBoundedInt(raw, "initialHighCpsCount", 0, 100_000);
  if (!initialHighCpsCount.ok) return initialHighCpsCount;
  const finalHighCpsCount = validateBoundedInt(raw, "finalHighCpsCount", 0, 100_000);
  if (!finalHighCpsCount.ok) return finalHighCpsCount;
  const initialLineTooLongCount = validateBoundedInt(raw, "initialLineTooLongCount", 0, 100_000);
  if (!initialLineTooLongCount.ok) return initialLineTooLongCount;
  const finalLineTooLongCount = validateBoundedInt(raw, "finalLineTooLongCount", 0, 100_000);
  if (!finalLineTooLongCount.ok) return finalLineTooLongCount;

  const avgCpsBefore = validateBoundedNumber(raw, "avgCpsBefore", 0, 500);
  if (!avgCpsBefore.ok) return avgCpsBefore;
  const avgCpsAfter = validateBoundedNumber(raw, "avgCpsAfter", 0, 500);
  if (!avgCpsAfter.ok) return avgCpsAfter;
  if (typeof raw.condenseCpsUsed !== "boolean") {
    return { ok: false, error: "Field 'condenseCpsUsed' must be a boolean" };
  }

  const encliticReorderedCount = validateBoundedInt(raw, "encliticReorderedCount", 0, 100_000);
  if (!encliticReorderedCount.ok) return encliticReorderedCount;
  const redundantPossessiveRemovedCount = validateBoundedInt(
    raw,
    "redundantPossessiveRemovedCount",
    0,
    100_000,
  );
  if (!redundantPossessiveRemovedCount.ok) return redundantPossessiveRemovedCount;
  const passiveCalqueRemovedCount = validateBoundedInt(
    raw,
    "passiveCalqueRemovedCount",
    0,
    100_000,
  );
  if (!passiveCalqueRemovedCount.ok) return passiveCalqueRemovedCount;
  const conjunctionCommaFixedCount = validateBoundedInt(
    raw,
    "conjunctionCommaFixedCount",
    0,
    100_000,
  );
  if (!conjunctionCommaFixedCount.ok) return conjunctionCommaFixedCount;
  const missedFillerDeletedCount = validateBoundedInt(
    raw,
    "missedFillerDeletedCount",
    0,
    100_000,
  );
  if (!missedFillerDeletedCount.ok) return missedFillerDeletedCount;
  const inflectionSuffixFixCount = validateBoundedInt(
    raw,
    "inflectionSuffixFixCount",
    0,
    100_000,
  );
  if (!inflectionSuffixFixCount.ok) return inflectionSuffixFixCount;
  const boundaryOrphanShiftedCount = validateBoundedInt(
    raw,
    "boundaryOrphanShiftedCount",
    0,
    100_000,
  );
  if (!boundaryOrphanShiftedCount.ok) return boundaryOrphanShiftedCount;

  const splitCount = validateBoundedInt(raw, "splitCount", 0, 100_000);
  if (!splitCount.ok) return splitCount;
  const splitAfterCommaCount = validateBoundedInt(raw, "splitAfterCommaCount", 0, 100_000);
  if (!splitAfterCommaCount.ok) return splitAfterCommaCount;
  const splitAfterConjunctionCount = validateBoundedInt(
    raw,
    "splitAfterConjunctionCount",
    0,
    100_000,
  );
  if (!splitAfterConjunctionCount.ok) return splitAfterConjunctionCount;
  const avgPreSplitChars = validateBoundedNumber(raw, "avgPreSplitChars", 0, 10_000);
  if (!avgPreSplitChars.ok) return avgPreSplitChars;
  const avgPreSplitDurationMs = validateBoundedNumber(
    raw,
    "avgPreSplitDurationMs",
    0,
    3_600_000,
  );
  if (!avgPreSplitDurationMs.ok) return avgPreSplitDurationMs;

  const mergeCount = validateBoundedInt(raw, "mergeCount", 0, 100_000);
  if (!mergeCount.ok) return mergeCount;
  const avgMergedShortCueDurationMs = validateBoundedNumber(
    raw,
    "avgMergedShortCueDurationMs",
    0,
    3_600_000,
  );
  if (!avgMergedShortCueDurationMs.ok) return avgMergedShortCueDurationMs;
  const avgMergedShortCueChars = validateBoundedNumber(
    raw,
    "avgMergedShortCueChars",
    0,
    10_000,
  );
  if (!avgMergedShortCueChars.ok) return avgMergedShortCueChars;
  const avgMergedGapMs = validateBoundedNumber(raw, "avgMergedGapMs", 0, 3_600_000);
  if (!avgMergedGapMs.ok) return avgMergedGapMs;

  const startTrimCount = validateBoundedInt(raw, "startTrimCount", 0, 100_000);
  if (!startTrimCount.ok) return startTrimCount;
  const avgStartDeltaMs = validateBoundedNumber(raw, "avgStartDeltaMs", -3_600_000, 3_600_000);
  if (!avgStartDeltaMs.ok) return avgStartDeltaMs;
  const endTrimCount = validateBoundedInt(raw, "endTrimCount", 0, 100_000);
  if (!endTrimCount.ok) return endTrimCount;
  const avgEndDeltaMs = validateBoundedNumber(raw, "avgEndDeltaMs", -3_600_000, 3_600_000);
  if (!avgEndDeltaMs.ok) return avgEndDeltaMs;

  return {
    ok: true,
    data: {
      totalBaselineCues: totalBaselineCues.data,
      totalFinalCues: totalFinalCues.data,
      unchangedCuesCount: unchangedCuesCount.data,
      textEditedCuesCount: textEditedCuesCount.data,
      editRatePct: editRatePct.data,
      initialHighCpsCount: initialHighCpsCount.data,
      finalHighCpsCount: finalHighCpsCount.data,
      initialLineTooLongCount: initialLineTooLongCount.data,
      finalLineTooLongCount: finalLineTooLongCount.data,
      avgCpsBefore: avgCpsBefore.data,
      avgCpsAfter: avgCpsAfter.data,
      condenseCpsUsed: raw.condenseCpsUsed,
      encliticReorderedCount: encliticReorderedCount.data,
      redundantPossessiveRemovedCount: redundantPossessiveRemovedCount.data,
      passiveCalqueRemovedCount: passiveCalqueRemovedCount.data,
      conjunctionCommaFixedCount: conjunctionCommaFixedCount.data,
      missedFillerDeletedCount: missedFillerDeletedCount.data,
      inflectionSuffixFixCount: inflectionSuffixFixCount.data,
      boundaryOrphanShiftedCount: boundaryOrphanShiftedCount.data,
      splitCount: splitCount.data,
      splitAfterCommaCount: splitAfterCommaCount.data,
      splitAfterConjunctionCount: splitAfterConjunctionCount.data,
      avgPreSplitChars: avgPreSplitChars.data,
      avgPreSplitDurationMs: avgPreSplitDurationMs.data,
      mergeCount: mergeCount.data,
      avgMergedShortCueDurationMs: avgMergedShortCueDurationMs.data,
      avgMergedShortCueChars: avgMergedShortCueChars.data,
      avgMergedGapMs: avgMergedGapMs.data,
      startTrimCount: startTrimCount.data,
      avgStartDeltaMs: avgStartDeltaMs.data,
      endTrimCount: endTrimCount.data,
      avgEndDeltaMs: avgEndDeltaMs.data,
    },
  };
}

/**
 * Validates Level 2B n-gram spans: strictly lowercase letters, spaces, hyphens, or
 * apostrophes only; rejects any uppercase characters, digits, symbols, URLs, or emails.
 */
export function normalizeAndValidateNgramSpan(
  raw: unknown,
  allowEmpty: boolean,
): ValidationResult<string> {
  if (typeof raw !== "string") {
    return { ok: false, error: "n-gram span must be a string" };
  }
  const normalized = raw.replace(/\s+/g, " ").trim();
  if (normalized.length === 0) {
    return allowEmpty
      ? { ok: true, data: "" }
      : { ok: false, error: "n-gram oldSpan cannot be empty" };
  }
  if (normalized.length > 80) {
    return { ok: false, error: "n-gram span exceeds maximum length" };
  }
  if (
    UPPERCASE_CHAR_RE.test(normalized) ||
    DIGIT_CHAR_RE.test(normalized) ||
    !NGRAM_ALLOWED_CHARS_RE.test(normalized)
  ) {
    return {
      ok: false,
      error: "n-gram span must contain only lowercase letters, spaces, hyphens, or apostrophes",
    };
  }

  const words = normalized.split(" ");
  if (words.length > 3) {
    return { ok: false, error: "n-gram span cannot exceed 3 words" };
  }
  for (const word of words) {
    if (word.length > 24 || !LOWERCASE_LETTER_RE.test(word)) {
      return { ok: false, error: "n-gram word is invalid or exceeds 24 characters" };
    }
  }
  if (words.length === 1 && words[0].length < 2) {
    return { ok: false, error: "single-word n-gram span must be at least 2 characters" };
  }

  return { ok: true, data: normalized };
}

export function validateExportTelemetryPayload(
  raw: unknown,
): ValidationResult<ExportTelemetryPayload> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Request body must be a JSON object" };
  }

  const appVersion = validateRegexString(raw, "appVersion", SEMVER_RE, 40);
  if (!appVersion.ok) return appVersion;
  const os = validateEnumString(raw, "os", ALLOWED_OS_VALUES);
  if (!os.ok) return os;
  const sourceLang = validateRegexString(raw, "sourceLang", LANG_CODE_RE, 24);
  if (!sourceLang.ok) return sourceLang;
  const targetLang = validateRegexString(raw, "targetLang", LANG_CODE_RE, 24);
  if (!targetLang.ok) return targetLang;
  const translationStyle = validateEnumString(
    raw,
    "translationStyle",
    ALLOWED_TRANSLATION_STYLES,
    "natural",
  );
  if (!translationStyle.ok) return translationStyle;
  const addressForm = validateEnumString(raw, "addressForm", ALLOWED_ADDRESS_FORMS, "auto");
  if (!addressForm.ok) return addressForm;

  const metrics = validateExportDiffMetrics(raw.metrics);
  if (!metrics.ok) return metrics;

  const ngramDiffs: SafeNgramDiff[] = [];
  if (raw.ngramDiffs !== undefined) {
    if (!Array.isArray(raw.ngramDiffs)) {
      return { ok: false, error: "Field 'ngramDiffs' must be an array" };
    }
    if (raw.ngramDiffs.length > MAX_NGRAM_DIFFS_PER_EXPORT) {
      return {
        ok: false,
        error: `Field 'ngramDiffs' cannot exceed ${MAX_NGRAM_DIFFS_PER_EXPORT} items`,
      };
    }

    for (let i = 0; i < raw.ngramDiffs.length; i++) {
      const item = raw.ngramDiffs[i];
      if (!isPlainRecord(item)) {
        return { ok: false, error: `ngramDiffs[${i}] must be an object` };
      }
      const langPair = validateRegexString(item, "langPair", LANG_PAIR_RE, 24);
      if (!langPair.ok) {
        return { ok: false, error: `ngramDiffs[${i}].langPair is invalid` };
      }
      const oldSpan = normalizeAndValidateNgramSpan(item.oldSpan, false);
      if (!oldSpan.ok) {
        return { ok: false, error: `ngramDiffs[${i}].oldSpan: ${oldSpan.error}` };
      }
      const newSpan = normalizeAndValidateNgramSpan(item.newSpan, true);
      if (!newSpan.ok) {
        return { ok: false, error: `ngramDiffs[${i}].newSpan: ${newSpan.error}` };
      }
      if (oldSpan.data === newSpan.data) {
        return { ok: false, error: `ngramDiffs[${i}] oldSpan and newSpan cannot be identical` };
      }
      const cohortProof = validateRegexString(item, "cohortProof", COHORT_PROOF_RE, 64);
      if (!cohortProof.ok) {
        return { ok: false, error: `ngramDiffs[${i}].cohortProof must be 16..64 hex chars` };
      }

      ngramDiffs.push({
        langPair: langPair.data,
        oldSpan: oldSpan.data,
        newSpan: newSpan.data,
        cohortProof: cohortProof.data.toLowerCase(),
      });
    }
  }

  return {
    ok: true,
    data: {
      appVersion: appVersion.data.replace(/^v/, ""),
      os: os.data,
      sourceLang: sourceLang.data,
      targetLang: targetLang.data,
      translationStyle: translationStyle.data,
      addressForm: addressForm.data,
      metrics: metrics.data,
      ngramDiffs,
    },
  };
}

export function validateTrainingPairsPayload(
  raw: unknown,
): ValidationResult<TrainingPairsPayload> {
  if (!isPlainRecord(raw)) {
    return { ok: false, error: "Request body must be a JSON object" };
  }

  const appVersion = validateRegexString(raw, "appVersion", SEMVER_RE, 40);
  if (!appVersion.ok) return appVersion;

  if (!Array.isArray(raw.pairs)) {
    return { ok: false, error: "Field 'pairs' is required and must be an array" };
  }
  if (raw.pairs.length > MAX_TRAINING_PAIRS_PER_REQUEST) {
    return {
      ok: false,
      error: `Field 'pairs' cannot exceed ${MAX_TRAINING_PAIRS_PER_REQUEST} items`,
    };
  }

  const pairs: TrainingCuePair[] = [];
  for (let i = 0; i < raw.pairs.length; i++) {
    const item = raw.pairs[i];
    if (!isPlainRecord(item)) {
      return { ok: false, error: `pairs[${i}] must be an object` };
    }
    const sourceLang = validateRegexString(item, "sourceLang", LANG_CODE_RE, 24);
    if (!sourceLang.ok) return { ok: false, error: `pairs[${i}].sourceLang is invalid` };
    const targetLang = validateRegexString(item, "targetLang", LANG_CODE_RE, 24);
    if (!targetLang.ok) return { ok: false, error: `pairs[${i}].targetLang is invalid` };
    const style = validateEnumString(item, "style", ALLOWED_TRANSLATION_STYLES, "natural");
    if (!style.ok) return { ok: false, error: `pairs[${i}].style is invalid` };
    const addressForm = validateEnumString(item, "addressForm", ALLOWED_ADDRESS_FORMS, "auto");
    if (!addressForm.ok) return { ok: false, error: `pairs[${i}].addressForm is invalid` };

    for (const textField of ["sourceText", "initialAiText", "finalHumanText"] as const) {
      const val = item[textField];
      if (typeof val !== "string" || val.trim().length === 0 || val.length > 2000) {
        return {
          ok: false,
          error: `pairs[${i}].${textField} must be a non-empty string (<= 2000 chars)`,
        };
      }
    }

    const sourceText = sanitizeText((item.sourceText as string).trim());
    const initialAiText = sanitizeText((item.initialAiText as string).trim());
    const finalHumanText = sanitizeText((item.finalHumanText as string).trim());

    if (!sourceText || !initialAiText || !finalHumanText) {
      return { ok: false, error: `pairs[${i}] text fields cannot be empty after sanitization` };
    }

    pairs.push({
      sourceLang: sourceLang.data,
      targetLang: targetLang.data,
      style: style.data,
      addressForm: addressForm.data,
      sourceText: truncateText(sourceText, 2000, "..."),
      initialAiText: truncateText(initialAiText, 2000, "..."),
      finalHumanText: truncateText(finalHumanText, 2000, "..."),
    });
  }

  return {
    ok: true,
    data: {
      appVersion: appVersion.data.replace(/^v/, ""),
      pairs,
    },
  };
}

function buildDiagnosticsDetailsBlock(diagnostics: string | null | undefined, maxChars: number): string {
  if (!diagnostics || diagnostics.trim().length === 0) {
    return "";
  }
  const sanitized = truncateText(
    neutralizeGitHubMentions(sanitizeText(diagnostics.trim())),
    maxChars,
  );
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
  const cleanDescription = truncateText(
    neutralizeGitHubMentions(sanitizeText(payload.description.trim())),
    8000,
  );
  const cleanSteps = payload.steps?.trim()
    ? truncateText(neutralizeGitHubMentions(sanitizeText(payload.steps.trim())), 4000)
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
  const cleanMessage = truncateText(
    neutralizeGitHubMentions(sanitizeText(payload.message.trim())),
    4000,
  );
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
    ? escapeCodeFence(
        truncateText(
          neutralizeGitHubMentions(sanitizeText(payload.stackTrace.trim())),
          16_000,
        ),
      )
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
  const cleanMessage = truncateText(
    neutralizeGitHubMentions(sanitizeText(payload.message.trim())),
    4000,
  );
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
    ? escapeCodeFence(
        truncateText(
          neutralizeGitHubMentions(sanitizeText(payload.stackTrace.trim())),
          12_000,
        ),
      )
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

/**
 * Zero-IP rate limiter (`V05-H1`): hashes `ip + ":" + utcDate + ":" + dailySalt`
 * with SHA-256 before constructing the Cache API key so no raw IP is ever stored.
 */
export async function checkRateLimit(
  request: Request,
  route: RateLimitRoute,
  nowMs = Date.now(),
  env?: Env,
): Promise<RateLimitResult> {
  const limit = RATE_LIMITS[route];
  const cache = getDefaultCache();
  if (!cache) {
    return { allowed: true, remaining: limit, retryAfterSeconds: 0 };
  }

  const ip = extractClientIp(request);
  const utcDate = new Date(nowMs).toISOString().slice(0, 10);
  // When SUBNET_HMAC_SECRET is configured the daily salt is derived from it (consistent
  // across isolates within a UTC day); otherwise an ephemeral per-isolate random salt is used.
  const secret =
    typeof env?.SUBNET_HMAC_SECRET === "string" ? env.SUBNET_HMAC_SECRET.trim() : "";
  const dailySalt =
    secret.length > 0
      ? await hmacSha256Hex(secret, `rate-limit-salt:${utcDate}`)
      : getDailyRateLimitSalt(utcDate);
  const ipHash = await sha256Hex(`${ip}:${utcDate}:${dailySalt}`);

  const windowMs = RATE_LIMIT_WINDOW_SECONDS * 1000;
  const bucket = Math.floor(nowMs / windowMs);
  const defaultResetAtMs = (bucket + 1) * windowMs;
  const cacheUrl = `https://rate-limit.easysrt.internal/${route}/${utcDate}/${ipHash}/${bucket}`;
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

function resolveGitHubRepo(env: Env): string | null {
  const repo = typeof env.GITHUB_REPO === "string" ? env.GITHUB_REPO.trim() : "";
  return GITHUB_REPO_RE.test(repo) ? repo : null;
}

function resolveGitHubConfig(
  env: Env,
): { ok: true; repo: string; token: string } | { ok: false } {
  const repo = resolveGitHubRepo(env);
  const token =
    typeof env.GITHUB_ISSUES_TOKEN === "string" ? env.GITHUB_ISSUES_TOKEN.trim() : "";

  if (!repo || token.length === 0) {
    return { ok: false };
  }

  return { ok: true, repo, token };
}

async function parseJsonRequestBody(
  request: Request,
  maxBytes = MAX_BODY_BYTES,
): Promise<{ ok: true; data: unknown } | { ok: false; response: Response }> {
  let rawText: string;
  try {
    rawText = await readBoundedText(
      request.body,
      maxBytes,
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

async function scheduleBackground(ctx: ExecutionContext | undefined, work: Promise<unknown>): Promise<void> {
  const safeWork = work.catch((err) => {
    console.error(
      JSON.stringify({
        event: "background_task_error",
        error: err instanceof Error ? err.message : String(err),
      }),
    );
  });
  if (ctx && typeof ctx.waitUntil === "function") {
    ctx.waitUntil(safeWork);
    return;
  }
  await safeWork;
}

async function handleBugReport(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "bug", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request, MAX_BODY_BYTES);
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
  const rateLimit = await checkRateLimit(request, "crash", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request, MAX_BODY_BYTES);
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

function parseAllowlistedUpdateQuery(
  url: URL,
): { target: string; arch: string; currentVersion: string } | null {
  const target = url.searchParams.get("target")?.trim().toLowerCase() ?? "";
  const arch = url.searchParams.get("arch")?.trim().toLowerCase() ?? "";
  const currentRaw = url.searchParams.get("current")?.trim() ?? "";

  if (!ALLOWED_UPDATE_TARGETS.has(target) || !ALLOWED_UPDATE_ARCHES.has(arch)) {
    return null;
  }
  if (currentRaw.length < 1 || currentRaw.length > 40 || !SEMVER_RE.test(currentRaw)) {
    return null;
  }
  return {
    target,
    arch,
    currentVersion: currentRaw.replace(/^v/, ""),
  };
}

function validateUpdateManifestBytes(rawBytes: Uint8Array): boolean {
  if (rawBytes.byteLength === 0) {
    return false;
  }
  try {
    const text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(rawBytes);
    const parsed = JSON.parse(text) as unknown;
    return (
      isPlainRecord(parsed) &&
      typeof parsed.version === "string" &&
      parsed.version.trim().length > 0 &&
      isPlainRecord(parsed.platforms)
    );
  } catch {
    return false;
  }
}

async function handleUpdateManifest(
  request: Request,
  env: Env,
  ctx?: ExecutionContext,
): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "update", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const url = new URL(request.url);
  const updateCheck = parseAllowlistedUpdateQuery(url);
  const repo = resolveGitHubRepo(env);
  if (!repo) {
    console.error(JSON.stringify({ event: "misconfigured_env", route: "update" }));
    return jsonResponse({ ok: false, error: "Update service is not configured" }, 500);
  }

  const cache = getDefaultCache();
  // Fixed cache key with NO query string so edge cache cannot be poisoned/fragmented.
  const cacheKey = new Request(FIXED_UPDATE_CACHE_URL, { method: "GET" });

  let rawBytes: Uint8Array | null = null;
  let cacheStatus: "HIT" | "MISS" = "MISS";

  if (cache) {
    try {
      const cached = await cache.match(cacheKey);
      if (cached && cached.ok) {
        const cachedBytes = await readBoundedBytes(
          cached.body,
          MAX_UPDATE_MANIFEST_BYTES,
          cached.headers.get("content-length"),
        );
        if (validateUpdateManifestBytes(cachedBytes)) {
          rawBytes = cachedBytes;
          cacheStatus = "HIT";
        }
      }
    } catch (err) {
      console.error(
        JSON.stringify({
          event: "update_cache_read_error",
          error: err instanceof Error ? err.message : String(err),
        }),
      );
    }
  }

  if (!rawBytes) {
    const upstreamUrl = `https://github.com/${repo}/releases/latest/download/latest.json`;
    const upstreamResp = await fetch(upstreamUrl, {
      method: "GET",
      headers: {
        "User-Agent": GITHUB_USER_AGENT,
        Accept: "application/json",
      },
    });

    if (!upstreamResp.ok) {
      console.error(
        JSON.stringify({
          event: "update_upstream_failed",
          status: upstreamResp.status,
        }),
      );
      return jsonResponse({ ok: false, error: "Failed to fetch upstream update manifest" }, 502);
    }

    let fetchedBytes: Uint8Array;
    try {
      fetchedBytes = await readBoundedBytes(
        upstreamResp.body,
        MAX_UPDATE_MANIFEST_BYTES,
        upstreamResp.headers.get("content-length"),
      );
    } catch (err) {
      if (err instanceof PayloadTooLargeError) {
        return jsonResponse({ ok: false, error: "Upstream update manifest exceeds 64 KiB cap" }, 502);
      }
      throw err;
    }

    if (!validateUpdateManifestBytes(fetchedBytes)) {
      console.error(JSON.stringify({ event: "update_upstream_invalid_manifest" }));
      return jsonResponse({ ok: false, error: "Invalid upstream update manifest" }, 502);
    }

    rawBytes = fetchedBytes;

    if (cache) {
      const toCache = new Response(rawBytes.slice(), {
        status: 200,
        headers: {
          "Content-Type": "application/json; charset=utf-8",
          "Cache-Control": `public, max-age=${UPDATE_CACHE_TTL_SECONDS}`,
        },
      });
      await scheduleBackground(ctx, cache.put(cacheKey, toCache));
    }
  }

  if (updateCheck && env.DB) {
    const utcDate = new Date().toISOString().slice(0, 10);
    const dbWrite = env.DB.prepare(
      `INSERT INTO daily_update_checks (utc_date, target, arch, current_version, check_count)
       VALUES (?, ?, ?, ?, 1)
       ON CONFLICT (utc_date, target, arch, current_version)
       DO UPDATE SET check_count = check_count + 1`,
    )
      .bind(utcDate, updateCheck.target, updateCheck.arch, updateCheck.currentVersion)
      .run();
    await scheduleBackground(ctx, dbWrite);
  }

  return new Response(rawBytes.slice(), {
    status: 200,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": `public, max-age=${UPDATE_CACHE_TTL_SECONDS}`,
      "X-Cache": cacheStatus,
    },
  });
}

async function handleJobMetrics(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "jobMetrics", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request, MAX_TELEMETRY_BODY_BYTES);
  if (!parsedBody.ok) {
    return parsedBody.response;
  }

  const validated = validateJobMetricsPayload(parsedBody.data);
  if (!validated.ok) {
    return jsonResponse({ ok: false, error: validated.error }, 400);
  }

  if (!env.DB) {
    console.error(JSON.stringify({ event: "misconfigured_db", route: "job_metrics" }));
    return jsonResponse({ ok: false, error: "Telemetry database is not configured" }, 500);
  }

  const nowIso = new Date().toISOString();
  const utcDate = nowIso.slice(0, 10);
  const d = validated.data;

  await env.DB.prepare(
    `INSERT INTO job_and_edit_metrics (
      created_at, utc_date, event_kind, app_version, os, source_lang, target_lang,
      translation_style, address_form, gpu_tier, whisper_backend, whisper_model_id,
      llm_provider, llm_model_id, audio_duration_ms, whisper_elapsed_ms, diarizer_elapsed_ms,
      llm_elapsed_ms, asr_rtf, llm_rtf, oom_cpu_fallback, batch_subdivision_retries,
      cue_count, preset
    ) VALUES (?, ?, 'job', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  )
    .bind(
      nowIso,
      utcDate,
      d.appVersion,
      d.os,
      d.sourceLang,
      d.targetLang,
      d.translationStyle,
      d.addressForm,
      d.gpuTier,
      d.whisperBackend,
      d.whisperModelId,
      d.llmProvider,
      d.llmModelId,
      d.audioDurationMs,
      d.whisperElapsedMs,
      d.diarizerElapsedMs,
      d.llmElapsedMs,
      d.asrRtf,
      d.llmRtf,
      d.oomCpuFallback ? 1 : 0,
      d.batchSubdivisionRetries,
      d.cueCount,
      d.preset,
    )
    .run();

  return jsonResponse({ ok: true }, 201);
}

async function handleExportTelemetry(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "export", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request, MAX_TELEMETRY_BODY_BYTES);
  if (!parsedBody.ok) {
    return parsedBody.response;
  }

  const validated = validateExportTelemetryPayload(parsedBody.data);
  if (!validated.ok) {
    return jsonResponse({ ok: false, error: validated.error }, 400);
  }

  if (!env.DB) {
    console.error(JSON.stringify({ event: "misconfigured_db", route: "export" }));
    return jsonResponse({ ok: false, error: "Telemetry database is not configured" }, 500);
  }

  const nowIso = new Date().toISOString();
  const utcDate = nowIso.slice(0, 10);
  const d = validated.data;
  const m = d.metrics;

  await env.DB.prepare(
    `INSERT INTO job_and_edit_metrics (
      created_at, utc_date, event_kind, app_version, os, source_lang, target_lang,
      translation_style, address_form,
      total_baseline_cues, total_final_cues, unchanged_cues_count, text_edited_cues_count,
      edit_rate_pct, initial_high_cps_count, final_high_cps_count, initial_line_too_long_count,
      final_line_too_long_count, avg_cps_before, avg_cps_after, condense_cps_used,
      enclitic_reordered_count, redundant_possessive_removed_count, passive_calque_removed_count,
      conjunction_comma_fixed_count, missed_filler_deleted_count, inflection_suffix_fix_count,
      boundary_orphan_shifted_count, split_count, split_after_comma_count,
      split_after_conjunction_count, avg_pre_split_chars, avg_pre_split_duration_ms,
      merge_count, avg_merged_short_cue_duration_ms, avg_merged_short_cue_chars,
      avg_merged_gap_ms, start_trim_count, avg_start_delta_ms, end_trim_count, avg_end_delta_ms
    ) VALUES (
      ?, ?, 'export', ?, ?, ?, ?, ?, ?,
      ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
      ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
      ?, ?, ?, ?, ?, ?, ?, ?
    )`,
  )
    .bind(
      nowIso,
      utcDate,
      d.appVersion,
      d.os,
      d.sourceLang,
      d.targetLang,
      d.translationStyle,
      d.addressForm,
      m.totalBaselineCues,
      m.totalFinalCues,
      m.unchangedCuesCount,
      m.textEditedCuesCount,
      m.editRatePct,
      m.initialHighCpsCount,
      m.finalHighCpsCount,
      m.initialLineTooLongCount,
      m.finalLineTooLongCount,
      m.avgCpsBefore,
      m.avgCpsAfter,
      m.condenseCpsUsed ? 1 : 0,
      m.encliticReorderedCount,
      m.redundantPossessiveRemovedCount,
      m.passiveCalqueRemovedCount,
      m.conjunctionCommaFixedCount,
      m.missedFillerDeletedCount,
      m.inflectionSuffixFixCount,
      m.boundaryOrphanShiftedCount,
      m.splitCount,
      m.splitAfterCommaCount,
      m.splitAfterConjunctionCount,
      m.avgPreSplitChars,
      m.avgPreSplitDurationMs,
      m.mergeCount,
      m.avgMergedShortCueDurationMs,
      m.avgMergedShortCueChars,
      m.avgMergedGapMs,
      m.startTrimCount,
      m.avgStartDeltaMs,
      m.endTrimCount,
      m.avgEndDeltaMs,
    )
    .run();

  let recordedNgrams = 0;
  if (d.ngramDiffs.length > 0) {
    const clientIp = extractClientIp(request);
    const subnetHash = await computeSubnetHash(clientIp, env, utcDate);
    const seenInRequest = new Set<string>();

    for (const diff of d.ngramDiffs) {
      const pairHash = await sha256Hex(`${diff.langPair}|${diff.oldSpan}->${diff.newSpan}`);
      const dedupeKey = `${pairHash}:${diff.cohortProof}`;
      if (seenInRequest.has(dedupeKey)) {
        continue;
      }
      seenInRequest.add(dedupeKey);

      const cohortInsert = await env.DB.prepare(
        `INSERT OR IGNORE INTO ngram_cohorts (pair_hash, cohort_proof, subnet_hash, created_at)
         VALUES (?, ?, ?, ?)`,
      )
        .bind(pairHash, diff.cohortProof, subnetHash, nowIso)
        .run();

      if ((cohortInsert.meta?.changes ?? 0) > 0) {
        await env.DB.prepare(
          `INSERT INTO ngram_candidates (
            pair_hash, lang_pair, old_span, new_span, cohort_count, subnet_count, created_at, updated_at
          )
          VALUES (
            ?, ?, ?, ?,
            (SELECT COUNT(*) FROM ngram_cohorts WHERE pair_hash = ?),
            (SELECT COUNT(DISTINCT subnet_hash) FROM ngram_cohorts WHERE pair_hash = ?),
            ?, ?
          )
          ON CONFLICT(pair_hash) DO UPDATE SET
            cohort_count = (SELECT COUNT(*) FROM ngram_cohorts WHERE pair_hash = excluded.pair_hash),
            subnet_count = (SELECT COUNT(DISTINCT subnet_hash) FROM ngram_cohorts WHERE pair_hash = excluded.pair_hash),
            updated_at = excluded.updated_at`,
        )
          .bind(
            pairHash,
            diff.langPair,
            diff.oldSpan,
            diff.newSpan,
            pairHash,
            pairHash,
            nowIso,
            nowIso,
          )
          .run();
        recordedNgrams += 1;
      }
    }
  }

  return jsonResponse({ ok: true, recordedNgrams }, 201);
}

async function handleTrainingPairs(request: Request, env: Env): Promise<Response> {
  const rateLimit = await checkRateLimit(request, "trainingPairs", Date.now(), env);
  if (!rateLimit.allowed) {
    return jsonResponse(
      { ok: false, error: "Rate limit exceeded. Please try again later." },
      429,
      { "Retry-After": String(rateLimit.retryAfterSeconds) },
    );
  }

  const parsedBody = await parseJsonRequestBody(request, MAX_TRAINING_PAIRS_BODY_BYTES);
  if (!parsedBody.ok) {
    return parsedBody.response;
  }

  const validated = validateTrainingPairsPayload(parsedBody.data);
  if (!validated.ok) {
    return jsonResponse({ ok: false, error: validated.error }, 400);
  }

  if (!env.DB) {
    console.error(JSON.stringify({ event: "misconfigured_db", route: "training_pairs" }));
    return jsonResponse({ ok: false, error: "Telemetry database is not configured" }, 500);
  }

  if (validated.data.pairs.length === 0) {
    return jsonResponse({ ok: true, inserted: 0 }, 200);
  }

  const nowIso = new Date().toISOString();
  const stmt = env.DB.prepare(
    `INSERT INTO training_edit_pairs (
      created_at, app_version, source_lang, target_lang, style, address_form,
      source_text, initial_ai_text, final_human_text
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  );

  const batchStatements = validated.data.pairs.map((p) =>
    stmt.bind(
      nowIso,
      validated.data.appVersion,
      p.sourceLang,
      p.targetLang,
      p.style,
      p.addressForm,
      p.sourceText,
      p.initialAiText,
      p.finalHumanText,
    ),
  );

  await env.DB.batch(batchStatements);

  return jsonResponse({ ok: true, inserted: validated.data.pairs.length }, 201);
}

async function handleAdminDigest(request: Request, env: Env): Promise<Response> {
  const expectedToken =
    typeof env.TELEMETRY_ADMIN_TOKEN === "string" ? env.TELEMETRY_ADMIN_TOKEN.trim() : "";
  if (expectedToken.length === 0) {
    console.error(JSON.stringify({ event: "misconfigured_admin_token" }));
    return jsonResponse({ ok: false, error: "Admin telemetry token is not configured" }, 500);
  }

  const authHeader = request.headers.get("Authorization")?.trim() ?? "";
  const match = /^Bearer\s+(.+)$/i.exec(authHeader);
  const providedToken = match?.[1]?.trim() ?? "";
  if (!(await timingSafeEqualStrings(providedToken, expectedToken))) {
    return jsonResponse(
      { ok: false, error: "Unauthorized" },
      401,
      { "WWW-Authenticate": "Bearer" },
    );
  }

  if (!env.DB) {
    console.error(JSON.stringify({ event: "misconfigured_db", route: "admin_digest" }));
    return jsonResponse({ ok: false, error: "Telemetry database is not configured" }, 500);
  }

  const url = new URL(request.url);
  const daysParam = url.searchParams.get("days");
  let days = 30;
  if (daysParam !== null && daysParam.trim() !== "") {
    if (!/^-?\d+$/.test(daysParam.trim())) {
      return jsonResponse({ ok: false, error: "Query parameter 'days' must be an integer" }, 400);
    }
    const parsedDays = Number.parseInt(daysParam.trim(), 10);
    days = Math.min(365, Math.max(1, parsedDays));
  }

  const format = (url.searchParams.get("format") ?? "json").trim().toLowerCase();
  if (format !== "json" && format !== "jsonl") {
    return jsonResponse({ ok: false, error: "Query parameter 'format' must be json or jsonl" }, 400);
  }

  const nowMs = Date.now();
  const cutoffMs = nowMs - days * 86_400_000;
  const cutoffDate = new Date(cutoffMs).toISOString().slice(0, 10);
  const cutoffIso = new Date(cutoffMs).toISOString();

  if (format === "jsonl") {
    const rows = await env.DB.prepare(
      `SELECT created_at, app_version, source_lang, target_lang, style, address_form,
              source_text, initial_ai_text, final_human_text
       FROM training_edit_pairs
       WHERE created_at >= ?
       ORDER BY id DESC
       LIMIT 5000`,
    )
      .bind(cutoffIso)
      .all<Record<string, unknown>>();

    const lines = (rows.results ?? []).map((row) =>
      JSON.stringify({
        createdAt: row.created_at,
        appVersion: row.app_version,
        sourceLang: row.source_lang,
        targetLang: row.target_lang,
        style: row.style,
        addressForm: row.address_form,
        sourceText: row.source_text,
        initialAiText: row.initial_ai_text,
        finalHumanText: row.final_human_text,
      }),
    );

    return new Response(lines.length > 0 ? `${lines.join("\n")}\n` : "", {
      status: 200,
      headers: {
        "Content-Type": "application/x-ndjson; charset=utf-8",
      },
    });
  }

  const [
    dailyTotalsRes,
    dauByOsRes,
    performanceRes,
    editTrendsRes,
    promotedNgramsRes,
    trainingSummaryRes,
    trainingTotalRes,
  ] = await Promise.all([
    env.DB.prepare(
      `SELECT utc_date, COALESCE(SUM(check_count), 0) AS total_checks
       FROM daily_update_checks
       WHERE utc_date >= ?
       GROUP BY utc_date
       ORDER BY utc_date DESC`,
    )
      .bind(cutoffDate)
      .all<Record<string, unknown>>(),
    env.DB.prepare(
      `SELECT utc_date, target, arch, current_version, check_count
       FROM daily_update_checks
       WHERE utc_date >= ?
       ORDER BY utc_date DESC, check_count DESC
       LIMIT 200`,
    )
      .bind(cutoffDate)
      .all<Record<string, unknown>>(),
    env.DB.prepare(
      `SELECT gpu_tier, whisper_backend, llm_provider, llm_model_id,
              COUNT(*) AS job_count,
              ROUND(AVG(asr_rtf), 4) AS avg_asr_rtf,
              ROUND(AVG(llm_rtf), 4) AS avg_llm_rtf,
              COALESCE(SUM(oom_cpu_fallback), 0) AS oom_fallbacks,
              COALESCE(SUM(batch_subdivision_retries), 0) AS batch_retries
       FROM job_and_edit_metrics
       WHERE event_kind = 'job' AND created_at >= ?
       GROUP BY gpu_tier, whisper_backend, llm_provider, llm_model_id
       ORDER BY job_count DESC
       LIMIT 100`,
    )
      .bind(cutoffIso)
      .all<Record<string, unknown>>(),
    env.DB.prepare(
      `SELECT app_version, target_lang, translation_style,
              COUNT(*) AS export_count,
              ROUND(AVG(edit_rate_pct), 2) AS avg_edit_rate_pct,
              ROUND(AVG(avg_cps_before), 2) AS avg_cps_before,
              ROUND(AVG(avg_cps_after), 2) AS avg_cps_after,
              COALESCE(SUM(enclitic_reordered_count), 0) AS enclitic_reordered,
              COALESCE(SUM(redundant_possessive_removed_count), 0) AS redundant_possessive_removed,
              COALESCE(SUM(passive_calque_removed_count), 0) AS passive_calque_removed,
              COALESCE(SUM(conjunction_comma_fixed_count), 0) AS conjunction_comma_fixed,
              COALESCE(SUM(missed_filler_deleted_count), 0) AS missed_filler_deleted,
              COALESCE(SUM(inflection_suffix_fix_count), 0) AS inflection_suffix_fix,
              COALESCE(SUM(boundary_orphan_shifted_count), 0) AS boundary_orphan_shifted,
              COALESCE(SUM(split_count), 0) AS split_count,
              COALESCE(SUM(merge_count), 0) AS merge_count,
              ROUND(AVG(avg_start_delta_ms), 1) AS avg_start_delta_ms,
              ROUND(AVG(avg_end_delta_ms), 1) AS avg_end_delta_ms
       FROM job_and_edit_metrics
       WHERE event_kind = 'export' AND created_at >= ?
       GROUP BY app_version, target_lang, translation_style
       ORDER BY app_version DESC, export_count DESC
       LIMIT 100`,
    )
      .bind(cutoffIso)
      .all<Record<string, unknown>>(),
    // Strict K-anonymity + Sybil visibility gate: ONLY cohort_count >= 5 AND subnet_count >= 3.
    env.DB.prepare(
      `SELECT lang_pair, old_span, new_span, cohort_count, subnet_count, updated_at
       FROM ngram_candidates
       WHERE cohort_count >= ? AND subnet_count >= ?
       ORDER BY cohort_count DESC, subnet_count DESC, updated_at DESC
       LIMIT 200`,
    )
      .bind(K_ANONYMITY_MIN_COHORTS, K_ANONYMITY_MIN_SUBNETS)
      .all<Record<string, unknown>>(),
    env.DB.prepare(
      `SELECT source_lang, target_lang, style, address_form, COUNT(*) AS pair_count
       FROM training_edit_pairs
       WHERE created_at >= ?
       GROUP BY source_lang, target_lang, style, address_form
       ORDER BY pair_count DESC
       LIMIT 50`,
    )
      .bind(cutoffIso)
      .all<Record<string, unknown>>(),
    env.DB.prepare(
      `SELECT COUNT(*) AS total_pairs
       FROM training_edit_pairs
       WHERE created_at >= ?`,
    )
      .bind(cutoffIso)
      .first<Record<string, unknown>>(),
  ]);

  return jsonResponse({
    ok: true,
    generatedAt: new Date(nowMs).toISOString(),
    windowDays: days,
    kAnonymityMinCohorts: K_ANONYMITY_MIN_COHORTS,
    kAnonymityMinSubnets: K_ANONYMITY_MIN_SUBNETS,
    dau: {
      dailyTotals: dailyTotalsRes.results ?? [],
      byOsAndVersion: dauByOsRes.results ?? [],
    },
    performance: performanceRes.results ?? [],
    editTrends: editTrendsRes.results ?? [],
    promotedNgrams: promotedNgramsRes.results ?? [],
    trainingPairsSummary: {
      totalCount: Number(trainingTotalRes?.total_pairs ?? 0),
      byLanguageAndStyle: trainingSummaryRes.results ?? [],
    },
  });
}

export async function runScheduledCleanup(
  env: Env,
  nowMs = Date.now(),
): Promise<{ ok: boolean; ngramCutoffIso: string; trainingCutoffIso: string }> {
  const ngramCutoffIso = new Date(nowMs - NGRAM_PURGE_DAYS * 86_400_000).toISOString();
  const trainingCutoffIso = new Date(nowMs - TRAINING_PAIRS_PURGE_DAYS * 86_400_000).toISOString();

  if (!env.DB) {
    console.error(JSON.stringify({ event: "scheduled_cleanup_skipped_no_db" }));
    return { ok: false, ngramCutoffIso, trainingCutoffIso };
  }

  await env.DB.batch([
    env.DB.prepare(
      `DELETE FROM ngram_cohorts
       WHERE pair_hash IN (
         SELECT pair_hash FROM ngram_candidates
         WHERE (cohort_count < ? OR subnet_count < ?) AND updated_at < ?
       )`,
    ).bind(K_ANONYMITY_MIN_COHORTS, K_ANONYMITY_MIN_SUBNETS, ngramCutoffIso),
    env.DB.prepare(
      `DELETE FROM ngram_candidates
       WHERE (cohort_count < ? OR subnet_count < ?) AND updated_at < ?`,
    ).bind(K_ANONYMITY_MIN_COHORTS, K_ANONYMITY_MIN_SUBNETS, ngramCutoffIso),
    env.DB.prepare(
      `DELETE FROM training_edit_pairs WHERE created_at < ?`,
    ).bind(trainingCutoffIso),
  ]);

  console.log(
    JSON.stringify({
      event: "scheduled_cleanup_completed",
      ngramCutoffIso,
      trainingCutoffIso,
    }),
  );

  return { ok: true, ngramCutoffIso, trainingCutoffIso };
}

function hasBrowserOrigin(request: Request): boolean {
  return request.headers.has("Origin");
}

function methodNotAllowed(allow: string): Response {
  return jsonResponse({ ok: false, error: "Method not allowed" }, 405, { Allow: allow });
}

export default {
  async fetch(
    request: Request,
    env: Env,
    ctx?: ExecutionContext,
  ): Promise<Response> {
    try {
      const url = new URL(request.url);
      const pathname =
        url.pathname.length > 1 && url.pathname.endsWith("/")
          ? url.pathname.slice(0, -1)
          : url.pathname;

      if (pathname === "/health") {
        if (request.method !== "GET") {
          return methodNotAllowed("GET");
        }
        return jsonResponse({ ok: true, service: SERVICE_NAME }, 200);
      }

      if (pathname.startsWith("/v1/") && hasBrowserOrigin(request)) {
        return jsonResponse(
          { ok: false, error: "Cross-origin browser requests are not allowed" },
          403,
        );
      }

      if (pathname === "/v1/report/bug" || pathname === "/v1/report/crash") {
        if (request.method !== "POST") {
          return methodNotAllowed("POST");
        }
        return pathname === "/v1/report/bug"
          ? await handleBugReport(request, env)
          : await handleCrashReport(request, env);
      }

      if (pathname === "/v1/update/latest.json") {
        if (request.method !== "GET") {
          return methodNotAllowed("GET");
        }
        return await handleUpdateManifest(request, env, ctx);
      }

      if (pathname === "/v1/telemetry/job-metrics") {
        if (request.method !== "POST") {
          return methodNotAllowed("POST");
        }
        return await handleJobMetrics(request, env);
      }

      if (pathname === "/v1/telemetry/export") {
        if (request.method !== "POST") {
          return methodNotAllowed("POST");
        }
        return await handleExportTelemetry(request, env);
      }

      if (pathname === "/v1/telemetry/training-pairs") {
        if (request.method !== "POST") {
          return methodNotAllowed("POST");
        }
        return await handleTrainingPairs(request, env);
      }

      if (pathname === "/v1/admin/digest") {
        if (request.method !== "GET") {
          return methodNotAllowed("GET");
        }
        return await handleAdminDigest(request, env);
      }

      if (request.method === "OPTIONS" && hasBrowserOrigin(request)) {
        return jsonResponse(
          { ok: false, error: "Cross-origin browser requests are not allowed" },
          403,
        );
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

  async scheduled(
    _controller: ScheduledController,
    env: Env,
    _ctx?: ExecutionContext,
  ): Promise<void> {
    await runScheduledCleanup(env);
  },
} satisfies ExportedHandler<Env>;
