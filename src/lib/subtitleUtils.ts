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

export function parseTimestampSrt(value: string): number | null {
  const cleaned = value.trim().replace(".", ",");
  const match =
    /^(?:(\d{1,2}):)?(\d{1,2}):(\d{1,2})(?:,(\d{1,3}))?$/.exec(cleaned);
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

export function calculateCueMetrics(
  cue: Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText">,
  rules: SegmentRules,
): { cps: number; warnings: string[] } {
  const activeText = (cue.translatedText.trim() || cue.sourceText.trim()).replace(
    /\r\n/g,
    "\n",
  );
  const durationMs = Math.max(1, cue.endMs - cue.startMs);
  const durationSec = durationMs / 1000;
  const charCount = [...activeText.replace(/\n/g, "")].length;
  const cps = Number((charCount / durationSec).toFixed(1));

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

const LATIN_DIGRAPHS: Array<[string, string]> = [
  ["DŽ", "Џ"],
  ["Dž", "Џ"],
  ["dž", "џ"],
  ["LJ", "Љ"],
  ["Lj", "Љ"],
  ["lj", "љ"],
  ["NJ", "Њ"],
  ["Nj", "Њ"],
  ["nj", "њ"],
];

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

const CYRILLIC_TO_LATIN: Record<string, string> = {
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
  Љ: "Lj",
  М: "M",
  Н: "N",
  Њ: "Nj",
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
  Џ: "Dž",
  Ш: "Š",
  а: "a",
  б: "b",
  в: "v",
  г: "g",
  д: "d",
  ђ: "đ",
  е: "e",
  ж: "ž",
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

export function convertSerbianScriptText(
  text: string,
  targetScript: SerbianScript,
): string {
  if (targetScript === "cyrillic") {
    let out = text;
    for (const [lat, cyr] of LATIN_DIGRAPHS) {
      out = out.split(lat).join(cyr);
    }
    return [...out]
      .map((ch) => LATIN_TO_CYRILLIC_SINGLE[ch] ?? ch)
      .join("");
  }

  return [...text]
    .map((ch) => CYRILLIC_TO_LATIN[ch] ?? ch)
    .join("");
}

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
