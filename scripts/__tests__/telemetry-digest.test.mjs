import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  formatDigestMarkdown,
  parseDigestArgs,
  resolveAdminToken,
  runTelemetryDigest,
  stripTerminalControls,
} from "../telemetry-digest.mjs";

describe("telemetry-digest CLI security and token handling (V05-M4)", () => {
  it("reads TELEMETRY_ADMIN_TOKEN strictly from env and rejects missing or unsafe tokens", () => {
    assert.throws(() => resolveAdminToken({}), /Missing TELEMETRY_ADMIN_TOKEN/);
    assert.throws(() => resolveAdminToken({ TELEMETRY_ADMIN_TOKEN: "   " }), /Missing TELEMETRY_ADMIN_TOKEN/);
    assert.throws(
      () => resolveAdminToken({ TELEMETRY_ADMIN_TOKEN: "tok with space" }),
      /whitespace or control/,
    );
    assert.throws(
      () => resolveAdminToken({ TELEMETRY_ADMIN_TOKEN: "tok\nnewline" }),
      /whitespace or control/,
    );
    assert.equal(resolveAdminToken({ TELEMETRY_ADMIN_TOKEN: "valid-secret-token_123" }), "valid-secret-token_123");
  });

  it("refuses passing tokens via CLI flags and clamps --days to 1..365", () => {
    for (const flag of ["--token", "--admin-token", "-t", "--bearer", "--token=abc"]) {
      assert.throws(() => parseDigestArgs([flag], {}), /Refusing CLI flag/);
    }

    assert.equal(parseDigestArgs(["--days", "0"], {}).days, 1);
    assert.equal(parseDigestArgs(["--days=999"], {}).days, 365);
    assert.equal(parseDigestArgs(["--days", "14"], {}).days, 14);
    assert.throws(() => parseDigestArgs(["--url", "http://evil.example"], {}), /insecure telemetry URL/);
    assert.equal(
      parseDigestArgs(["--url", "http://127.0.0.1:8787/"], {}).baseUrl,
      "http://127.0.0.1:8787",
    );
  });

  it("strips ANSI CSI, OSC, and C0/C1 control sequences from server & GitHub strings", () => {
    const poisoned =
      "ok\x1b[31mRED\x1b[0m \x1b]8;;https://evil.example\x07click\x1b]8;;\x1b\\ \x1bPabc\x1b\\ \x00\x07\x9b";
    const clean = stripTerminalControls(poisoned);
    assert.ok(!clean.includes("\x1b"));
    assert.ok(!clean.includes("\x00"));
    assert.ok(!clean.includes("\x07"));
    assert.ok(!clean.includes("\x9b"));
    assert.equal(clean, "okRED click  ");
  });
});

describe("telemetry-digest report fetching and Markdown rendering", () => {
  it("fetches /v1/admin/digest and GitHub releases, strips ANSI escapes, and renders Markdown", async () => {
    const calls = [];
    const mockFetch = async (url, init) => {
      calls.push({ url, headers: init?.headers ?? {} });
      if (url.startsWith("https://easysrt-telemetry.ig-systems.rs/v1/admin/digest")) {
        assert.equal(init.headers.Authorization, "Bearer admin-secret-xyz");
        return new Response(
          JSON.stringify({
            ok: true,
            generatedAt: "2026-11-27T12:00:00Z",
            windowDays: 30,
            kAnonymityMinCohorts: 5,
            kAnonymityMinSubnets: 3,
            dau: {
              dailyTotals: [{ utc_date: "2026-11-27", total_checks: 42 }],
              byOsAndVersion: [
                {
                  utc_date: "2026-11-27",
                  target: "windows",
                  arch: "x86_64",
                  current_version: "0.5.0",
                  check_count: 30,
                },
              ],
            },
            performance: [
              {
                gpu_tier: "vram8Gb",
                whisper_backend: "cuda",
                llm_provider: "localLlama",
                llm_model_id: "gemma3\x1b[31m-evil",
                job_count: 12,
                avg_asr_rtf: 0.081,
                avg_llm_rtf: 0.145,
                oom_fallbacks: 0,
                batch_retries: 1,
              },
            ],
            editTrends: [
              {
                app_version: "0.5.0",
                target_lang: "sr",
                translation_style: "natural",
                export_count: 10,
                avg_edit_rate_pct: 18.5,
                avg_cps_before: 16.2,
                avg_cps_after: 14.0,
                enclitic_reordered: 3,
                redundant_possessive_removed: 2,
                passive_calque_removed: 1,
                conjunction_comma_fixed: 4,
                missed_filler_deleted: 2,
                inflection_suffix_fix: 5,
                boundary_orphan_shifted: 1,
                split_count: 2,
                merge_count: 1,
                avg_start_delta_ms: -40,
                avg_end_delta_ms: 110,
              },
            ],
            promotedNgrams: [
              {
                lang_pair: "en-sr",
                old_span: "tijekom\x1b]0;pwned\x07",
                new_span: "tokom|pipe",
                cohort_count: 8,
                subnet_count: 5,
                updated_at: "2026-11-27T01:00:00Z",
              },
            ],
            trainingPairsSummary: {
              totalCount: 15,
              byLanguageAndStyle: [
                {
                  source_lang: "en",
                  target_lang: "sr",
                  style: "natural",
                  address_form: "auto",
                  pair_count: 15,
                },
              ],
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (url.startsWith("https://api.github.com/repos/Gruya13/EasySRT-Studio/releases")) {
        return new Response(
          JSON.stringify([
            {
              tag_name: "v0.4.0\x1b[32m",
              published_at: "2026-11-25T10:00:00Z",
              assets: [{ download_count: 120 }, { download_count: 45 }],
            },
          ]),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`Unexpected fetch: ${url}`);
    };

    const output = await runTelemetryDigest({
      argv: ["--days", "30"],
      env: { TELEMETRY_ADMIN_TOKEN: "admin-secret-xyz" },
      fetchImpl: mockFetch,
    });

    assert.equal(calls.length, 2);
    assert.ok(!output.includes("\x1b"));
    assert.ok(!output.includes("\x07"));
    assert.ok(output.includes("Total downloads across recent 1 release(s): **165**"));
    assert.ok(output.includes("| 2026-11-27 | 42 |"));
    assert.ok(output.includes("gemma3-evil"));
    assert.ok(output.includes("tokom\\|pipe"));
    assert.ok(output.includes("Total sanitized cue pairs in window: **15**"));
  });

  it("renders digest gracefully when GitHub release API fails", () => {
    const md = formatDigestMarkdown({
      digest: {
        generatedAt: "2026-11-27T12:00:00Z",
        windowDays: 7,
        dau: { dailyTotals: [], byOsAndVersion: [] },
        performance: [],
        editTrends: [],
        promotedNgrams: [],
        trainingPairsSummary: { totalCount: 0, byLanguageAndStyle: [] },
      },
      releases: [],
      releasesError: "HTTP 403\x1b[31m rate limit",
      repo: "Gruya13/EasySRT-Studio",
      days: 7,
    });
    assert.ok(!md.includes("\x1b"));
    assert.ok(md.includes("GitHub release stats unavailable: HTTP 403 rate limit"));
  });
});
