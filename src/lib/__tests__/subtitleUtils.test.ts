// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";

import {
  calculateCps,
  calculateCueMetrics,
  computeCueWarnings,
  convertSerbianScriptText,
  cyrillicToLatin,
  formatSrtDocument,
  formatTimestampSrt,
  formatTimestampVtt,
  formatTxtDocument,
  formatVttDocument,
  getPresetSegmentRules,
  latinToCyrillic,
  parseBulkGlossary,
  parseSubtitleFileText,
  parseTimestampSrt,
  wrapBalancedLines,
} from "../subtitleUtils.ts";
import type { SubtitleCue } from "../../types/models.ts";

describe("Timestamp Formatting & Parsing", () => {
  it("formats SRT and VTT timestamps across hours, minutes, seconds, and milliseconds", () => {
    assert.equal(formatTimestampSrt(0), "00:00:00,000");
    assert.equal(formatTimestampVtt(0), "00:00:00.000");

    assert.equal(formatTimestampSrt(-250), "00:00:00,000");
    assert.equal(formatTimestampSrt(500), "00:00:00,500");
    assert.equal(formatTimestampVtt(500), "00:00:00.500");

    // 1h 23m 45s 678ms = 3600000 + 1380000 + 45000 + 678 = 5025678
    assert.equal(formatTimestampSrt(5_025_678), "01:23:45,678");
    assert.equal(formatTimestampVtt(5_025_678), "01:23:45.678");
  });

  it("parses valid SRT and VTT timestamps and rejects out-of-range or malformed strings", () => {
    assert.equal(parseTimestampSrt("00:00:00,000"), 0);
    assert.equal(parseTimestampSrt("00:00:00.500"), 500);
    assert.equal(parseTimestampSrt("01:23:45,678"), 5_025_678);
    assert.equal(parseTimestampSrt("01:23:45.678"), 5_025_678);

    // Short MM:SS.mmm VTT format and partial millisecond digits
    assert.equal(parseTimestampSrt("02:05.500"), 125_500);
    assert.equal(parseTimestampSrt("00:01:02,5"), 62_500);

    // Invalid boundaries (minutes >= 60, seconds >= 60, malformed text)
    assert.equal(parseTimestampSrt("00:60:00,000"), null);
    assert.equal(parseTimestampSrt("00:00:60,000"), null);
    assert.equal(parseTimestampSrt("not-a-timestamp"), null);
    assert.equal(parseTimestampSrt(""), null);
  });
});

describe("CPS, Warnings & Balanced Line Wrapping", () => {
  it("calculates CPS accurately for single-line and multi-line text", () => {
    // 20 characters over 2000ms = 10.0 CPS
    assert.equal(calculateCps("12345678901234567890", 0, 2000), 10.0);
    // Newlines are excluded from character count
    assert.equal(calculateCps("1234567890\n1234567890", 1000, 3000), 10.0);
    // Supports 2-argument (text, durationMs) form as well
    assert.equal(calculateCps("1234567890", 1000), 10.0);
    assert.equal(calculateCps("   ", 0, 2000), 0);
  });

  it("computes cue warnings for high/critical CPS, line length, line count, and duration", () => {
    const standardRules = getPresetSegmentRules("standard"); // 2x42, maxCps=17, min=1000, max=7000

    const cleanCue = {
      startMs: 1000,
      endMs: 4000,
      sourceText: "Dobro došli u EasySRT Studio.",
      translatedText: "",
    };
    assert.deepEqual(computeCueWarnings(cleanCue, standardRules), []);

    // Fast reading speed + short duration + long line + 3 lines
    const overloadedCue = {
      startMs: 0,
      endMs: 500,
      sourceText:
        "Ovaj prvi red ima mnogo više od četrdeset dva karaktera u jednom redu titla.\nDrugi red.\nTreći red.",
      translatedText: "",
    };
    const warnings = computeCueWarnings(overloadedCue, standardRules);
    assert.ok(warnings.includes("critical_cps"));
    assert.ok(warnings.includes("line_too_long"));
    assert.ok(warnings.includes("too_many_lines"));
    assert.ok(warnings.includes("short_duration"));

    const longDurationCue = {
      startMs: 0,
      endMs: 9000,
      sourceText: "Veoma dug titl.",
      translatedText: "",
    };
    const longWarnings = computeCueWarnings(longDurationCue, standardRules);
    assert.ok(longWarnings.includes("long_duration"));
  });

  it("wraps long sentences into two balanced lines within maxCharsPerLine", () => {
    const shortText = "Kratak titl u jednom redu.";
    assert.equal(wrapBalancedLines(shortText, 42, 2), shortText);

    const longText =
      "Ovo je jedna prilično dugačka rečenica koja mora da se podeli u dva balansirana reda.";
    const wrapped = wrapBalancedLines(longText, 45, 2);
    const lines = wrapped.split("\n");
    assert.equal(lines.length, 2);
    assert.ok([...lines[0]].length <= 45, `Line 1 too long: ${lines[0]}`);
    assert.ok([...lines[1]].length <= 45, `Line 2 too long: ${lines[1]}`);
    assert.equal(lines.join(" "), longText);
  });
});

describe("Serbian Cyrillic <-> Latin Transliteration", () => {
  it("converts Cyrillic to Latin with smart digraph casing (Љ, Њ, Џ)", () => {
    assert.equal(cyrillicToLatin("ЉУБАВ"), "LJUBAV");
    assert.equal(cyrillicToLatin("Љубав"), "Ljubav");
    assert.equal(cyrillicToLatin("љубав"), "ljubav");

    assert.equal(cyrillicToLatin("ЊЕГОШ"), "NJEGOŠ");
    assert.equal(cyrillicToLatin("Његош"), "Njegoš");
    assert.equal(cyrillicToLatin("његош"), "njegoš");

    assert.equal(cyrillicToLatin("ЏЕП"), "DŽEP");
    assert.equal(cyrillicToLatin("Џеп"), "Džep");
    assert.equal(cyrillicToLatin("џеп"), "džep");

    assert.equal(cyrillicToLatin("КОЊ"), "KONJ");
    assert.equal(cyrillicToLatin("Коњ"), "Konj");
  });

  it("converts Latin to Cyrillic respecting digraphs, prefix exceptions, and protected tags/URLs", () => {
    assert.equal(
      latinToCyrillic("Ljubav i pažnja u džepu."),
      "Љубав и пажња у џепу.",
    );
    assert.equal(
      latinToCyrillic("LJUBAV I PAŽNJA U DŽEPU."),
      "ЉУБАВ И ПАЖЊА У ЏЕПУ.",
    );

    // Morphological prefix-boundary exceptions where n+j or d+ž stay separate
    assert.equal(latinToCyrillic("injekcija"), "инјекција");
    assert.equal(latinToCyrillic("Injekcija"), "Инјекција");
    assert.equal(latinToCyrillic("INJEKCIJA"), "ИНЈЕКЦИЈА");
    assert.equal(latinToCyrillic("konjugacija"), "конјугација");
    assert.equal(latinToCyrillic("konjunkcija"), "конјункција");
    assert.equal(latinToCyrillic("adjektiv"), "адјектив");
    assert.equal(latinToCyrillic("nadživeti"), "надживети");
    assert.equal(latinToCyrillic("odživeti"), "одживети");
    assert.equal(latinToCyrillic("podžanr"), "поджанр");
    assert.equal(latinToCyrillic("predživot"), "предживот");
    assert.equal(latinToCyrillic("Tanjug"), "Танјуг");

    // Protected spans (HTML tags, URLs, emails, [MARKERS], ASS tags)
    const withProtected =
      "<i>Posetite https://example.com/ljubav ili pišite na info@primer.rs [SMEH] {\\an8}</i>";
    assert.equal(
      latinToCyrillic(withProtected),
      "<i>Посетите https://example.com/ljubav или пишите на info@primer.rs [SMEH] {\\an8}</i>",
    );

    // Round-trip via convertSerbianScriptText
    const sample = "Đorđe Čvarkov i Žika Šarenica gledaju film u Nišu.";
    const cyr = convertSerbianScriptText(sample, "cyrillic");
    assert.equal(
      cyr,
      "Ђорђе Чварков и Жика Шареница гледају филм у Нишу.",
    );
    assert.equal(convertSerbianScriptText(cyr, "latin"), sample);
  });
});

describe("Bulk Glossary Parser", () => {
  it("parses arrow pairs, equals pairs, and standalone terms across lines and commas", () => {
    const raw =
      "EasySRT Studio -> EasySRT Studio\nwhisper.cpp = whisper.cpp\nNVIDIA RTX, CUDA -> CUDA";
    const parsed = parseBulkGlossary(raw);
    assert.deepEqual(parsed, {
      "EasySRT Studio": "EasySRT Studio",
      "whisper.cpp": "whisper.cpp",
      "NVIDIA RTX": "NVIDIA RTX",
      CUDA: "CUDA",
    });
  });
});

describe("SRT & WebVTT Parsing and Formatting", () => {
  const rules = getPresetSegmentRules("standard");

  it("parses .srt content with UTF-8 BOM, CRLF line endings, and multi-line Serbian text", () => {
    const srtContent =
      "\uFEFF1\r\n00:00:00,500 --> 00:00:03,500\r\nDobro došli u studio!\r\nDanas testiramo čćžšđ ČĆŽŠĐ.\r\n\r\n2\r\n00:00:04,000 --> 00:00:06,500\r\nDrugi titl u fajlu.\r\n";
    const cues = parseSubtitleFileText(srtContent, rules);
    assert.equal(cues.length, 2);
    assert.equal(cues[0].id, 1);
    assert.equal(cues[0].startMs, 500);
    assert.equal(cues[0].endMs, 3500);
    assert.equal(
      cues[0].sourceText,
      "Dobro došli u studio!\nDanas testiramo čćžšđ ČĆŽŠĐ.",
    );
    assert.ok(cues[0].cps > 0);

    assert.equal(cues[1].id, 2);
    assert.equal(cues[1].startMs, 4000);
    assert.equal(cues[1].endMs, 6500);
    assert.equal(cues[1].sourceText, "Drugi titl u fajlu.");
  });

  it("parses .vtt content with WEBVTT header, NOTE blocks, cue settings, and short timestamps", () => {
    const vttContent = [
      "WEBVTT - EasySRT Studio",
      "Kind: captions",
      "Language: sr",
      "",
      "NOTE Ovo je komentar u VTT fajlu",
      "",
      "1",
      "00:00:01.250 --> 00:00:04.000 align:start position:0%",
      "Prvi WebVTT titl sa podešavanjima.",
      "",
      "00:05.500 --> 00:08.750 line:85%",
      "Drugi WebVTT titl bez eksplicitnog broja.",
    ].join("\n");

    const cues = parseSubtitleFileText(vttContent, rules);
    assert.equal(cues.length, 2);
    assert.equal(cues[0].id, 1);
    assert.equal(cues[0].startMs, 1250);
    assert.equal(cues[0].endMs, 4000);
    assert.equal(cues[0].sourceText, "Prvi WebVTT titl sa podešavanjima.");

    assert.equal(cues[1].id, 2);
    assert.equal(cues[1].startMs, 5500);
    assert.equal(cues[1].endMs, 8750);
    assert.equal(
      cues[1].sourceText,
      "Drugi WebVTT titl bez eksplicitnog broja.",
    );
  });

  it("formats SRT, VTT, and TXT documents with optional UTF-8 BOM and round-trips cleanly", () => {
    const sampleCues: SubtitleCue[] = [
      {
        id: 1,
        startMs: 500,
        endMs: 2750,
        sourceText: "Hello everyone!",
        translatedText: "Zdravo svima!\nČestitamo Đorđu.",
        cps: calculateCueMetrics(
          {
            startMs: 500,
            endMs: 2750,
            sourceText: "Hello everyone!",
            translatedText: "Zdravo svima!\nČestitamo Đorđu.",
          },
          rules,
        ).cps,
        warnings: [],
      },
      {
        id: 2,
        startMs: 3000,
        endMs: 5125,
        sourceText: "Second segment.",
        translatedText: "Drugi segment: čćžšđ ЉЊЏ.",
        cps: 11.8,
        warnings: [],
      },
    ];

    const srt = formatSrtDocument(sampleCues, {
      useSource: false,
      utf8Bom: true,
    });
    assert.ok(srt.startsWith("\uFEFF"));
    assert.ok(
      srt.includes(
        "1\n00:00:00,500 --> 00:00:02,750\nZdravo svima!\nČestitamo Đorđu.\n",
      ),
    );

    const sourceSrt = formatSrtDocument(sampleCues, {
      useSource: true,
      utf8Bom: false,
    });
    assert.ok(!sourceSrt.startsWith("\uFEFF"));
    assert.ok(sourceSrt.includes("Hello everyone!"));

    const vtt = formatVttDocument(sampleCues, false, false);
    assert.ok(vtt.startsWith("WEBVTT\n\n"));
    assert.ok(vtt.includes("00:00:00.500 --> 00:00:02.750"));

    const txt = formatTxtDocument(sampleCues, false, false);
    assert.equal(
      txt,
      "Zdravo svima! Čestitamo Đorđu.\nDrugi segment: čćžšđ ЉЊЏ.\n",
    );

    // Round-trip SRT & VTT back through parseSubtitleFileText
    const reparsedSrt = parseSubtitleFileText(srt, rules);
    assert.equal(reparsedSrt.length, 2);
    assert.equal(reparsedSrt[0].startMs, 500);
    assert.equal(reparsedSrt[0].endMs, 2750);
    assert.equal(reparsedSrt[0].sourceText, "Zdravo svima!\nČestitamo Đorđu.");

    const reparsedVtt = parseSubtitleFileText(vtt, rules);
    assert.equal(reparsedVtt.length, 2);
    assert.equal(reparsedVtt[1].startMs, 3000);
    assert.equal(reparsedVtt[1].endMs, 5125);
    assert.equal(reparsedVtt[1].sourceText, "Drugi segment: čćžšđ ЉЊЏ.");
  });
});
