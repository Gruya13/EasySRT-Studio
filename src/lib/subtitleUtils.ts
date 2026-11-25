import type {
  SegmentPreset,
  SegmentRules,
  SerbianScript,
  SubtitleCue,
  TranslationStyle,
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
  const splitOnSpeakerChange = current?.splitOnSpeakerChange ?? true;
  if (preset === "standard") {
    return {
      preset: "standard",
      maxCharsPerLine: 42,
      maxLines: 2,
      minDurationMs: 1000,
      maxDurationMs: 7000,
      maxCps: 17,
      splitOnSpeakerChange,
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
      splitOnSpeakerChange,
    };
  }
  return {
    preset: "custom",
    maxCharsPerLine: current?.maxCharsPerLine ?? 42,
    maxLines: current?.maxLines ?? 2,
    minDurationMs: current?.minDurationMs ?? 1000,
    maxDurationMs: current?.maxDurationMs ?? 7000,
    maxCps: current?.maxCps ?? 17,
    splitOnSpeakerChange,
  };
}

export function normalizeSegmentRules(
  rules?: Partial<SegmentRules> | null,
): SegmentRules {
  const base = getPresetSegmentRules(rules?.preset ?? "standard");
  return {
    ...base,
    ...rules,
    preset: rules?.preset ?? base.preset,
    splitOnSpeakerChange: rules?.splitOnSpeakerChange ?? true,
  };
}

export function normalizeTranslationStyle(
  style?: TranslationStyle | string | null,
): TranslationStyle {
  return style === "formal" ? "formal" : "natural";
}

/** Minimum breathing gap between consecutive subtitle cues (2 frames @ 25 fps). */
export const MIN_INTER_CUE_GAP_MS = 80;

const MIN_SPLIT_HALF_DURATION_MS = 160;

/**
 * Computes left-end / right-start timestamps when splitting a cue at `splitMs`,
 * leaving a `MIN_INTER_CUE_GAP_MS` (80 ms) breathing gap (`splitMs - 40` / `splitMs + 40`)
 * whenever cue durations permit while keeping `>= 160 ms` per half.
 */
export function computeSplitBoundaries(
  startMs: number,
  endMs: number,
  splitMs: number,
): { leftEndMs: number; rightStartMs: number } {
  const safeStart = Math.max(0, Math.round(startMs));
  const safeEnd = Math.max(safeStart, Math.round(endMs));
  const total = safeEnd - safeStart;
  const halfGap = Math.floor(MIN_INTER_CUE_GAP_MS / 2);

  if (total >= MIN_SPLIT_HALF_DURATION_MS * 2 + MIN_INTER_CUE_GAP_MS) {
    const minSplit = safeStart + MIN_SPLIT_HALF_DURATION_MS + halfGap;
    const maxSplit = safeEnd - MIN_SPLIT_HALF_DURATION_MS - halfGap;
    const clampedSplit = Math.min(
      maxSplit,
      Math.max(minSplit, Math.round(splitMs)),
    );
    return {
      leftEndMs: clampedSplit - halfGap,
      rightStartMs: clampedSplit + halfGap,
    };
  }

  if (total >= MIN_SPLIT_HALF_DURATION_MS * 2) {
    return {
      leftEndMs: safeStart + MIN_SPLIT_HALF_DURATION_MS,
      rightStartMs: safeEnd - MIN_SPLIT_HALF_DURATION_MS,
    };
  }

  if (total <= 1) {
    return { leftEndMs: safeStart, rightStartMs: safeEnd };
  }

  const mid = Math.min(
    safeEnd - 1,
    Math.max(safeStart + 1, Math.round(splitMs)),
  );
  return { leftEndMs: mid, rightStartMs: mid };
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
  const cleaned = text
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .trim()
    .replace(/\n/g, "");
  const charCount = [...cleaned].length;
  if (charCount === 0) {
    return 0;
  }
  const durationMs =
    typeof endMs === "number"
      ? Math.max(0, endMs - startOrDurationMs)
      : Math.max(0, startOrDurationMs);
  if (durationMs === 0) {
    return 999.0;
  }
  const durationSec = durationMs / 1000;
  return Math.round((charCount / durationSec) * 10) / 10;
}

/**
 * Computes canonical warning identifiers (`critical_cps`, `high_cps`, `line_too_long`,
 * `too_many_lines`, `duration_too_short`, `duration_too_long`) for a cue under `rules`.
 */
export function computeCueWarnings(
  cue: Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText"> & {
    cps?: number;
  },
  rules: SegmentRules,
): string[] {
  const activeText = (cue.translatedText.trim() || cue.sourceText.trim())
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .trim();
  const durationMs = Math.max(0, cue.endMs - cue.startMs);
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
    warnings.push("duration_too_short");
  } else if (durationMs > rules.maxDurationMs) {
    warnings.push("duration_too_long");
  }

  return warnings;
}

export function calculateCueMetrics(
  cue: Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText">,
  rules: SegmentRules,
): { cps: number; warnings: string[] } {
  const activeText = (cue.translatedText.trim() || cue.sourceText.trim())
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .trim();
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

// ---------------------------------------------------------------------------
// Linguistically-Aware Multi-Line Wrapping (1..=4 lines, 1:1 with segmenter.rs)
// ---------------------------------------------------------------------------

const SENTENCE_ABBREVIATIONS = new Set([
  "dr.",
  "mr.",
  "sv.",
  "br.",
  "st.",
  "prof.",
  "tj.",
  "npr.",
  "др.",
  "мр.",
  "св.",
  "бр.",
  "проф.",
  "тј.",
  "нпр.",
]);

const WRAP_NEGATIONS = new Set([
  "ne",
  "ni",
  "niti",
  "nije",
  "nisam",
  "nisi",
  "nismo",
  "niste",
  "nisu",
  "neću",
  "necu",
  "nećeš",
  "neces",
  "neće",
  "nece",
  "nećemo",
  "necemo",
  "nećete",
  "necete",
  "nemoj",
  "nemojte",
  "nemam",
  "nema",
  "nemaju",
  "не",
  "ни",
  "нити",
  "није",
  "нисам",
  "ниси",
  "нисмо",
  "нисте",
  "нису",
  "нећу",
  "нећеш",
  "неће",
  "нећемо",
  "нећете",
  "немој",
  "немојте",
  "немам",
  "нема",
  "немају",
  "not",
  "no",
  "never",
  "don't",
  "doesn't",
  "didn't",
  "won't",
  "wouldn't",
  "can't",
  "cannot",
  "couldn't",
  "shouldn't",
  "isn't",
  "aren't",
  "wasn't",
  "weren't",
  "haven't",
  "hasn't",
  "hadn't",
]);

const WRAP_PREPOSITIONS_AND_ARTICLES = new Set([
  "u",
  "na",
  "za",
  "sa",
  "s",
  "od",
  "do",
  "iz",
  "o",
  "po",
  "pri",
  "ka",
  "k",
  "uz",
  "niz",
  "nad",
  "pod",
  "pred",
  "među",
  "medju",
  "kroz",
  "kod",
  "bez",
  "osim",
  "prema",
  "protiv",
  "oko",
  "posle",
  "pre",
  "у",
  "на",
  "за",
  "са",
  "с",
  "од",
  "до",
  "из",
  "о",
  "по",
  "при",
  "ка",
  "к",
  "уз",
  "низ",
  "над",
  "под",
  "пред",
  "међу",
  "кроз",
  "код",
  "без",
  "осим",
  "према",
  "против",
  "око",
  "после",
  "пре",
  "a",
  "an",
  "the",
  "in",
  "on",
  "at",
  "to",
  "of",
  "for",
  "with",
  "from",
  "by",
  "into",
  "onto",
  "upon",
  "about",
  "against",
  "between",
  "through",
  "during",
  "without",
  "within",
  "under",
  "over",
]);

const WRAP_AUXILIARIES_AND_ENCLITICS = new Set([
  "sam",
  "si",
  "je",
  "smo",
  "ste",
  "su",
  "ću",
  "cu",
  "ćeš",
  "ces",
  "će",
  "ce",
  "ćemo",
  "cemo",
  "ćete",
  "cete",
  "bih",
  "bi",
  "bismo",
  "biste",
  "se",
  "li",
  "bio",
  "bila",
  "bilo",
  "bili",
  "bile",
  "сам",
  "си",
  "је",
  "смо",
  "сте",
  "су",
  "ћу",
  "ћеш",
  "ће",
  "ћемо",
  "ћете",
  "бих",
  "би",
  "бисмо",
  "бисте",
  "се",
  "ли",
  "био",
  "била",
  "било",
  "били",
  "биле",
  "is",
  "am",
  "are",
  "was",
  "were",
  "be",
  "been",
  "being",
  "has",
  "have",
  "had",
  "do",
  "does",
  "did",
  "will",
  "would",
  "shall",
  "should",
  "can",
  "could",
  "may",
  "might",
  "must",
]);

const WRAP_CONJUNCTIONS = new Set([
  "i",
  "a",
  "ali",
  "ili",
  "da",
  "pa",
  "te",
  "jer",
  "što",
  "sto",
  "ako",
  "dok",
  "kad",
  "kada",
  "nego",
  "već",
  "vec",
  "kako",
  "gde",
  "gdje",
  "koji",
  "koja",
  "koje",
  "čiji",
  "ciji",
  "и",
  "а",
  "али",
  "или",
  "да",
  "па",
  "те",
  "јер",
  "што",
  "ако",
  "док",
  "кад",
  "када",
  "него",
  "већ",
  "како",
  "где",
  "гдје",
  "који",
  "која",
  "које",
  "чији",
  "and",
  "or",
  "but",
  "so",
  "yet",
  "nor",
  "that",
  "which",
  "who",
  "whom",
  "whose",
  "where",
  "when",
  "while",
  "because",
  "although",
  "since",
  "until",
  "unless",
  "if",
  "whether",
]);

const WRAP_PRONOUNS = new Set([
  "ja",
  "ti",
  "on",
  "ona",
  "ono",
  "mi",
  "vi",
  "oni",
  "one",
  "moj",
  "moja",
  "moje",
  "tvoj",
  "tvoja",
  "tvoje",
  "njegov",
  "njen",
  "naš",
  "nas",
  "vaš",
  "vas",
  "njihov",
  "svoj",
  "svoja",
  "svoje",
  "ovaj",
  "ova",
  "ovo",
  "taj",
  "ta",
  "to",
  "onaj",
  "mu",
  "joj",
  "ga",
  "ih",
  "nam",
  "vam",
  "ја",
  "ти",
  "он",
  "она",
  "оно",
  "ми",
  "ви",
  "они",
  "оне",
  "мој",
  "моја",
  "моје",
  "твој",
  "твоја",
  "твоје",
  "његов",
  "њен",
  "наш",
  "ваш",
  "њихов",
  "свој",
  "своја",
  "своје",
  "овај",
  "ова",
  "ово",
  "тај",
  "та",
  "то",
  "онај",
  "му",
  "јој",
  "га",
  "их",
  "нам",
  "вам",
  "i",
  "you",
  "he",
  "she",
  "it",
  "we",
  "they",
  "my",
  "your",
  "his",
  "her",
  "its",
  "our",
  "their",
  "this",
  "that",
  "these",
  "those",
]);

const SENTENCE_OPENING_NON_NAMES = new Set([
  "danas",
  "sada",
  "sad",
  "ovde",
  "ovdje",
  "tamo",
  "tada",
  "tad",
  "kada",
  "kad",
  "ali",
  "međutim",
  "medjutim",
  "ipak",
  "dakle",
  "zato",
  "potom",
  "zatim",
  "onda",
  "juče",
  "juce",
  "sutra",
  "uvek",
  "uvijek",
  "nikad",
  "nikada",
  "možda",
  "mozda",
  "naravno",
  "zapravo",
  "inače",
  "inace",
  "dok",
  "ako",
  "pošto",
  "posto",
  "iako",
  "čim",
  "cim",
  "već",
  "vec",
  "samo",
  "čak",
  "cak",
  "baš",
  "bas",
  "i",
  "a",
  "pa",
  "da",
  "ne",
  "ovaj",
  "ova",
  "ovo",
  "taj",
  "ta",
  "to",
  "onaj",
  "ona",
  "ono",
  "on",
  "mi",
  "vi",
  "oni",
  "one",
  "ja",
  "ti",
  "svi",
  "sve",
  "neko",
  "niko",
  "šta",
  "sta",
  "ko",
  "kako",
  "zašto",
  "zasto",
  "gde",
  "gdje",
  "koliko",
  "данас",
  "сада",
  "сад",
  "овде",
  "овдје",
  "тамо",
  "тада",
  "тад",
  "када",
  "кад",
  "али",
  "међутим",
  "ипак",
  "дакле",
  "зато",
  "потом",
  "затим",
  "онда",
  "јуче",
  "сутра",
  "увек",
  "увијек",
  "никад",
  "никада",
  "можда",
  "наравно",
  "заправо",
  "иначе",
  "док",
  "ако",
  "пошто",
  "иако",
  "чим",
  "већ",
  "само",
  "чак",
  "баш",
  "и",
  "а",
  "па",
  "да",
  "не",
  "овај",
  "ова",
  "ово",
  "тај",
  "та",
  "то",
  "онај",
  "она",
  "оно",
  "он",
  "ми",
  "ви",
  "они",
  "оне",
  "ја",
  "ти",
  "сви",
  "све",
  "неко",
  "нико",
  "шта",
  "ко",
  "како",
  "зашто",
  "где",
  "гдје",
  "колико",
  "when",
  "today",
  "tomorrow",
  "yesterday",
  "here",
  "there",
  "then",
  "now",
  "however",
  "therefore",
  "meanwhile",
  "otherwise",
  "suddenly",
  "actually",
  "maybe",
  "perhaps",
  "always",
  "never",
  "often",
  "sometimes",
  "already",
  "still",
  "just",
  "only",
  "even",
  "also",
  "but",
  "and",
  "or",
  "so",
  "yet",
  "because",
  "although",
  "though",
  "while",
  "since",
  "until",
  "unless",
  "if",
  "whether",
  "after",
  "before",
  "during",
  "why",
  "how",
  "what",
  "where",
  "who",
  "this",
  "that",
  "these",
  "those",
  "they",
  "we",
  "he",
  "she",
  "it",
  "you",
  "everyone",
  "someone",
  "nobody",
  "finally",
  "first",
  "next",
  "later",
]);

function endsWithSentencePunct(word: string): boolean {
  const trimmed = word.replace(/["'”’)\]]+$/u, "");
  if (!trimmed) {
    return false;
  }
  if (SENTENCE_ABBREVIATIONS.has(trimmed.toLowerCase())) {
    return false;
  }
  return /[.!?…]$/u.test(trimmed);
}

function endsWithClausePunct(word: string): boolean {
  const trimmed = word.replace(/["'”’)\]]+$/u, "");
  return /[,;:—–]$/u.test(trimmed);
}

function normalizeWrapToken(word: string): string {
  return word
    .replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "")
    .replace(/’/g, "'")
    .toLowerCase();
}

function startsWithUppercaseAlpha(word: string): boolean {
  const trimmed = word.replace(/^["'„“‘«(\[]+/u, "");
  const first = [...trimmed][0];
  return isUppercaseChar(first);
}

function isProperNamePair(
  words: string[],
  k: number,
  prevClean: string,
): boolean {
  if (k <= 0 || k >= words.length) {
    return false;
  }
  if (
    !startsWithUppercaseAlpha(words[k - 1]) ||
    !startsWithUppercaseAlpha(words[k])
  ) {
    return false;
  }
  const isSentenceStart = k === 1 || endsWithSentencePunct(words[k - 2]);
  return !(isSentenceStart && SENTENCE_OPENING_NON_NAMES.has(prevClean));
}

function linguisticSplitPenalty(words: string[], k: number): number {
  if (k <= 0 || k >= words.length) {
    return 0;
  }
  const prev = words[k - 1];
  if (endsWithSentencePunct(prev) || endsWithClausePunct(prev)) {
    return 0;
  }
  const clean = normalizeWrapToken(prev);
  let penalty = 0;
  if (WRAP_NEGATIONS.has(clean)) {
    penalty = Math.max(penalty, 42);
  }
  if (WRAP_PREPOSITIONS_AND_ARTICLES.has(clean)) {
    penalty = Math.max(penalty, 38);
  }
  if (isProperNamePair(words, k, clean)) {
    penalty = Math.max(penalty, 36);
  }
  if (WRAP_AUXILIARIES_AND_ENCLITICS.has(clean)) {
    penalty = Math.max(penalty, 34);
  }
  if (WRAP_CONJUNCTIONS.has(clean)) {
    penalty = Math.max(penalty, 30);
  }
  if (WRAP_PRONOUNS.has(clean)) {
    penalty = Math.max(penalty, 28);
  }
  return penalty;
}

function punctBreakBonus(word: string): number {
  if (endsWithSentencePunct(word)) {
    return 14;
  }
  if (endsWithClausePunct(word)) {
    return 10;
  }
  return 0;
}

function solveExactLineCountDp(
  prefixChars: number[],
  boundaryCosts: number[],
  totalSingleLineChars: number,
  maxChars: number,
  lineCount: number,
): { overflow: number; splits: number[] } {
  const n = prefixChars.length - 1;
  const l = lineCount;
  const totalLinesChars = totalSingleLineChars - (lineCount - 1);

  const lineLen = (a: number, b: number): number =>
    prefixChars[b] - prefixChars[a] + (b - a - 1);

  const INF = Number.MAX_SAFE_INTEGER;
  const dpOv: number[][] = Array.from({ length: lineCount + 1 }, () =>
    new Array<number>(n + 1).fill(INF),
  );
  const dpSc: number[][] = Array.from({ length: lineCount + 1 }, () =>
    new Array<number>(n + 1).fill(INF),
  );
  const nextSplit: number[][] = Array.from({ length: lineCount + 1 }, () =>
    new Array<number>(n + 1).fill(n),
  );

  for (let i = 0; i < n; i++) {
    const len = lineLen(i, n);
    const ov = Math.max(0, len - maxChars);
    const balance = Math.abs(l * len - totalLinesChars);
    dpOv[1][i] = ov;
    dpSc[1][i] = ov * 1000 * l + balance;
    nextSplit[1][i] = n;
  }

  for (let r = 2; r <= lineCount; r++) {
    const iStart = r === lineCount ? 0 : 1;
    const iEnd = r === lineCount ? 0 : n - r;
    for (let i = iStart; i <= iEnd; i++) {
      let bestOv = INF;
      let bestSc = INF;
      let bestK = i + 1;

      for (let k = i + 1; k <= n - (r - 1); k++) {
        const tailOv = dpOv[r - 1][k];
        if (tailOv >= INF) {
          continue;
        }
        const len = lineLen(i, k);
        const ov = Math.max(0, len - maxChars);
        const balance = Math.abs(l * len - totalLinesChars);
        const candOv = ov + tailOv;
        const candSc =
          ov * 1000 * l + balance + boundaryCosts[k] * l + dpSc[r - 1][k];

        if (candOv < bestOv || (candOv === bestOv && candSc < bestSc)) {
          bestOv = candOv;
          bestSc = candSc;
          bestK = k;
        }
      }

      dpOv[r][i] = bestOv;
      dpSc[r][i] = bestSc;
      nextSplit[r][i] = bestK;
    }
  }

  const splits: number[] = [];
  let cur = 0;
  for (let r = lineCount; r >= 2; r--) {
    const k = nextSplit[r][cur];
    splits.push(k);
    cur = k;
  }

  return { overflow: dpOv[lineCount][0], splits };
}

/**
 * Balances text into at most `maxLines` (`1..=4`) linguistically-clean lines using
 * exact dynamic programming (minimizing overflow first, then smallest feasible line count,
 * then length balance + linguistic split penalties - punctuation break bonuses).
 * Behaviorally 1:1 identical to Rust `segmenter::wrap_balanced_lines`.
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
  const singleLineChars = [...singleLine].length;
  const maxChars = Math.max(1, Math.trunc(maxCharsPerLine));
  const clampedMaxLines = Math.min(4, Math.max(1, Math.trunc(maxLines)));

  if (
    clampedMaxLines <= 1 ||
    singleLineChars <= maxChars ||
    words.length === 1
  ) {
    return singleLine;
  }

  const n = words.length;
  const effectiveMaxLines = Math.min(clampedMaxLines, n);

  const prefixChars: number[] = [0];
  for (const w of words) {
    prefixChars.push(prefixChars[prefixChars.length - 1] + [...w].length);
  }

  const boundaryCosts: number[] = new Array<number>(n).fill(0);
  for (let k = 1; k < n; k++) {
    boundaryCosts[k] =
      linguisticSplitPenalty(words, k) - punctBreakBonus(words[k - 1]);
  }

  let bestOverflow = Number.MAX_SAFE_INTEGER;
  let bestSplits: number[] = [];

  for (let lineCount = 2; lineCount <= effectiveMaxLines; lineCount++) {
    const { overflow, splits } = solveExactLineCountDp(
      prefixChars,
      boundaryCosts,
      singleLineChars,
      maxChars,
      lineCount,
    );
    if (overflow < bestOverflow) {
      bestOverflow = overflow;
      bestSplits = splits;
      if (overflow === 0) {
        break;
      }
    }
  }

  const lines: string[] = [];
  let start = 0;
  for (const end of bestSplits) {
    lines.push(words.slice(start, end).join(" "));
    start = end;
  }
  lines.push(words.slice(start).join(" "));
  return lines.join("\n");
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
