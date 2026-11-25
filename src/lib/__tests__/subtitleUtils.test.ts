// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";

import {
  MIN_INTER_CUE_GAP_MS,
  calculateCps,
  calculateCueMetrics,
  compareSemver,
  computeCueWarnings,
  computeSplitBoundaries,
  convertSerbianScriptText,
  cyrillicToLatin,
  formatSrtDocument,
  formatTimestampSrt,
  formatTimestampVtt,
  formatTxtDocument,
  formatVttDocument,
  getPresetSegmentRules,
  latinToCyrillic,
  normalizeSegmentRules,
  normalizeTranslationStyle,
  parseBulkGlossary,
  parseSubtitleFileText,
  parseTimestampSrt,
  resolveInitialBatchActiveJobId,
  shouldActivateSyncedProject,
  wrapBalancedLines,
} from "../subtitleUtils.ts";
import type {
  JobOptions,
  SegmentRules,
  SubtitleCue,
  TimedWord,
} from "../../types/models.ts";

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
    assert.ok(warnings.includes("duration_too_short"));

    const highCpsCue = {
      startMs: 0,
      endMs: 2000,
      sourceText: "Ovo je malo brži titl sa trideset osam", // 38 chars / 2s = 19.0 CPS (high_cps, not critical_cps)
      translatedText: "",
    };
    const highWarnings = computeCueWarnings(highCpsCue, standardRules);
    assert.deepEqual(highWarnings, ["high_cps"]);

    const longDurationCue = {
      startMs: 0,
      endMs: 9000,
      sourceText: "Veoma dug titl.",
      translatedText: "",
    };
    const longWarnings = computeCueWarnings(longDurationCue, standardRules);
    assert.ok(longWarnings.includes("duration_too_long"));
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

  it("enforces Workstream 7 80 ms inter-cue breathing gap when splitting cues", () => {
    assert.equal(MIN_INTER_CUE_GAP_MS, 80);

    // Normal cue split leaves exact 80 ms gap (splitMs - 40 / splitMs + 40)
    const normal = computeSplitBoundaries(1000, 3000, 2000);
    assert.deepEqual(normal, { leftEndMs: 1960, rightStartMs: 2040 });
    assert.equal(normal.rightStartMs - normal.leftEndMs, 80);

    // Near left edge of a >= 400 ms cue clamps split to keep >= 160 ms per half AND 80 ms gap
    const nearLeft = computeSplitBoundaries(1000, 2000, 1100);
    assert.deepEqual(nearLeft, { leftEndMs: 1160, rightStartMs: 1240 });
    assert.ok(nearLeft.leftEndMs - 1000 >= 160);
    assert.ok(2000 - nearLeft.rightStartMs >= 160);
    assert.equal(nearLeft.rightStartMs - nearLeft.leftEndMs, 80);

    // Near right edge clamps symmetrically
    const nearRight = computeSplitBoundaries(1000, 2000, 1950);
    assert.deepEqual(nearRight, { leftEndMs: 1760, rightStartMs: 1840 });
    assert.ok(2000 - nearRight.rightStartMs >= 160);
    assert.equal(nearRight.rightStartMs - nearRight.leftEndMs, 80);

    // Tight cue (360 ms total: 320..399 ms) keeps >= 160 ms per half with partial 40 ms gap
    const tight = computeSplitBoundaries(0, 360, 180);
    assert.deepEqual(tight, { leftEndMs: 160, rightStartMs: 200 });
    assert.equal(tight.rightStartMs - tight.leftEndMs, 40);

    // Exact 320 ms cue keeps 160 ms per half with 0 ms gap
    const minTwoHalves = computeSplitBoundaries(500, 820, 660);
    assert.deepEqual(minTwoHalves, { leftEndMs: 660, rightStartMs: 660 });

    // Ultra-short cue (< 320 ms) never produces zero-length or inverted halves
    const ultraShort = computeSplitBoundaries(100, 300, 200);
    assert.deepEqual(ultraShort, { leftEndMs: 200, rightStartMs: 200 });
    assert.ok(ultraShort.leftEndMs > 100 && ultraShort.rightStartMs < 300);
  });

  it("guards negations, prepositions, auxiliaries, and proper-name pairs in wrapBalancedLines", () => {
    // Negation guard: never leave "ne" at the end of Line 1
    const negWrapped = wrapBalancedLines(
      "Rekao mi je da ne dolazim sutra na posao.",
      28,
      2,
    );
    const negLines = negWrapped.split("\n");
    assert.equal(negLines.length, 2);
    assert.notEqual(negLines[0].split(" ").at(-1)?.toLowerCase(), "ne");
    assert.notEqual(negLines[0].split(" ").at(-1)?.toLowerCase(), "da");

    // Preposition & auxiliary guard: never end Line 1 on "sa", "smo", or "o"
    const prepWrapped = wrapBalancedLines(
      "Danas smo razgovarali sa našim kolegama o planu.",
      42,
      2,
    );
    assert.equal(
      prepWrapped,
      "Danas smo razgovarali\nsa našim kolegama o planu.",
    );

    // Proper-name pair guard keeps "Nikola Tesla" intact on the same line
    const nameWrapped = wrapBalancedLines(
      "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
      42,
      2,
    );
    assert.ok(
      nameWrapped.includes("Nikola Tesla"),
      `Expected 'Nikola Tesla' intact on one line, got: ${nameWrapped}`,
    );

    // Sentence-opening adverb exclusion allows breaking after "Međutim" while
    // genuine proper-name pair "Aleksandar Tesla" stays together
    assert.equal(
      wrapBalancedLines("Međutim Marko radi.", 14, 2),
      "Međutim\nMarko radi.",
    );
    assert.equal(
      wrapBalancedLines("Aleksandar Tesla radi.", 18, 2),
      "Aleksandar Tesla\nradi.",
    );

    // Hard line-width limit (1000/char overflow) still overrides linguistic penalty when unavoidable
    const forced = wrapBalancedLines(
      "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
      30,
      2,
    );
    assert.equal(
      forced,
      "Danas Marko dolazi ranije u\nkancelariju da završi izveštaj.",
    );
  });

  it("matches Rust 1:1 on the shared 15-sentence bilingual parity corpus (maxLines 1..=4) and canonical warnings", () => {
    const corpus: Array<[string, number, [string, string, string, string]]> = [
      [
        "Rekao mi je da ne dolazim sutra na posao.",
        28,
        [
          "Rekao mi je da ne dolazim sutra na posao.",
          "Rekao mi je da ne dolazim\nsutra na posao.",
          "Rekao mi je da ne dolazim\nsutra na posao.",
          "Rekao mi je da ne dolazim\nsutra na posao.",
        ],
      ],
      [
        "Danas smo razgovarali sa našim kolegama o planu.",
        42,
        [
          "Danas smo razgovarali sa našim kolegama o planu.",
          "Danas smo razgovarali\nsa našim kolegama o planu.",
          "Danas smo razgovarali\nsa našim kolegama o planu.",
          "Danas smo razgovarali\nsa našim kolegama o planu.",
        ],
      ],
      [
        "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
        38,
        [
          "Ovaj projekat predvodi Nikola Tesla u glavnoj laboratoriji.",
          "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
          "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
          "Ovaj projekat predvodi Nikola Tesla\nu glavnoj laboratoriji.",
        ],
      ],
      [
        "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
        30,
        [
          "Danas Marko dolazi ranije u kancelariju da završi izveštaj.",
          "Danas Marko dolazi ranije u\nkancelariju da završi izveštaj.",
          "Danas Marko dolazi\nranije u kancelariju\nda završi izveštaj.",
          "Danas Marko dolazi\nranije u kancelariju\nda završi izveštaj.",
        ],
      ],
      [
        "Овај пројекат предводи Никола Тесла у главној лабораторији у Београду.",
        34,
        [
          "Овај пројекат предводи Никола Тесла у главној лабораторији у Београду.",
          "Овај пројекат предводи Никола Тесла\nу главној лабораторији у Београду.",
          "Овај пројекат предводи\nНикола Тесла у главној\nлабораторији у Београду.",
          "Овај пројекат предводи\nНикола Тесла у главној\nлабораторији у Београду.",
        ],
      ],
      [
        "Нисам сигуран да нећемо морати да кренемо раније него што смо планирали.",
        30,
        [
          "Нисам сигуран да нећемо морати да кренемо раније него што смо планирали.",
          "Нисам сигуран да нећемо морати\nда кренемо раније него што смо планирали.",
          "Нисам сигуран да нећемо морати\nда кренемо раније\nнего што смо планирали.",
          "Нисам сигуран да нећемо морати\nда кренемо раније\nнего што смо планирали.",
        ],
      ],
      [
        "We didn't expect to meet San Francisco engineers at the conference today.",
        30,
        [
          "We didn't expect to meet San Francisco engineers at the conference today.",
          "We didn't expect to meet San Francisco\nengineers at the conference today.",
          "We didn't expect to meet\nSan Francisco engineers\nat the conference today.",
          "We didn't expect to meet\nSan Francisco engineers\nat the conference today.",
        ],
      ],
      [
        "Today John arrives early, because he wants to review the entire draft before noon.",
        30,
        [
          "Today John arrives early, because he wants to review the entire draft before noon.",
          "Today John arrives early, because he wants\nto review the entire draft before noon.",
          "Today John arrives early,\nbecause he wants to review\nthe entire draft before noon.",
          "Today John arrives early,\nbecause he wants to review\nthe entire draft before noon.",
        ],
      ],
      [
        "First sentence ends here. Second sentence opens cleanly! Third part wraps onto line three.",
        36,
        [
          "First sentence ends here. Second sentence opens cleanly! Third part wraps onto line three.",
          "First sentence ends here. Second sentence opens\ncleanly! Third part wraps onto line three.",
          "First sentence ends here.\nSecond sentence opens cleanly!\nThird part wraps onto line three.",
          "First sentence ends here.\nSecond sentence opens cleanly!\nThird part wraps onto line three.",
        ],
      ],
      [
        "Profesor dr. Marko Petrović je objasnio zašto ne smemo da preskočimo ovaj važan korak u analizi podataka.",
        32,
        [
          "Profesor dr. Marko Petrović je objasnio zašto ne smemo da preskočimo ovaj važan korak u analizi podataka.",
          "Profesor dr. Marko Petrović je objasnio zašto ne smemo\nda preskočimo ovaj važan korak u analizi podataka.",
          "Profesor dr. Marko Petrović je objasnio\nzašto ne smemo da preskočimo ovaj\nvažan korak u analizi podataka.",
          "Profesor dr. Marko Petrović\nje objasnio zašto ne smemo\nda preskočimo ovaj važan\nkorak u analizi podataka.",
        ],
      ],
      [
        "Када смо стигли у Нови Сад, одмах смо отишли на састанак са колегама из централног тима за развој софтвера.",
        32,
        [
          "Када смо стигли у Нови Сад, одмах смо отишли на састанак са колегама из централног тима за развој софтвера.",
          "Када смо стигли у Нови Сад, одмах смо отишли на састанак\nса колегама из централног тима за развој софтвера.",
          "Када смо стигли у Нови Сад, одмах\nсмо отишли на састанак са колегама\nиз централног тима за развој софтвера.",
          "Када смо стигли у Нови Сад,\nодмах смо отишли на састанак\nса колегама из централног\nтима за развој софтвера.",
        ],
      ],
      [
        "She told us that she can't join the meeting in New York without her manager's final approval.",
        30,
        [
          "She told us that she can't join the meeting in New York without her manager's final approval.",
          "She told us that she can't join the meeting\nin New York without her manager's final approval.",
          "She told us that she can't join\nthe meeting in New York without\nher manager's final approval.",
          "She told us that she can't\njoin the meeting in New York\nwithout her manager's\nfinal approval.",
        ],
      ],
      [
        "Međutim Marko radi bez prestanka na ovom zadatku.",
        18,
        [
          "Međutim Marko radi bez prestanka na ovom zadatku.",
          "Međutim Marko radi\nbez prestanka na ovom zadatku.",
          "Međutim Marko\nradi bez prestanka\nna ovom zadatku.",
          "Međutim Marko\nradi bez prestanka\nna ovom zadatku.",
        ],
      ],
      [
        "Aleksandar Tesla radi svakog dana u laboratoriji.",
        20,
        [
          "Aleksandar Tesla radi svakog dana u laboratoriji.",
          "Aleksandar Tesla radi\nsvakog dana u laboratoriji.",
          "Aleksandar Tesla\nradi svakog dana\nu laboratoriji.",
          "Aleksandar Tesla\nradi svakog dana\nu laboratoriji.",
        ],
      ],
      [
        "Kratak titl u jednom redu.",
        42,
        [
          "Kratak titl u jednom redu.",
          "Kratak titl u jednom redu.",
          "Kratak titl u jednom redu.",
          "Kratak titl u jednom redu.",
        ],
      ],
    ];

    assert.equal(corpus.length, 15);
    for (const [idx, [text, maxChars, expectedByLines]] of corpus.entries()) {
      for (let maxLines = 1; maxLines <= 4; maxLines++) {
        const actual = wrapBalancedLines(text, maxChars, maxLines);
        assert.equal(
          actual,
          expectedByLines[maxLines - 1],
          `Parity mismatch on sentence #${idx + 1} (maxLines=${maxLines})`,
        );
        assert.equal(
          actual.split("\n").join(" "),
          text,
          `Wrapped text must preserve all words on sentence #${idx + 1}`,
        );
      }
    }

    // Canonical cue warning IDs & thresholds parity matrix
    const standardRules = getPresetSegmentRules("standard");
    const warningCases: Array<{
      cue: Pick<
        SubtitleCue,
        "startMs" | "endMs" | "sourceText" | "translatedText"
      >;
      expectedCps: number;
      expectedWarnings: string[];
    }> = [
      {
        cue: {
          startMs: 1000,
          endMs: 4000,
          sourceText: "Dobro došli u EasySRT Studio.",
          translatedText: "",
        },
        expectedCps: 9.7,
        expectedWarnings: [],
      },
      {
        cue: {
          startMs: 0,
          endMs: 2000,
          sourceText: "Ovo je malo brži titl sa trideset osam",
          translatedText: "",
        },
        expectedCps: 19.0,
        expectedWarnings: ["high_cps"],
      },
      {
        cue: {
          startMs: 0,
          endMs: 500,
          sourceText:
            "Ovaj prvi red ima mnogo više od četrdeset dva karaktera u jednom redu titla.\nDrugi red.\nTreći red.",
          translatedText: "",
        },
        expectedCps: 192.0,
        expectedWarnings: [
          "critical_cps",
          "line_too_long",
          "too_many_lines",
          "duration_too_short",
        ],
      },
      {
        cue: {
          startMs: 0,
          endMs: 8500,
          sourceText: "Dugačak prikaz na ekranu.",
          translatedText: "",
        },
        expectedCps: 2.9,
        expectedWarnings: ["duration_too_long"],
      },
      {
        cue: {
          startMs: 0,
          endMs: 3000,
          sourceText: "Prvi red.\nDrugi red.\nTreći red.",
          translatedText: "",
        },
        expectedCps: 9.7,
        expectedWarnings: ["too_many_lines"],
      },
      {
        cue: {
          startMs: 0,
          endMs: 200,
          sourceText: "   ",
          translatedText: "",
        },
        expectedCps: 0.0,
        expectedWarnings: [],
      },
    ];

    for (const tc of warningCases) {
      const metrics = calculateCueMetrics(tc.cue, standardRules);
      assert.equal(metrics.cps, tc.expectedCps);
      assert.deepEqual(metrics.warnings, tc.expectedWarnings);
    }
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

describe("Batch Queue Active Selection Locking", () => {
  it("keeps the user's selected file active while background batch jobs start or complete", () => {
    // Initially (null or demo), the first batch job becomes active
    assert.equal(resolveInitialBatchActiveJobId(null, "job-1"), "job-1");
    assert.equal(
      resolveInitialBatchActiveJobId("demo-session-01", "job-1"),
      "job-1",
    );
    assert.equal(shouldActivateSyncedProject(null, "job-1"), true);
    assert.equal(
      shouldActivateSyncedProject("demo-session-01", "job-1"),
      true,
    );

    // Once the user selects job-1, background updates or starts for job-2/job-3 must NOT hijack focus
    assert.equal(resolveInitialBatchActiveJobId("job-1", "job-2"), "job-1");
    assert.equal(shouldActivateSyncedProject("job-1", "job-2"), false);
    assert.equal(shouldActivateSyncedProject("job-1", "job-3"), false);

    // Updates for the selected job itself (e.g., finishing translation) DO update the active editor view
    assert.equal(shouldActivateSyncedProject("job-1", "job-1"), true);

    // Explicit user actions (loading checkpoint / importing subtitle) force activation
    assert.equal(shouldActivateSyncedProject("job-1", "job-2", true), true);
  });
});

describe("Semantic Version Comparison", () => {
  it("compares semver strings with or without v-prefix cleanly", () => {
    assert.equal(compareSemver("v0.1.2", "0.1.1"), 1);
    assert.equal(compareSemver("0.2.0", "v0.1.9"), 1);
    assert.equal(compareSemver("1.0.0", "0.9.9"), 1);
    assert.equal(compareSemver("v0.1.1", "0.1.1"), 0);
    assert.equal(compareSemver("0.1.1-beta", "v0.1.1"), 0);
    assert.equal(compareSemver("v0.1.0", "0.1.1"), -1);
  });
});

describe("Speaker-Change Rules & Translation Style Contracts", () => {
  it("defaults splitOnSpeakerChange to true for all presets and preserves explicit overrides", () => {
    assert.equal(getPresetSegmentRules("standard").splitOnSpeakerChange, true);
    assert.equal(getPresetSegmentRules("reels").splitOnSpeakerChange, true);
    assert.equal(getPresetSegmentRules("custom").splitOnSpeakerChange, true);

    const legacyRules: SegmentRules = {
      preset: "reels",
      maxCharsPerLine: 20,
      maxLines: 1,
      minDurationMs: 600,
      maxDurationMs: 3500,
      maxCps: 18,
    };
    assert.equal(
      getPresetSegmentRules("standard", legacyRules).splitOnSpeakerChange,
      true,
    );
    assert.equal(
      normalizeSegmentRules(legacyRules).splitOnSpeakerChange,
      true,
    );
    assert.equal(normalizeSegmentRules(legacyRules).preset, "reels");

    const disabledRules: SegmentRules = {
      ...legacyRules,
      splitOnSpeakerChange: false,
    };
    assert.equal(
      getPresetSegmentRules("standard", disabledRules).splitOnSpeakerChange,
      false,
    );
    assert.equal(
      getPresetSegmentRules("reels", disabledRules).splitOnSpeakerChange,
      false,
    );
    assert.equal(
      getPresetSegmentRules("custom", disabledRules).splitOnSpeakerChange,
      false,
    );
    assert.equal(
      normalizeSegmentRules(disabledRules).splitOnSpeakerChange,
      false,
    );
  });

  it("normalizes translationStyle with backward-compatible natural default", () => {
    assert.equal(normalizeTranslationStyle(undefined), "natural");
    assert.equal(normalizeTranslationStyle(null), "natural");
    assert.equal(normalizeTranslationStyle(""), "natural");
    assert.equal(normalizeTranslationStyle("natural"), "natural");
    assert.equal(normalizeTranslationStyle("formal"), "formal");
    assert.equal(normalizeTranslationStyle("unknown"), "natural");

    const word: TimedWord = {
      startMs: 0,
      endMs: 300,
      text: "Hello",
      probability: 0.99,
      speakerChange: true,
    };
    assert.equal(word.speakerChange, true);

    const opts: JobOptions = {
      sourceLanguage: "en",
      targetLanguage: "sr",
      script: "latin",
      segmentRules: getPresetSegmentRules("standard"),
      glossary: {},
      polishSerbian: true,
      translationStyle: "formal",
      providerOverride: null,
    };
    assert.equal(normalizeTranslationStyle(opts.translationStyle), "formal");
  });
});
