import type {
  SegmentPreset,
  SegmentRules,
  SerbianScript,
  SubtitleCue,
} from "../types/models";

export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

export function formatTimestampSrt(ms: number): string {
  const clamped = Math.max(0, Math.round(ms));
  const hours = Math.floor(clamped / 3_600_000);
  const minutes = Math.floor((clamped % 3_600_000) / 60_000);
  const seconds = Math.floor((clamped % 60_000) / 1000);
  const millis = clamped % 1000;

  return `${String(hours).padStart(2, "0")}:${String(minutes).padStart(
    2,
    "0",
  )}:${String(seconds).padStart(2, "0")},${String(millis).padStart(3, "0")}`;
}

export function formatTimestampVtt(ms: number): string {
  return formatTimestampSrt(ms).replace(",", ".");
}

export function parseTimestampSrt(value: string): number | null {
  const cleaned = value.trim().replace(".", ",");
  const match =
    /^(?:(\d{1,3}):)?(\d{1,2}):(\d{1,2})(?:,(\d{1,3}))?$/.exec(cleaned);
  if (!match) {
    return null;
  }
  const hours = match[1] ? Number.parseInt(match[1], 10) : 0;
  const minutes = Number.parseInt(match[2], 10);
  const seconds = Number.parseInt(match[3], 10);
  const millisRaw = match[4] ? match[4].padEnd(3, "0") : "000";
  const millis = Number.parseInt(millisRaw, 10);

  if (
    Number.isNaN(hours) ||
    Number.isNaN(minutes) ||
    Number.isNaN(seconds) ||
    Number.isNaN(millis) ||
    minutes >= 60 ||
    seconds >= 60
  ) {
    return null;
  }

  return hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + millis;
}

export function formatDurationShort(ms: number): string {
  const totalSec = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  const idx = Math.min(
    units.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const val = bytes / Math.pow(1024, idx);
  return `${val >= 100 ? val.toFixed(0) : val >= 10 ? val.toFixed(1) : val.toFixed(2)} ${units[idx]}`;
}

export function getPresetSegmentRules(
  preset: SegmentPreset,
  current?: SegmentRules,
): SegmentRules {
  if (preset === "standard") {
    return {
      preset: "standard",
      maxCharsPerLine: 42,
      maxLines: 2,
      minDurationMs: 1000,
      maxDurationMs: 7000,
      maxCps: 17,
    };
  }
  if (preset === "reels") {
    return {
      preset: "reels",
      maxCharsPerLine: 20,
      maxLines: 1,
      minDurationMs: 600,
      maxDurationMs: 3500,
      maxCps: 18,
    };
  }
  return {
    preset: "custom",
    maxCharsPerLine: current?.maxCharsPerLine ?? 42,
    maxLines: current?.maxLines ?? 2,
    minDurationMs: current?.minDurationMs ?? 1000,
    maxDurationMs: current?.maxDurationMs ?? 7000,
    maxCps: current?.maxCps ?? 17,
  };
}

/**
 * Calculates Characters Per Second (CPS) rounded to 1 decimal place.
 * Supports both `calculateCps(text, durationMs)` and `calculateCps(text, startMs, endMs)`.
 */
export function calculateCps(
  text: string,
  startOrDurationMs: number,
  endMs?: number,
): number {
  const cleaned = text.replace(/\r\n/g, "\n").trim().replace(/\n/g, "");
  const charCount = [...cleaned].length;
  if (charCount === 0) {
    return 0;
  }
  const durationMs =
    typeof endMs === "number"
      ? Math.max(1, endMs - startOrDurationMs)
      : Math.max(1, startOrDurationMs);
  const durationSec = durationMs / 1000;
  return Number((charCount / durationSec).toFixed(1));
}

/**
 * Computes warning identifiers (`high_cps`, `critical_cps`, `line_too_long`,
 * `too_many_lines`, `short_duration`, `long_duration`) for a cue under `rules`.
 */
export function computeCueWarnings(
  cue: Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText"> & {
    cps?: number;
  },
  rules: SegmentRules,
): string[] {
  const activeText = (
    cue.translatedText.trim() || cue.sourceText.trim()
  ).replace(/\r\n/g, "\n");
  const durationMs = Math.max(1, cue.endMs - cue.startMs);
  const charCount = [...activeText.replace(/\n/g, "")].length;
  const cps =
    typeof cue.cps === "number" && cue.cps > 0
      ? cue.cps
      : calculateCps(activeText, cue.startMs, cue.endMs);

  const lines = activeText.length > 0 ? activeText.split("\n") : [];
  const warnings: string[] = [];

  if (cps > rules.maxCps + 4) {
    warnings.push("critical_cps");
  } else if (cps > rules.maxCps) {
    warnings.push("high_cps");
  }

  if (lines.some((line) => [...line].length > rules.maxCharsPerLine)) {
    warnings.push("line_too_long");
  }

  if (lines.length > rules.maxLines) {
    warnings.push("too_many_lines");
  }

  if (durationMs < rules.minDurationMs && charCount > 0) {
    warnings.push("short_duration");
  } else if (durationMs > rules.maxDurationMs) {
    warnings.push("long_duration");
  }

  return warnings;
}

export function calculateCueMetrics(
  cue: Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText">,
  rules: SegmentRules,
): { cps: number; warnings: string[] } {
  const activeText = (
    cue.translatedText.trim() || cue.sourceText.trim()
  ).replace(/\r\n/g, "\n");
  const cps = calculateCps(activeText, cue.startMs, cue.endMs);
  const warnings = computeCueWarnings({ ...cue, cps }, rules);
  return { cps, warnings };
}

export function recalculateCue(
  cue: SubtitleCue,
  rules: SegmentRules,
): SubtitleCue {
  const { cps, warnings } = calculateCueMetrics(cue, rules);
  return {
    ...cue,
    cps,
    warnings,
  };
}

/**
 * Balances text into at most `maxLines` lines (preferring 1 line if it fits within
 * `maxCharsPerLine`, or splitting into 2 balanced lines near the visual midpoint).
 */
export function wrapBalancedLines(
  text: string,
  maxCharsPerLine = 42,
  maxLines = 2,
): string {
  const words = text
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (words.length === 0) {
    return "";
  }

  const singleLine = words.join(" ");
  const maxChars = Math.max(1, maxCharsPerLine);

  if (maxLines <= 1 || [...singleLine].length <= maxChars || words.length === 1) {
    return singleLine;
  }

  let bestSplit = 1;
  let bestScore = Number.POSITIVE_INFINITY;

  for (let k = 1; k < words.length; k++) {
    const line1 = words.slice(0, k).join(" ");
    const line2 = words.slice(k).join(" ");
    const len1 = [...line1].length;
    const len2 = [...line2].length;

    const overflow1 = Math.max(0, len1 - maxChars);
    const overflow2 = Math.max(0, len2 - maxChars);
    const overflowPenalty = (overflow1 + overflow2) * 1000;

    const diff = Math.abs(len1 - len2);
    const prevWord = words[k - 1].replace(/["'”’)\]]+$/, "");
    const punctBonus = /[,;:—–.!?…]$/.test(prevWord) ? 6 : 0;

    const score = overflowPenalty + diff - punctBonus;
    if (score < bestScore) {
      bestScore = score;
      bestSplit = k;
    }
  }

  const line1 = words.slice(0, bestSplit).join(" ");
  const line2 = words.slice(bestSplit).join(" ");
  return `${line1}\n${line2}`;
}

// ---------------------------------------------------------------------------
// Deterministic Serbian Cyrillic <-> Latin Script Conversion
// ---------------------------------------------------------------------------

const DIGRAPH_EXCEPTION_PREFIXES = [
  "injekc",
  "injekt",
  "konjug",
  "konjunk",
  "adjekt",
  "nadživ",
  "nadziv",
  "odživ",
  "odziv",
  "podžanr",
  "podžup",
  "predživot",
  "predzivot",
  "tanjug",
];

const PROTECTED_SPAN_REGEX =
  /(?:https?:\/\/[^\s<>]+|www\.[^\s<>]+|[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}|<[^>]+>|\{\\[^}]+\}|\[[^\]]+\])/gi;

const LATIN_TO_CYRILLIC_SINGLE: Record<string, string> = {
  A: "А",
  B: "Б",
  V: "В",
  G: "Г",
  D: "Д",
  Đ: "Ђ",
  E: "Е",
  Ž: "Ж",
  Z: "З",
  I: "И",
  J: "Ј",
  K: "К",
  L: "Л",
  M: "М",
  N: "Н",
  O: "О",
  P: "П",
  R: "Р",
  S: "С",
  T: "Т",
  Ć: "Ћ",
  U: "У",
  F: "Ф",
  H: "Х",
  C: "Ц",
  Č: "Ч",
  Š: "Ш",
  a: "а",
  b: "б",
  v: "в",
  g: "г",
  d: "д",
  đ: "ђ",
  e: "е",
  ž: "ж",
  z: "з",
  i: "и",
  j: "ј",
  k: "к",
  l: "л",
  m: "м",
  n: "н",
  o: "о",
  p: "п",
  r: "р",
  s: "с",
  t: "т",
  ć: "ћ",
  u: "у",
  f: "ф",
  h: "х",
  c: "ц",
  č: "ч",
  š: "ш",
};

const CYRILLIC_TO_LATIN_SINGLE: Record<string, string> = {
  А: "A",
  Б: "B",
  В: "V",
  Г: "G",
  Д: "D",
  Ђ: "Đ",
  Е: "E",
  Ж: "Ž",
  З: "Z",
  И: "I",
  Ј: "J",
  К: "K",
  Л: "L",
  М: "M",
  Н: "N",
  О: "O",
  П: "P",
  Р: "R",
  С: "S",
  Т: "T",
  Ћ: "Ć",
  У: "U",
  Ф: "F",
  Х: "H",
  Ц: "C",
  Ч: "Č",
  Ш: "Š",
  а: "a",
  б: "b",
  в: "v",
  г: "g",
  д: "d",
  ђ: "đ",
  е: "e",
  ж: "ж" === "ж" ? "ž" : "ž",
  з: "z",
  и: "i",
  ј: "j",
  к: "k",
  л: "l",
  љ: "lj",
  м: "m",
  н: "n",
  њ: "nj",
  о: "o",
  п: "p",
  р: "r",
  с: "s",
  т: "t",
  ћ: "ć",
  у: "u",
  ф: "f",
  х: "h",
  ц: "c",
  ч: "č",
  џ: "dž",
  ш: "š",
};

function isAlphabeticChar(ch: string | undefined): boolean {
  return Boolean(ch && /\p{L}/u.test(ch));
}

function isUppercaseChar(ch: string | undefined): boolean {
  return Boolean(
    ch && isAlphabeticChar(ch) && ch === ch.toUpperCase() && ch !== ch.toLowerCase(),
  );
}

/**
 * Converts Serbian Cyrillic text to Serbian Latin with smart digraph casing
 * (`ЉУБАВ` -> `LJUBAV`, `Љубав` -> `Ljubav`, `љубав` -> `ljubav`).
 */
export function cyrillicToLatin(text: string): string {
  const chars = [...text];
  let out = "";

  for (let idx = 0; idx < chars.length; idx++) {
    const ch = chars[idx];
    const isAllCapsContext = () => {
      const next = chars[idx + 1];
      const prev = idx > 0 ? chars[idx - 1] : undefined;
      const nextUpper = isUppercaseChar(next);
      const nextAlpha = isAlphabeticChar(next);
      const prevUpper = isUppercaseChar(prev);
      return nextUpper || (!nextAlpha && prevUpper);
    };

    if (ch === "Љ") {
      out += isAllCapsContext() ? "LJ" : "Lj";
    } else if (ch === "Њ") {
      out += isAllCapsContext() ? "NJ" : "Nj";
    } else if (ch === "Џ") {
      out += isAllCapsContext() ? "DŽ" : "Dž";
    } else {
      out += CYRILLIC_TO_LATIN_SINGLE[ch] ?? ch;
    }
  }

  return out;
}

function isDigraphExceptionWord(wordLower: string): boolean {
  return DIGRAPH_EXCEPTION_PREFIXES.some((prefix) =>
    wordLower.startsWith(prefix),
  );
}

function convertWordToCyrillic(word: string): string {
  const lower = word.toLowerCase();
  const skipDigraphMerge = isDigraphExceptionWord(lower);
  const chars = [...word];
  let out = "";
  let i = 0;

  while (i < chars.length) {
    const ch = chars[i];
    const next = chars[i + 1];

    if (next !== undefined) {
      // Lj / LJ / lj is never a morphological prefix split in exception words
      if ((ch === "L" || ch === "l") && (next === "J" || next === "j")) {
        out += isUppercaseChar(ch) ? "Љ" : "љ";
        i += 2;
        continue;
      }

      if (!skipDigraphMerge) {
        if ((ch === "N" || ch === "n") && (next === "J" || next === "j")) {
          out += isUppercaseChar(ch) ? "Њ" : "њ";
          i += 2;
          continue;
        }
        if ((ch === "D" || ch === "d") && (next === "Ž" || next === "ž")) {
          out += isUppercaseChar(ch) ? "Џ" : "џ";
          i += 2;
          continue;
        }
      }
    }

    out += LATIN_TO_CYRILLIC_SINGLE[ch] ?? ch;
    i += 1;
  }

  return out;
}

function convertPlainSegmentToCyrillic(segment: string): string {
  return segment.replace(/\p{L}+/gu, (word) => convertWordToCyrillic(word));
}

/**
 * Converts Serbian Latin text to Serbian Cyrillic, preserving protected spans
 * (URLs, emails, HTML/ASS tags, `[...]` markers) and respecting prefix-boundary
 * exceptions (`injekcija` -> `инјекција`, `konjugacija` -> `конјугација`, `nadživeti` -> `надживети`).
 */
export function latinToCyrillic(text: string): string {
  let out = "";
  let lastEnd = 0;
  const regex = new RegExp(PROTECTED_SPAN_REGEX.source, "gi");
  let match: RegExpExecArray | null;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastEnd) {
      out += convertPlainSegmentToCyrillic(text.slice(lastEnd, match.index));
    }
    out += match[0];
    lastEnd = match.index + match[0].length;
  }

  if (lastEnd < text.length) {
    out += convertPlainSegmentToCyrillic(text.slice(lastEnd));
  }

  return out;
}

export function convertSerbianScriptText(
  text: string,
  targetScript: SerbianScript,
): string {
  return targetScript === "cyrillic"
    ? latinToCyrillic(text)
    : cyrillicToLatin(text);
}

// ---------------------------------------------------------------------------
// Glossary Parsing
// ---------------------------------------------------------------------------

export function parseBulkGlossary(raw: string): Record<string, string> {
  const result: Record<string, string> = {};
  const chunks = raw
    .split(/\r?\n|,/)
    .map((item) => item.trim())
    .filter(Boolean);

  for (const chunk of chunks) {
    const arrowIndex = chunk.indexOf("->");
    const eqIndex = chunk.indexOf("=");
    if (arrowIndex !== -1) {
      const k = chunk.slice(0, arrowIndex).trim();
      const v = chunk.slice(arrowIndex + 2).trim();
      if (k) {
        result[k] = v || k;
      }
    } else if (eqIndex !== -1) {
      const k = chunk.slice(0, eqIndex).trim();
      const v = chunk.slice(eqIndex + 1).trim();
      if (k) {
        result[k] = v || k;
      }
    } else {
      result[chunk] = chunk;
    }
  }
  return result;
}

// ---------------------------------------------------------------------------
// .SRT / .VTT Subtitle File Parser & Exporters
// ---------------------------------------------------------------------------

function parseArrowTimestampLine(
  line: string,
): { startMs: number; endMs: number } | null {
  const arrowIdx = line.indexOf("-->");
  if (arrowIdx === -1) {
    return null;
  }
  const rawLeft = line.slice(0, arrowIdx).trim();
  const rawRightTokens = line
    .slice(arrowIdx + 3)
    .trim()
    .split(/\s+/);
  const rawRight = rawRightTokens[0] ?? "";
  if (!rawLeft || !rawRight) {
    return null;
  }

  const startMs = parseTimestampSrt(rawLeft);
  const endMs = parseTimestampSrt(rawRight);
  if (startMs === null || endMs === null) {
    return null;
  }

  return {
    startMs,
    endMs: Math.max(startMs, endMs),
  };
}

/**
 * Pure parser for `.srt` and `.vtt` subtitle file contents.
 * Handles UTF-8 BOM, CRLF/LF line endings, comma and dot millisecond separators,
 * VTT headers/metadata/cue settings, and multi-line cue text.
 */
export function parseSubtitleFileText(
  content: string,
  rules: SegmentRules = getPresetSegmentRules("standard"),
): SubtitleCue[] {
  const normalized = content
    .replace(/^\uFEFF/, "")
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .trim();

  if (!normalized) {
    return [];
  }

  const blocks = normalized.split(/\n\s*\n/);
  const cues: SubtitleCue[] = [];

  for (const rawBlock of blocks) {
    const lines = rawBlock
      .split("\n")
      .map((l) => l.trimEnd());

    // Skip empty blocks, WebVTT header/metadata blocks, or NOTE/STYLE/REGION blocks without timestamps
    const firstNonEmpty = lines.find((l) => l.trim().length > 0)?.trim() ?? "";
    if (!firstNonEmpty) {
      continue;
    }

    let arrowLineIdx = -1;
    let parsedTimes: { startMs: number; endMs: number } | null = null;

    for (let i = 0; i < lines.length; i++) {
      const candidate = parseArrowTimestampLine(lines[i]);
      if (candidate !== null) {
        arrowLineIdx = i;
        parsedTimes = candidate;
        break;
      }
    }

    if (arrowLineIdx === -1 || parsedTimes === null) {
      continue;
    }

    if (
      firstNonEmpty.startsWith("NOTE") &&
      arrowLineIdx > 0 &&
      ! /^\d+$/.test(firstNonEmpty)
    ) {
      continue;
    }

    const textLines = lines
      .slice(arrowLineIdx + 1)
      .map((l) => l.trim())
      .filter((l, idx, arr) => l.length > 0 || (idx > 0 && idx < arr.length - 1));
    const cueText = textLines.join("\n").trim();
    if (!cueText) {
      continue;
    }

    const baseCue = {
      id: cues.length + 1,
      startMs: parsedTimes.startMs,
      endMs: parsedTimes.endMs,
      sourceText: cueText,
      translatedText: "",
    };
    const { cps, warnings } = calculateCueMetrics(baseCue, rules);
    cues.push({
      ...baseCue,
      cps,
      warnings,
    });
  }

  return cues;
}

export interface FormatDocumentOptions {
  useSource?: boolean;
  utf8Bom?: boolean;
}

function resolveFormatOptions(
  optionsOrUseSource?: FormatDocumentOptions | boolean,
  utf8BomArg?: boolean,
): { useSource: boolean; utf8Bom: boolean } {
  if (typeof optionsOrUseSource === "boolean") {
    return {
      useSource: optionsOrUseSource,
      utf8Bom: Boolean(utf8BomArg),
    };
  }
  return {
    useSource: Boolean(optionsOrUseSource?.useSource),
    utf8Bom: Boolean(optionsOrUseSource?.utf8Bom ?? utf8BomArg),
  };
}

function selectCueExportText(cue: SubtitleCue, useSource: boolean): string {
  if (useSource || !cue.translatedText.trim()) {
    return cue.sourceText.trim();
  }
  return cue.translatedText.trim();
}

export function formatSrtDocument(
  cues: SubtitleCue[],
  optionsOrUseSource?: FormatDocumentOptions | boolean,
  utf8BomArg?: boolean,
): string {
  const { useSource, utf8Bom } = resolveFormatOptions(
    optionsOrUseSource,
    utf8BomArg,
  );
  const prefix = utf8Bom ? "\uFEFF" : "";
  const blocks = cues.map((cue, idx) => {
    const seq = idx + 1;
    const start = formatTimestampSrt(cue.startMs);
    const end = formatTimestampSrt(Math.max(cue.startMs, cue.endMs));
    const text = selectCueExportText(cue, useSource);
    return `${seq}\n${start} --> ${end}\n${text}\n`;
  });
  return prefix + blocks.join("\n");
}

export function formatVttDocument(
  cues: SubtitleCue[],
  optionsOrUseSource?: FormatDocumentOptions | boolean,
  utf8BomArg?: boolean,
): string {
  const { useSource, utf8Bom } = resolveFormatOptions(
    optionsOrUseSource,
    utf8BomArg,
  );
  const prefix = (utf8Bom ? "\uFEFF" : "") + "WEBVTT\n\n";
  const blocks = cues.map((cue, idx) => {
    const seq = idx + 1;
    const start = formatTimestampVtt(cue.startMs);
    const end = formatTimestampVtt(Math.max(cue.startMs, cue.endMs));
    const text = selectCueExportText(cue, useSource);
    return `${seq}\n${start} --> ${end}\n${text}\n`;
  });
  return prefix + blocks.join("\n");
}

export function formatTxtDocument(
  cues: SubtitleCue[],
  optionsOrUseSource?: FormatDocumentOptions | boolean,
  utf8BomArg?: boolean,
): string {
  const { useSource, utf8Bom } = resolveFormatOptions(
    optionsOrUseSource,
    utf8BomArg,
  );
  const prefix = utf8Bom ? "\uFEFF" : "";
  const lines: string[] = [];
  for (const cue of cues) {
    const raw = selectCueExportText(cue, useSource);
    const singleLine = raw.split(/\s+/).filter(Boolean).join(" ");
    if (singleLine) {
      lines.push(singleLine);
    }
  }
  return prefix + (lines.length > 0 ? `${lines.join("\n")}\n` : "");
}

/** Returns whether an incoming project sync should switch the active editor selection. */
export function shouldActivateSyncedProject(
  currentActiveJobId: string | null,
  incomingJobId: string,
  forceActivate = false,
): boolean {
  return (
    Boolean(forceActivate) ||
    currentActiveJobId === null ||
    currentActiveJobId === "demo-session-01" ||
    currentActiveJobId === incomingJobId
  );
}

/** Resolves initial activeJobId when batch starts without overriding an already selected job. */
export function resolveInitialBatchActiveJobId(
  currentActiveJobId: string | null,
  firstRunnableJobId: string | null,
): string | null {
  if (!currentActiveJobId || currentActiveJobId === "demo-session-01") {
    return firstRunnableJobId;
  }
  return currentActiveJobId;
}

/** Compares two semantic version strings (with optional leading "v"). Returns 1 if a > b, -1 if a < b, 0 if equal. */
export function compareSemver(a: string, b: string): number {
  const parse = (raw: string): [number, number, number] => {
    const cleaned = raw.trim().replace(/^v/i, "").split("-")[0] ?? "";
    const parts = cleaned.split(".").map((n) => {
      const parsed = Number.parseInt(n, 10);
      return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
    });
    return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
  };
  const [aMaj, aMin, aPatch] = parse(a);
  const [bMaj, bMin, bPatch] = parse(b);
  if (aMaj !== bMaj) return aMaj > bMaj ? 1 : -1;
  if (aMin !== bMin) return aMin > bMin ? 1 : -1;
  if (aPatch !== bPatch) return aPatch > bPatch ? 1 : -1;
  return 0;
}
