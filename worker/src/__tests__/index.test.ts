// @ts-ignore Node built-in test runner without @types/node
import { describe, it, afterEach } from "node:test";
// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in fs/sqlite without @types/node
import { readFileSync } from "node:fs";
// @ts-ignore Node built-in sqlite without @types/node
import { DatabaseSync } from "node:sqlite";

import worker, {
  MAX_BODY_BYTES,
  MAX_TELEMETRY_BODY_BYTES,
  MAX_TRAINING_PAIRS_BODY_BYTES,
  extractSubnetPrefix,
  neutralizeGitHubMentions,
  runScheduledCleanup,
  sanitizeText,
  type Env,
} from "../index.ts";

const TEST_ENV: Env = {
  GITHUB_REPO: "Gruya13/EasySRT-Studio",
  GITHUB_ISSUES_TOKEN: "test-placeholder-token-not-real",
};

const SYNTHETIC_GEMINI_KEY = ["AIza", "SyTestDummyKey0123456789abcdefghijk"].join("");

const originalFetch = globalThis.fetch;
const originalCaches = (globalThis as { caches?: unknown }).caches;

afterEach(() => {
  globalThis.fetch = originalFetch;
  if (originalCaches === undefined) {
    delete (globalThis as { caches?: unknown }).caches;
  } else {
    (globalThis as { caches?: unknown }).caches = originalCaches;
  }
});

interface RecordedFetch {
  url: string;
  method: string;
  headers: Headers;
  bodyText: string | null;
}

function installMockFetch(
  handler: (call: RecordedFetch, index: number) => Response | Promise<Response>,
): RecordedFetch[] {
  const calls: RecordedFetch[] = [];
  globalThis.fetch = (async (
    input: string | URL | Request,
    init?: RequestInit,
  ): Promise<Response> => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const headers = new Headers(
      init?.headers ?? (input instanceof Request ? input.headers : undefined),
    );
    const bodyText =
      typeof init?.body === "string"
        ? init.body
        : init?.body instanceof Uint8Array
          ? new TextDecoder().decode(init.body)
          : null;

    const record: RecordedFetch = { url, method, headers, bodyText };
    const idx = calls.length;
    calls.push(record);
    return await handler(record, idx);
  }) as typeof fetch;
  return calls;
}

function createInMemoryCache(recordedKeys?: string[]): Cache {
  const store = new Map<string, Response>();
  return {
    async match(request: RequestInfo | URL): Promise<Response | undefined> {
      const key = typeof request === "string" ? request : request instanceof URL ? request.toString() : request.url;
      recordedKeys?.push(key);
      const hit = store.get(key);
      return hit ? hit.clone() : undefined;
    },
    async put(request: RequestInfo | URL, response: Response): Promise<void> {
      const key = typeof request === "string" ? request : request instanceof URL ? request.toString() : request.url;
      recordedKeys?.push(key);
      store.set(key, response.clone());
    },
    async delete(request: RequestInfo | URL): Promise<boolean> {
      const key = typeof request === "string" ? request : request instanceof URL ? request.toString() : request.url;
      return store.delete(key);
    },
  } as unknown as Cache;
}

// @ts-ignore Node runtime import.meta.url without @types/node
const SCHEMA_SQL = readFileSync(new URL("../../schema.sql", import.meta.url), "utf8");

function createTestD1(): { db: D1Database; sqlite: DatabaseSync } {
  const sqlite = new DatabaseSync(":memory:");
  sqlite.exec(SCHEMA_SQL);

  const makeStmt = (sql: string, params: unknown[] = []): D1PreparedStatement => {
    const runSync = () => {
      const stmt = sqlite.prepare(sql);
      const res = stmt.run(...(params as Array<string | number | null | Uint8Array>));
      return {
        success: true,
        meta: {
          duration: 0,
          changes: Number(res.changes),
          last_row_id: Number(res.lastInsertRowid),
          changed_db: Number(res.changes) > 0,
          size_after: 0,
          rows_read: 0,
          rows_written: Number(res.changes),
        },
        results: [],
      };
    };

    const allSync = () => {
      const stmt = sqlite.prepare(sql);
      const rows = stmt.all(...(params as Array<string | number | null | Uint8Array>));
      return {
        success: true,
        meta: {
          duration: 0,
          changes: 0,
          last_row_id: 0,
          changed_db: false,
          size_after: 0,
          rows_read: rows.length,
          rows_written: 0,
        },
        results: rows.map((row: Record<string, unknown>) => ({ ...row })),
      };
    };

    const prepared = {
      bind(...bound: unknown[]) {
        return makeStmt(sql, bound);
      },
      async first<T = unknown>(colName?: string): Promise<T | null> {
        const res = allSync();
        const row = res.results[0] as Record<string, unknown> | undefined;
        if (!row) return null;
        if (typeof colName === "string") {
          return (row[colName] ?? null) as T | null;
        }
        return row as T;
      },
      async run<T = Record<string, unknown>>() {
        return runSync() as unknown as D1Result<T>;
      },
      async all<T = Record<string, unknown>>() {
        return allSync() as unknown as D1Result<T>;
      },
      async raw() {
        return [];
      },
      _executeForBatch() {
        const trimmed = sql.trimStart().toUpperCase();
        if (trimmed.startsWith("SELECT")) {
          return allSync();
        }
        return runSync();
      },
    };
    return prepared as unknown as D1PreparedStatement;
  };

  const db = {
    prepare(query: string) {
      return makeStmt(query);
    },
    async batch(statements: D1PreparedStatement[]) {
      return statements.map((s) =>
        (s as unknown as { _executeForBatch: () => D1Result<Record<string, unknown>> })._executeForBatch(),
      );
    },
    async exec(query: string) {
      sqlite.exec(query);
      return { count: 1, duration: 0 };
    },
  } as unknown as D1Database;

  return { db, sqlite };
}

function createTestExecutionContext(): { ctx: ExecutionContext; flush: () => Promise<void> } {
  const pending: Promise<unknown>[] = [];
  const ctx = {
    waitUntil(promise: Promise<unknown>) {
      pending.push(promise);
    },
    passThroughOnException() {},
  } as unknown as ExecutionContext;
  return {
    ctx,
    async flush() {
      await Promise.all(pending.splice(0));
    },
  };
}

describe("Health, Routing, and Origin / CORS Policy", () => {
  it("rejects browser Origin requests to bug and crash endpoints (POST and OPTIONS) with 403 and no CORS wildcard", async () => {
    const calls = installMockFetch(() => new Response("should not be called", { status: 500 }));

    const originCases: Array<{ label: string; req: Request }> = [
      {
        label: "POST bug with Origin",
        req: new Request("https://telemetry.example.workers.dev/v1/report/bug", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Origin: "https://evil.example",
          },
          body: JSON.stringify({
            appVersion: "0.4.0",
            title: "Should be blocked",
            description: "Browser Origin requests must be rejected",
          }),
        }),
      },
      {
        label: "POST crash with Origin",
        req: new Request("https://telemetry.example.workers.dev/v1/report/crash", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Origin: "https://evil.example",
          },
          body: JSON.stringify({
            appVersion: "0.4.0",
            crashKind: "rust_panic",
            fingerprint: "a1b2c3d4e5f60708",
            message: "Should be blocked",
          }),
        }),
      },
      {
        label: "POST crash with null Origin",
        req: new Request("https://telemetry.example.workers.dev/v1/report/crash", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Origin: "null",
          },
          body: "{}",
        }),
      },
      {
        label: "OPTIONS preflight on bug with Origin",
        req: new Request("https://telemetry.example.workers.dev/v1/report/bug", {
          method: "OPTIONS",
          headers: {
            Origin: "https://evil.example",
            "Access-Control-Request-Method": "POST",
          },
        }),
      },
      {
        label: "OPTIONS preflight on crash with Origin",
        req: new Request("https://telemetry.example.workers.dev/v1/report/crash", {
          method: "OPTIONS",
          headers: {
            Origin: "https://evil.example",
            "Access-Control-Request-Method": "POST",
          },
        }),
      },
    ];

    for (const tc of originCases) {
      const res = await worker.fetch(tc.req, TEST_ENV);
      assert.equal(res.status, 403, `Expected 403 for: ${tc.label}`);
      assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
      const body = (await res.json()) as { ok: boolean; error: string };
      assert.equal(body.ok, false);
      assert.ok(body.error.length > 0);
    }

    assert.equal(calls.length, 0);
  });

  it("rejects OPTIONS without Origin on report routes with 405 and no CORS header", async () => {
    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "OPTIONS",
    });
    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 405);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
    assert.equal(res.headers.get("Allow"), "POST");
  });

  it("responds to GET /health with 200, service identity, and no wildcard CORS header", async () => {
    const req = new Request("https://telemetry.example.workers.dev/health", {
      method: "GET",
    });
    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
    const body = (await res.json()) as { ok: boolean; service: string };
    assert.deepEqual(body, { ok: true, service: "easysrt-telemetry" });
  });

  it("returns 404 for unknown paths and 405 for unsupported methods", async () => {
    const notFound = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/unknown"),
      TEST_ENV,
    );
    assert.equal(notFound.status, 404);
    assert.equal(notFound.headers.get("Access-Control-Allow-Origin"), null);

    const wrongMethod = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/report/bug", { method: "GET" }),
      TEST_ENV,
    );
    assert.equal(wrongMethod.status, 405);
    assert.equal(wrongMethod.headers.get("Access-Control-Allow-Origin"), null);
  });
});

describe("Server-side Sanitization & Mention Neutralization", () => {
  it("redacts API keys, HF/Gemini/GitHub tokens, Bearer tokens, x-api-key, URL userinfo, and Win/Linux/macOS user paths", () => {
    assert.equal(SYNTHETIC_GEMINI_KEY.length, 39);
    const input = [
      "OpenAI key: sk-proj_abc123456789XYZ",
      "DeepSeek key: sk-1234567890abcdef",
      "HF token: hf_AbCdEfGhIjKlMnOpQrStUvWx1234",
      `Gemini key: ${SYNTHETIC_GEMINI_KEY}`,
      "GH classic: ghp_AbCdEf0123456789xyz",
      "GH fine-grained: github_pat_11ABCDEF_secretToken99",
      "Header: Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig",
      "Header2: X-API-Key: anthropicHeaderSecret42",
      "Query: https://api.example.com/v1?x-api-key=querySecret99&model=gemini",
      'JsonHeader: {"x-api-key": "jsonHeaderSecret77", "ok": true}',
      "Config: api_key=mySuperSecretValue123",
      "CredUrl1: https://alice:s3cr3tPass@api.example.com/v1/chat",
      "CredUrl2: socks5://proxyUser:proxyPass@192.168.1.5:1080",
      "CleanUrlWithEmailParam: https://example.com/help?contact=support@example.com",
      "Win backslash: C:\\Users\\Marko\\AppData\\Local\\com.easysrt.studio\\logs\\app.log",
      "Win slash: D:/Users/Ana/Videos/sample.mp4",
      "Linux home: /home/alice/Projects/EasySRT Studio/file.srt",
      "macOS home: /Users/elena/Movies/interview.mov",
      "macOS env: HOME=/Users/stefan",
    ].join("\n");

    const clean = sanitizeText(input);
    assert.ok(!clean.includes("sk-proj_abc123456789XYZ"));
    assert.ok(!clean.includes("sk-1234567890abcdef"));
    assert.ok(!clean.includes("hf_AbCdEfGhIjKlMnOpQrStUvWx1234"));
    assert.ok(!clean.includes(SYNTHETIC_GEMINI_KEY));
    assert.ok(clean.includes("Gemini key: [REDACTED_API_KEY]"));
    assert.ok(!clean.includes("ghp_AbCdEf0123456789xyz"));
    assert.ok(!clean.includes("github_pat_11ABCDEF_secretToken99"));
    assert.ok(!clean.includes("eyJhbGciOiJIUzI1NiJ9"));
    assert.ok(!clean.includes("anthropicHeaderSecret42"));
    assert.ok(!clean.includes("querySecret99"));
    assert.ok(!clean.includes("jsonHeaderSecret77"));
    assert.ok(!clean.includes("mySuperSecretValue123"));
    assert.ok(!clean.includes("alice:s3cr3tPass"));
    assert.ok(!clean.includes("proxyUser:proxyPass"));
    assert.ok(!clean.includes("Marko"));
    assert.ok(!clean.includes("Ana"));
    assert.ok(!clean.includes("/home/alice/"));
    assert.ok(!clean.includes("/Users/elena/"));
    assert.ok(!clean.includes("/Users/stefan"));

    assert.ok(clean.includes("[REDACTED_API_KEY]"));
    assert.ok(clean.includes("[REDACTED_HF_TOKEN]"));
    assert.ok(clean.includes("[REDACTED_GH_TOKEN]"));
    assert.ok(clean.includes("Bearer [REDACTED_TOKEN]"));
    assert.ok(clean.includes("X-API-Key: [REDACTED]"));
    assert.ok(clean.includes("https://api.example.com/v1?x-api-key=[REDACTED]&model=gemini"));
    assert.ok(clean.includes('{"x-api-key": "[REDACTED]", "ok": true}'));
    assert.ok(clean.includes("https://<redacted>@api.example.com/v1/chat"));
    assert.ok(clean.includes("socks5://<redacted>@192.168.1.5:1080"));
    assert.ok(clean.includes("https://example.com/help?contact=support@example.com"));
    assert.ok(clean.includes("C:\\Users\\<USER>\\AppData"));
    assert.ok(clean.includes("D:/Users/<USER>/Videos"));
    assert.ok(clean.includes("/home/<USER>/Projects"));
    assert.ok(clean.includes("/Users/<USER>/Movies/interview.mov"));
    assert.ok(clean.includes("HOME=/Users/<USER>"));

    // Idempotency check
    assert.equal(sanitizeText(clean), clean);
  });

  it("neutralizes @username and @org/team mentions without breaking emails or normal text", () => {
    const raw =
      "@octocat please check cc @torvalds, (@alice), [@org/security-team] — email user@example.com or john.doe+tag@sub.example.org; version @ 2026.";
    const neutralized = neutralizeGitHubMentions(raw);

    assert.ok(!/(^|[^A-Za-z0-9_])@[A-Za-z0-9]/.test(neutralized));
    assert.ok(neutralized.includes("@\u200Boctocat"));
    assert.ok(neutralized.includes("@\u200Btorvalds"));
    assert.ok(neutralized.includes("(@\u200Balice)"));
    assert.ok(neutralized.includes("[@\u200Borg/security-team]"));
    assert.ok(neutralized.includes("user@example.com"));
    assert.ok(neutralized.includes("john.doe+tag@sub.example.org"));
    assert.ok(neutralized.includes("version @ 2026."));

    // Idempotency check
    assert.equal(neutralizeGitHubMentions(neutralized), neutralized);
  });
});

describe("POST /v1/report/bug", () => {
  it("creates a sanitized GitHub issue and returns 201", async () => {
    const calls = installMockFetch((call) => {
      assert.equal(call.method, "POST");
      assert.equal(call.url, "https://api.github.com/repos/Gruya13/EasySRT-Studio/issues");
      assert.equal(call.headers.get("User-Agent"), "EasySRT-Telemetry-Worker");
      assert.equal(call.headers.get("Authorization"), `Bearer ${TEST_ENV.GITHUB_ISSUES_TOKEN}`);
      assert.equal(call.headers.get("Accept"), "application/vnd.github+json");

      return new Response(
        JSON.stringify({
          number: 101,
          html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/101",
        }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      );
    });

    const payload = {
      appVersion: "0.3.0",
      title: "Export fails for path C:\\Users\\Petar\\video.mp4",
      description:
        "When exporting SRT with token sk-live_secretkey9999 and ghp_SecretToken12345 it failed.",
      steps: "1. Open /home/petar/test.mp4\n2. Click Export",
      contact: "user@example.com",
      platform: "windows-x64",
      gpu: "NVIDIA GeForce RTX 3060 (12288 MiB, driver 560.94)",
      diagnostics:
        "Authorization: Bearer tok.abc_123\ngithub_pat_11XYZ_987654\n</details>\n```boom```",
    };

    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
    const json = (await res.json()) as {
      ok: boolean;
      deduplicated: boolean;
      issueNumber: number;
      issueUrl: string;
    };
    assert.deepEqual(json, {
      ok: true,
      deduplicated: false,
      issueNumber: 101,
      issueUrl: "https://github.com/Gruya13/EasySRT-Studio/issues/101",
    });

    assert.equal(calls.length, 1);
    const ghBody = JSON.parse(calls[0].bodyText ?? "{}") as {
      title: string;
      labels: string[];
      body: string;
    };

    assert.equal(ghBody.title, "[Bug v0.3.0] Export fails for path C:\\Users\\<USER>\\video.mp4");
    assert.deepEqual(ghBody.labels, ["bug"]);
    assert.ok(ghBody.body.includes("<details>\n<summary>System Diagnostics & Logs</summary>"));
    assert.ok(ghBody.body.includes("user@example.com"));
    assert.ok(!ghBody.body.includes("sk-live_secretkey9999"));
    assert.ok(!ghBody.body.includes("ghp_SecretToken12345"));
    assert.ok(!ghBody.body.includes("tok.abc_123"));
    assert.ok(!ghBody.body.includes("github_pat_11XYZ_987654"));
    assert.ok(!ghBody.body.includes("/home/petar/"));
    assert.ok(!ghBody.body.includes("Petar"));
    assert.ok(ghBody.body.includes("<\\/details>"));
  });

  it("neutralizes @mentions across bug title, description, steps, contact, and logs before GitHub API", async () => {
    const calls = installMockFetch(() =>
      new Response(
        JSON.stringify({
          number: 102,
          html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/102",
        }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      ),
    );

    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        appVersion: "0.4.0",
        title: "Crash when @octocat exports /Users/elena/clip.mp4",
        description:
          `Ping @torvalds and @org/maintainers. Also hf_TokenAbCdEf123456 and ${SYNTHETIC_GEMINI_KEY}`,
        steps: "1. Notify @alice\n2. Check https://bob:secret@api.example.com/v1",
        contact: "reporter@example.com (@reporter)",
        diagnostics: "LOG: @maintainer x-api-key: secretHeader999",
      }),
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);

    const ghBody = JSON.parse(calls[0].bodyText ?? "{}") as {
      title: string;
      body: string;
    };

    assert.ok(!ghBody.title.includes("@octocat"));
    assert.ok(ghBody.title.includes("@\u200Boctocat"));
    assert.ok(ghBody.title.includes("/Users/<USER>/clip.mp4"));
    assert.ok(!ghBody.body.includes("@torvalds"));
    assert.ok(!ghBody.body.includes("@org/maintainers"));
    assert.ok(!ghBody.body.includes("@alice"));
    assert.ok(!ghBody.body.includes("@reporter"));
    assert.ok(!ghBody.body.includes("@maintainer"));
    assert.ok(ghBody.body.includes("@\u200Btorvalds"));
    assert.ok(ghBody.body.includes("@\u200Borg/maintainers"));
    assert.ok(ghBody.body.includes("@\u200Balice"));
    assert.ok(ghBody.body.includes("reporter@example.com"));
    assert.ok(!ghBody.body.includes("hf_TokenAbCdEf123456"));
    assert.ok(!ghBody.body.includes(SYNTHETIC_GEMINI_KEY));
    assert.ok(!ghBody.body.includes("bob:secret"));
    assert.ok(!ghBody.body.includes("secretHeader999"));
  });

  it("returns 400 on invalid bug report inputs without calling GitHub", async () => {
    const calls = installMockFetch(() => new Response("should not be called", { status: 500 }));

    const invalidCases: Array<{ label: string; body: string }> = [
      { label: "empty body", body: "" },
      { label: "invalid JSON", body: "{not json" },
      {
        label: "title too short",
        body: JSON.stringify({ appVersion: "0.3.0", title: "ab", description: "Valid description" }),
      },
      {
        label: "title too long",
        body: JSON.stringify({
          appVersion: "0.3.0",
          title: "x".repeat(181),
          description: "Valid description",
        }),
      },
      {
        label: "description too short",
        body: JSON.stringify({ appVersion: "0.3.0", title: "Valid title", description: "1234" }),
      },
      {
        label: "description too long",
        body: JSON.stringify({
          appVersion: "0.3.0",
          title: "Valid title",
          description: "d".repeat(8001),
        }),
      },
      {
        label: "missing appVersion",
        body: JSON.stringify({ title: "Valid title", description: "Valid description" }),
      },
      {
        label: "non-string optional steps",
        body: JSON.stringify({
          appVersion: "0.3.0",
          title: "Valid title",
          description: "Valid description",
          steps: 42,
        }),
      },
    ];

    for (const tc of invalidCases) {
      const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: tc.body,
      });
      const res = await worker.fetch(req, TEST_ENV);
      assert.equal(res.status, 400, `Expected 400 for case: ${tc.label}`);
      const json = (await res.json()) as { ok: boolean; error: string };
      assert.equal(json.ok, false);
      assert.ok(json.error.length > 0);
    }

    assert.equal(calls.length, 0);
  });
});

describe("POST /v1/report/crash", () => {
  it("creates a new crash issue (201) when no open matching fingerprint exists", async () => {
    const calls = installMockFetch((call) => {
      if (call.method === "GET" && call.url.startsWith("https://api.github.com/search/issues")) {
        return new Response(JSON.stringify({ total_count: 0, items: [] }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      if (call.method === "POST" && call.url.endsWith("/issues")) {
        return new Response(
          JSON.stringify({
            number: 205,
            html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/205",
          }),
          { status: 201, headers: { "Content-Type": "application/json" } },
        );
      }
      throw new Error(`Unexpected fetch: ${call.method} ${call.url}`);
    });

    const req = new Request("https://telemetry.example.workers.dev/v1/report/crash", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        appVersion: "0.3.0",
        crashKind: "rust_panic",
        fingerprint: "a1b2c3d4e5f60708",
        message: "whisper-cli failed for @octocat with status 0xC0000135 at C:\\Users\\Luka\\bin",
        stackTrace: "0: pipeline::whisper::run_whisper_cli (/home/luka/src/whisper.rs:42) @dev",
        source: "whisper.rs",
        platform: "windows-x64",
        gpu: "NVIDIA RTX 4060",
        diagnostics: "last log line with sk-secret12345678 and @team/ops /Users/maya/app.log",
      }),
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
    const json = (await res.json()) as {
      ok: boolean;
      deduplicated: boolean;
      issueNumber: number;
      issueUrl: string;
    };
    assert.deepEqual(json, {
      ok: true,
      deduplicated: false,
      issueNumber: 205,
      issueUrl: "https://github.com/Gruya13/EasySRT-Studio/issues/205",
    });

    assert.equal(calls.length, 2);
    assert.ok(calls[0].url.includes("crash-fp%3Aa1b2c3d4e5f60708"));
    const created = JSON.parse(calls[1].bodyText ?? "{}") as {
      title: string;
      labels: string[];
      body: string;
    };
    assert.ok(created.title.startsWith("[Crash v0.3.0] rust_panic: "));
    assert.ok(!created.title.includes("@octocat"));
    assert.ok(created.title.includes("@\u200Boctocat"));
    assert.deepEqual(created.labels, ["crash-report"]);
    assert.ok(created.body.includes("<!-- crash-fp:a1b2c3d4e5f60708 -->"));
    assert.ok(created.body.includes("<details>\n<summary>System Diagnostics & Logs</summary>"));
    assert.ok(!created.body.includes("Luka"));
    assert.ok(!created.body.includes("/home/luka/"));
    assert.ok(!created.body.includes("/Users/maya/"));
    assert.ok(!created.body.includes("sk-secret12345678"));
    assert.ok(!created.body.includes("@octocat"));
    assert.ok(!created.body.includes("@dev"));
    assert.ok(!created.body.includes("@team/ops"));
    assert.ok(created.body.includes("/Users/<USER>/app.log"));
  });

  it("deduplicates by commenting on an existing open crash issue and returns 200", async () => {
    const calls = installMockFetch((call) => {
      if (call.method === "GET" && call.url.startsWith("https://api.github.com/search/issues")) {
        return new Response(
          JSON.stringify({
            total_count: 1,
            items: [
              {
                number: 77,
                state: "open",
                html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/77",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (call.method === "POST" && call.url.endsWith("/issues/77/comments")) {
        return new Response(JSON.stringify({ id: 9001 }), {
          status: 201,
          headers: { "Content-Type": "application/json" },
        });
      }
      throw new Error(`Unexpected fetch: ${call.method} ${call.url}`);
    });

    const req = new Request("https://telemetry.example.workers.dev/v1/report/crash", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        appVersion: "0.3.0",
        crashKind: "frontend_error",
        fingerprint: "deadbeef12345678",
        message: "Cannot read properties of undefined (@octocat)",
        platform: "windows-x64",
        gpu: "NVIDIA GTX 1660 Super",
        diagnostics: "log excerpt ghp_HiddenToken999 @maintainer",
      }),
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get("Access-Control-Allow-Origin"), null);
    const json = (await res.json()) as {
      ok: boolean;
      deduplicated: boolean;
      issueNumber: number;
      issueUrl: string;
    };
    assert.deepEqual(json, {
      ok: true,
      deduplicated: true,
      issueNumber: 77,
      issueUrl: "https://github.com/Gruya13/EasySRT-Studio/issues/77",
    });

    assert.equal(calls.length, 2);
    assert.equal(
      calls[1].url,
      "https://api.github.com/repos/Gruya13/EasySRT-Studio/issues/77/comments",
    );
    const commentPayload = JSON.parse(calls[1].bodyText ?? "{}") as { body: string };
    assert.ok(commentPayload.body.includes("0.3.0"));
    assert.ok(commentPayload.body.includes("NVIDIA GTX 1660 Super"));
    assert.ok(commentPayload.body.includes("<details>\n<summary>System Diagnostics & Logs</summary>"));
    assert.ok(!commentPayload.body.includes("ghp_HiddenToken999"));
    assert.ok(!commentPayload.body.includes("@octocat"));
    assert.ok(!commentPayload.body.includes("@maintainer"));
    assert.ok(commentPayload.body.includes("@\u200Boctocat"));
  });

  it("rejects invalid crash fingerprints with 400", async () => {
    const calls = installMockFetch(() => new Response("unexpected", { status: 500 }));
    for (const badFp of ["short", "bad-chars-!!!", "a".repeat(65)]) {
      const req = new Request("https://telemetry.example.workers.dev/v1/report/crash", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          appVersion: "0.3.0",
          crashKind: "rust_panic",
          fingerprint: badFp,
          message: "Something crashed",
        }),
      });
      const res = await worker.fetch(req, TEST_ENV);
      assert.equal(res.status, 400, `Expected 400 for fingerprint '${badFp}'`);
    }
    assert.equal(calls.length, 0);
  });
});

describe("Bounded Body Reader (>128 KiB rejection)", () => {
  it("rejects streamed payloads exceeding 128 KiB with HTTP 413 without Content-Length", async () => {
    let chunksProduced = 0;
    const oversizedStream = new ReadableStream<Uint8Array>({
      pull(controller) {
        chunksProduced += 1;
        // 32 KiB chunks: 5 chunks = 160 KiB > 128 KiB
        controller.enqueue(new Uint8Array(32_768).fill(65));
        if (chunksProduced >= 20) {
          controller.close();
        }
      },
    });

    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: oversizedStream,
      // @ts-ignore Node fetch requires duplex for ReadableStream request bodies
      duplex: "half",
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 413);
    const json = (await res.json()) as { ok: boolean; error: string };
    assert.equal(json.ok, false);
    assert.ok(chunksProduced <= 6, `Stream should abort early on cap (produced ${chunksProduced})`);
  });

  it("rejects payloads with Content-Length > 131072 immediately with HTTP 413", async () => {
    const req = new Request("https://telemetry.example.workers.dev/v1/report/crash", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Content-Length": String(MAX_BODY_BYTES + 1),
      },
      body: "{}",
    });

    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 413);
  });

  it("accepts a valid payload at exactly 131072 bytes", async () => {
    installMockFetch(() =>
      new Response(
        JSON.stringify({
          number: 300,
          html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/300",
        }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      ),
    );

    const basePayload = {
      appVersion: "0.3.0",
      title: "Boundary size test",
      description: "Testing exact 128 KiB boundary payload",
      diagnostics: "",
    };
    const overhead = new TextEncoder().encode(JSON.stringify(basePayload)).byteLength;
    const exactPayload = JSON.stringify({
      ...basePayload,
      diagnostics: "x".repeat(MAX_BODY_BYTES - overhead),
    });
    assert.equal(new TextEncoder().encode(exactPayload).byteLength, MAX_BODY_BYTES);

    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: exactPayload,
    });
    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 201);
  });
});

describe("Rate Limiting via caches.default", () => {
  it("enforces 5 bug reports per 10 minutes per IP and returns 429 on exceed", async () => {
    (globalThis as { caches?: { default: Cache } }).caches = {
      default: createInMemoryCache(),
    };

    installMockFetch(() =>
      new Response(
        JSON.stringify({
          number: 400,
          html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/400",
        }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      ),
    );

    const makeReq = (ip: string) =>
      new Request("https://telemetry.example.workers.dev/v1/report/bug", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "CF-Connecting-IP": ip,
        },
        body: JSON.stringify({
          appVersion: "0.3.0",
          title: "Rate limit bug test",
          description: "Testing per-IP rate limiting behavior",
        }),
      });

    for (let i = 0; i < 5; i++) {
      const res = await worker.fetch(makeReq("203.0.113.10"), TEST_ENV);
      assert.equal(res.status, 201, `Request ${i + 1} should succeed`);
    }

    const blocked = await worker.fetch(makeReq("203.0.113.10"), TEST_ENV);
    assert.equal(blocked.status, 429);
    assert.ok(Number(blocked.headers.get("Retry-After")) >= 1);

    // Different IP is unaffected
    const otherIp = await worker.fetch(makeReq("203.0.113.11"), TEST_ENV);
    assert.equal(otherIp.status, 201);
  });

  it("enforces 15 crash reports per 10 minutes per IP and returns 429 on exceed", async () => {
    (globalThis as { caches?: { default: Cache } }).caches = {
      default: createInMemoryCache(),
    };

    installMockFetch((call) => {
      if (call.method === "GET") {
        return new Response(JSON.stringify({ total_count: 0, items: [] }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response(
        JSON.stringify({
          number: 450,
          html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/450",
        }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      );
    });

    const makeCrashReq = (ip: string) =>
      new Request("https://telemetry.example.workers.dev/v1/report/crash", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "CF-Connecting-IP": ip,
        },
        body: JSON.stringify({
          appVersion: "0.3.0",
          crashKind: "subprocess_crash",
          fingerprint: "11223344aabbccdd",
          message: "llama-server exited unexpectedly",
        }),
      });

    for (let i = 0; i < 15; i++) {
      const res = await worker.fetch(makeCrashReq("198.51.100.7"), TEST_ENV);
      assert.equal(res.status, 201, `Crash request ${i + 1} should succeed`);
    }

    const blocked = await worker.fetch(makeCrashReq("198.51.100.7"), TEST_ENV);
    assert.equal(blocked.status, 429);
    assert.ok(Number(blocked.headers.get("Retry-After")) >= 1);
  });
});

describe("Configuration & Upstream Error Handling", () => {
  it("returns 500 when GITHUB_ISSUES_TOKEN is missing without leaking details", async () => {
    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        appVersion: "0.3.0",
        title: "Missing secret check",
        description: "Ensures unconfigured worker returns 500 cleanly",
      }),
    });
    const res = await worker.fetch(req, {
      GITHUB_REPO: "Gruya13/EasySRT-Studio",
      GITHUB_ISSUES_TOKEN: "",
    });
    assert.equal(res.status, 500);
    const json = (await res.json()) as { ok: boolean; error: string };
    assert.equal(json.ok, false);
  });

  it("returns 502 when GitHub API rejects issue creation", async () => {
    installMockFetch(() => new Response("rate limited", { status: 403 }));
    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        appVersion: "0.3.0",
        title: "Upstream failure check",
        description: "Ensures GitHub API error surfaces as 502",
      }),
    });
    const res = await worker.fetch(req, TEST_ENV);
    assert.equal(res.status, 502);
    const json = (await res.json()) as { ok: boolean; error: string };
    assert.equal(json.ok, false);
  });
});

function buildSampleExportMetrics(overrides: Record<string, unknown> = {}) {
  return {
    totalBaselineCues: 40,
    totalFinalCues: 41,
    unchangedCuesCount: 30,
    textEditedCuesCount: 10,
    editRatePct: 25.0,
    initialHighCpsCount: 3,
    finalHighCpsCount: 0,
    initialLineTooLongCount: 2,
    finalLineTooLongCount: 0,
    avgCpsBefore: 16.4,
    avgCpsAfter: 14.1,
    condenseCpsUsed: false,
    encliticReorderedCount: 2,
    redundantPossessiveRemovedCount: 1,
    passiveCalqueRemovedCount: 1,
    conjunctionCommaFixedCount: 1,
    missedFillerDeletedCount: 2,
    inflectionSuffixFixCount: 1,
    boundaryOrphanShiftedCount: 0,
    splitCount: 1,
    splitAfterCommaCount: 1,
    splitAfterConjunctionCount: 0,
    avgPreSplitChars: 74,
    avgPreSplitDurationMs: 5200,
    mergeCount: 0,
    avgMergedShortCueDurationMs: 0,
    avgMergedShortCueChars: 0,
    avgMergedGapMs: 0,
    startTrimCount: 2,
    avgStartDeltaMs: -50,
    endTrimCount: 3,
    avgEndDeltaMs: 120,
    ...overrides,
  };
}

describe("Zero-IP Rate Limiting & Subnet Prefix Extraction (V05-H1 / V05-M2)", () => {
  it("hashes CF-Connecting-IP before constructing Cache API keys (no raw IP in key)", async () => {
    const cacheKeys: string[] = [];
    (globalThis as { caches?: { default: Cache } }).caches = {
      default: createInMemoryCache(cacheKeys),
    };
    installMockFetch(() =>
      new Response(
        JSON.stringify({ number: 501, html_url: "https://github.com/Gruya13/EasySRT-Studio/issues/501" }),
        { status: 201, headers: { "Content-Type": "application/json" } },
      ),
    );

    const secretIp = "203.0.113.77";
    const req = new Request("https://telemetry.example.workers.dev/v1/report/bug", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "CF-Connecting-IP": secretIp,
      },
      body: JSON.stringify({
        appVersion: "0.5.0",
        title: "Zero IP cache check",
        description: "Verify raw IP never appears in Cache API URL",
      }),
    });

    const res = await worker.fetch(req, { ...TEST_ENV, SUBNET_HMAC_SECRET: "unit-test-hmac-secret" });
    assert.equal(res.status, 201);
    assert.ok(cacheKeys.length >= 2);
    for (const key of cacheKeys) {
      assert.ok(!key.includes(secretIp), `Raw IP leaked into cache key: ${key}`);
      assert.match(key, /\/[0-9a-f]{64}\/\d+$/);
    }
  });

  it("extracts /24 IPv4, IPv4-mapped IPv6, and /48 IPv6 subnet prefixes accurately", () => {
    assert.equal(extractSubnetPrefix("203.0.113.42"), "ipv4:203.0.113.0/24");
    assert.equal(extractSubnetPrefix("::ffff:198.51.100.99"), "ipv4:198.51.100.0/24");
    assert.equal(extractSubnetPrefix("2001:db8:abcd:0001::1"), "ipv6:2001:0db8:abcd::/48");
    assert.equal(extractSubnetPrefix("2001:db8:abcd:ffff::99"), "ipv6:2001:0db8:abcd::/48");
    assert.equal(extractSubnetPrefix("2001:db8:1234::1"), "ipv6:2001:0db8:1234::/48");
    assert.equal(extractSubnetPrefix("unknown"), "unknown");
  });
});

describe("GET /v1/update/latest.json (V05-M1)", () => {
  it("caches upstream latest.json under a fixed query-free key, returns exact bytes, and allowlists D1 DAU logging", async () => {
    const cacheKeys: string[] = [];
    (globalThis as { caches?: { default: Cache } }).caches = {
      default: createInMemoryCache(cacheKeys),
    };
    const { db, sqlite } = createTestD1();
    const env: Env = { ...TEST_ENV, DB: db };

    // Deliberately non-minified JSON formatting + minisign signature to verify byte-for-byte preservation.
    const exactManifestBytes =
      '{\n  "version": "0.5.0",\n  "notes": "Wave 2 release",\n  "platforms": {\n    "windows-x86_64": {\n      "signature": "RWQ_EXACT_MINISIGN_BYTES==",\n      "url": "https://github.com/Gruya13/EasySRT-Studio/releases/download/v0.5.0/setup.exe"\n    }\n  }\n}\n';

    const calls = installMockFetch((call) => {
      assert.equal(call.url, "https://github.com/Gruya13/EasySRT-Studio/releases/latest/download/latest.json");
      return new Response(exactManifestBytes, {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });

    const { ctx: ctx1, flush: flush1 } = createTestExecutionContext();
    const res1 = await worker.fetch(
      new Request(
        "https://telemetry.example.workers.dev/v1/update/latest.json?target=windows&arch=x86_64&current=0.4.0",
      ),
      env,
      ctx1,
    );
    await flush1();
    assert.equal(res1.status, 200);
    assert.equal(res1.headers.get("X-Cache"), "MISS");
    assert.equal(res1.headers.get("Cache-Control"), "public, max-age=300");
    assert.equal(await res1.text(), exactManifestBytes);

    // Second request with different query params must hit the SAME fixed query-free cache key!
    const { ctx: ctx2, flush: flush2 } = createTestExecutionContext();
    const res2 = await worker.fetch(
      new Request(
        "https://telemetry.example.workers.dev/v1/update/latest.json?target=linux&arch=aarch64&current=v0.4.1",
      ),
      env,
      ctx2,
    );
    await flush2();
    assert.equal(res2.status, 200);
    assert.equal(res2.headers.get("X-Cache"), "HIT");
    assert.equal(await res2.text(), exactManifestBytes);
    assert.equal(calls.length, 1, "Upstream GitHub must only be fetched once during 300s TTL");

    const updateCacheKeys = cacheKeys.filter((k) => k.includes("update-cache.easysrt.internal"));
    assert.ok(updateCacheKeys.length >= 2);
    for (const k of updateCacheKeys) {
      assert.equal(k, "https://update-cache.easysrt.internal/v1/update/latest.json");
    }

    // Invalid target/arch/semver must NEVER write poisoned rows into D1.
    const { ctx: ctx3, flush: flush3 } = createTestExecutionContext();
    const res3 = await worker.fetch(
      new Request(
        "https://telemetry.example.workers.dev/v1/update/latest.json?target=evil&arch=x86_64&current=not-semver",
      ),
      env,
      ctx3,
    );
    await flush3();
    assert.equal(res3.status, 200);

    const rows = (
      sqlite
        .prepare("SELECT target, arch, current_version, check_count FROM daily_update_checks ORDER BY target")
        .all() as Array<Record<string, unknown>>
    ).map((r) => ({ ...r }));
    assert.deepEqual(rows, [
      { target: "linux", arch: "aarch64", current_version: "0.4.1", check_count: 1 },
      { target: "windows", arch: "x86_64", current_version: "0.4.0", check_count: 1 },
    ]);
  });

  it("rejects malformed or oversized (>64 KiB) upstream manifests with 502 and blocks Origin with 403", async () => {
    const { db } = createTestD1();
    const env: Env = { ...TEST_ENV, DB: db };

    const originRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/update/latest.json", {
        headers: { Origin: "https://evil.example" },
      }),
      env,
    );
    assert.equal(originRes.status, 403);

    installMockFetch(() =>
      new Response(JSON.stringify({ version: "0.5.0" }), { status: 200 }),
    );
    const missingPlatforms = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/update/latest.json"),
      env,
    );
    assert.equal(missingPlatforms.status, 502);

    installMockFetch(() =>
      new Response("x".repeat(70_000), { status: 200 }),
    );
    const oversized = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/update/latest.json"),
      env,
    );
    assert.equal(oversized.status, 502);
  });
});

describe("Telemetry Ingestion, Sybil Guard (V05-M2), Admin Digest & Cron Purge", () => {
  it("records Level 1 job metrics and enforces the 32 KiB cap", async () => {
    const { db, sqlite } = createTestD1();
    const env: Env = { ...TEST_ENV, DB: db };

    const validRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/telemetry/job-metrics", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          appVersion: "0.5.0",
          os: "linux",
          gpuTier: "vram8Gb",
          whisperBackend: "vulkan",
          whisperModelId: "whisper-large-v3-turbo-q5_0",
          llmProvider: "deepSeek",
          llmModelId: "deepseek-chat",
          audioDurationMs: 120_000,
          whisperElapsedMs: 10_000,
          diarizerElapsedMs: 800,
          llmElapsedMs: 6_500,
          asrRtf: 0.083,
          llmRtf: 0.054,
          oomCpuFallback: false,
          batchSubdivisionRetries: 1,
          cueCount: 35,
          sourceLang: "en",
          targetLang: "sr",
          translationStyle: "natural",
          addressForm: "auto",
          preset: "standard",
        }),
      }),
      env,
    );
    assert.equal(validRes.status, 201);

    const row = {
      ...(sqlite
        .prepare("SELECT event_kind, gpu_tier, whisper_backend, oom_cpu_fallback FROM job_and_edit_metrics")
        .get() as Record<string, unknown>),
    };
    assert.deepEqual(row, {
      event_kind: "job",
      gpu_tier: "vram8Gb",
      whisper_backend: "vulkan",
      oom_cpu_fallback: 0,
    });

    const tooLargeRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/telemetry/job-metrics", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": String(MAX_TELEMETRY_BODY_BYTES + 1),
        },
        body: "{}",
      }),
      env,
    );
    assert.equal(tooLargeRes.status, 413);
  });

  it("rejects unsafe n-gram diffs containing uppercase, digits, or symbols", async () => {
    const { db } = createTestD1();
    const env: Env = { ...TEST_ENV, DB: db, SUBNET_HMAC_SECRET: "sybil-secret" };

    for (const badDiff of [
      { langPair: "en-sr", oldSpan: "Marko", newSpan: "petar", cohortProof: "a".repeat(24) },
      { langPair: "en-sr", oldSpan: "godina 2026", newSpan: "letos", cohortProof: "b".repeat(24) },
      { langPair: "en-sr", oldSpan: "user@example.com", newSpan: "mejl", cohortProof: "c".repeat(24) },
      { langPair: "en-sr", oldSpan: "jedan dva tri cetiri", newSpan: "pet", cohortProof: "d".repeat(24) },
    ]) {
      const res = await worker.fetch(
        new Request("https://telemetry.example.workers.dev/v1/telemetry/export", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "CF-Connecting-IP": "198.51.100.10",
          },
          body: JSON.stringify({
            appVersion: "0.5.0",
            os: "windows",
            sourceLang: "en",
            targetLang: "sr",
            translationStyle: "natural",
            addressForm: "auto",
            metrics: buildSampleExportMetrics(),
            ngramDiffs: [badDiff],
          }),
        }),
        env,
      );
      assert.equal(res.status, 400, `Expected 400 for unsafe n-gram ${badDiff.oldSpan}`);
    }
  });

  it("enforces K>=5 cohorts AND >=3 distinct /24 or /48 subnets (Sybil Guard) before exposing in /v1/admin/digest", async () => {
    const { db } = createTestD1();
    const env: Env = {
      ...TEST_ENV,
      DB: db,
      SUBNET_HMAC_SECRET: "test-sybil-hmac-secret",
      TELEMETRY_ADMIN_TOKEN: "super-secret-admin-token-42",
    };

    const sendExport = (ip: string, ngramDiffs: Array<{ langPair: string; oldSpan: string; newSpan: string; cohortProof: string }>) =>
      worker.fetch(
        new Request("https://telemetry.example.workers.dev/v1/telemetry/export", {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "CF-Connecting-IP": ip,
          },
          body: JSON.stringify({
            appVersion: "0.5.0",
            os: "windows",
            sourceLang: "en",
            targetLang: "sr",
            translationStyle: "natural",
            addressForm: "auto",
            metrics: buildSampleExportMetrics(),
            ngramDiffs,
          }),
        }),
        env,
      );

    // 1. Sybil cluster: 5 distinct cohortProofs all from the SAME /24 IPv4 subnet (198.51.100.x).
    for (let i = 1; i <= 5; i++) {
      const res = await sendExport(`198.51.100.${i}`, [
        {
          langPair: "en-sr",
          oldSpan: "sybil fraza",
          newSpan: "lazna zamena",
          cohortProof: `${String(i).repeat(24)}`,
        },
      ]);
      assert.equal(res.status, 201);
    }

    // 2. IPv6 /48 Sybil cluster: 5 distinct cohorts from only 2 distinct /48 prefixes.
    const ipv6Ips = [
      "2001:db8:aaaa:0001::1",
      "2001:db8:aaaa:0002::1",
      "2001:db8:aaaa:0003::1",
      "2001:db8:bbbb:0001::1",
      "2001:db8:bbbb:0002::1",
    ];
    for (let i = 0; i < ipv6Ips.length; i++) {
      const res = await sendExport(ipv6Ips[i], [
        {
          langPair: "en-sr",
          oldSpan: "mreza nedovoljno",
          newSpan: "samo dve podmreze",
          cohortProof: `a${String(i + 1).repeat(23)}`,
        },
      ]);
      assert.equal(res.status, 201);
    }

    // 3. Legitimate organic phrase: 5 distinct cohorts across 3 distinct /24 subnets + 1 replay.
    const organicSubmissions = [
      { ip: "192.0.2.10", proof: "f1".repeat(12) },
      { ip: "192.0.2.10", proof: "f1".repeat(12) }, // duplicate replay must be ignored
      { ip: "192.0.2.25", proof: "f2".repeat(12) },
      { ip: "198.51.100.50", proof: "f3".repeat(12) },
      { ip: "203.0.113.11", proof: "f4".repeat(12) },
      { ip: "203.0.113.99", proof: "f5".repeat(12) },
    ];
    for (const sub of organicSubmissions) {
      const res = await sendExport(sub.ip, [
        {
          langPair: "en-sr",
          oldSpan: "tijekom",
          newSpan: "tokom",
          cohortProof: sub.proof,
        },
      ]);
      assert.equal(res.status, 201);
    }

    // Unauthorized digest requests must fail with 401.
    const unauthRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/admin/digest"),
      env,
    );
    assert.equal(unauthRes.status, 401);

    const wrongTokenRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/admin/digest", {
        headers: { Authorization: "Bearer wrong-token" },
      }),
      env,
    );
    assert.equal(wrongTokenRes.status, 401);

    // Authorized digest must expose ONLY "tijekom -> tokom" (5 cohorts, 3 subnets).
    const digestRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/admin/digest?days=999", {
        headers: { Authorization: "Bearer super-secret-admin-token-42" },
      }),
      env,
    );
    assert.equal(digestRes.status, 200);
    const digest = (await digestRes.json()) as {
      ok: boolean;
      windowDays: number;
      promotedNgrams: Array<{
        lang_pair: string;
        old_span: string;
        new_span: string;
        cohort_count: number;
        subnet_count: number;
      }>;
    };
    assert.equal(digest.ok, true);
    assert.equal(digest.windowDays, 365);
    assert.equal(digest.promotedNgrams.length, 1);
    assert.equal(digest.promotedNgrams[0].old_span, "tijekom");
    assert.equal(digest.promotedNgrams[0].new_span, "tokom");
    assert.equal(digest.promotedNgrams[0].cohort_count, 5);
    assert.equal(digest.promotedNgrams[0].subnet_count, 3);
  });

  it("sanitizes Level 3 training pairs, enforces 64 KiB / 25-pair limits, and exports JSONL", async () => {
    const { db } = createTestD1();
    const env: Env = {
      ...TEST_ENV,
      DB: db,
      TELEMETRY_ADMIN_TOKEN: "admin-token-jsonl",
    };

    const tooManyPairs = Array.from({ length: 26 }, () => ({
      sourceLang: "en",
      targetLang: "sr",
      style: "natural",
      addressForm: "auto",
      sourceText: "Hello",
      initialAiText: "Zdravo",
      finalHumanText: "Ćao",
    }));
    const reject26 = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/telemetry/training-pairs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ appVersion: "0.5.0", pairs: tooManyPairs }),
      }),
      env,
    );
    assert.equal(reject26.status, 400);

    const oversized = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/telemetry/training-pairs", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": String(MAX_TRAINING_PAIRS_BODY_BYTES + 1),
        },
        body: "{}",
      }),
      env,
    );
    assert.equal(oversized.status, 413);

    const saveRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/telemetry/training-pairs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          appVersion: "0.5.0",
          pairs: [
            {
              sourceLang: "en",
              targetLang: "sr",
              style: "natural",
              addressForm: "politeVi",
              sourceText: "Check /home/marko/doc.txt with sk-secret12345678",
              initialAiText: "Proveri sa ključem sk-secret12345678",
              finalHumanText: "Proverite dokument sa [REDACTED_API_KEY]",
            },
          ],
        }),
      }),
      env,
    );
    assert.equal(saveRes.status, 201);

    const jsonlRes = await worker.fetch(
      new Request("https://telemetry.example.workers.dev/v1/admin/digest?format=jsonl", {
        headers: { Authorization: "Bearer admin-token-jsonl" },
      }),
      env,
    );
    assert.equal(jsonlRes.status, 200);
    assert.equal(jsonlRes.headers.get("Content-Type"), "application/x-ndjson; charset=utf-8");
    const lines = (await jsonlRes.text()).trim().split("\n");
    assert.equal(lines.length, 1);
    const parsed = JSON.parse(lines[0]) as { sourceText: string; initialAiText: string };
    assert.ok(!parsed.sourceText.includes("marko"));
    assert.ok(!parsed.sourceText.includes("sk-secret12345678"));
    assert.ok(!parsed.initialAiText.includes("sk-secret12345678"));
  });

  it("purges unpromoted n-grams older than 30 days and training pairs older than 90 days in scheduled()", async () => {
    const { db, sqlite } = createTestD1();
    const env: Env = { ...TEST_ENV, DB: db };
    const nowMs = Date.parse("2026-11-27T03:00:00Z");
    const dayMs = 86_400_000;
    const old40d = new Date(nowMs - 40 * dayMs).toISOString();
    const fresh10d = new Date(nowMs - 10 * dayMs).toISOString();
    const old100d = new Date(nowMs - 100 * dayMs).toISOString();

    sqlite
      .prepare(
        `INSERT INTO ngram_candidates (pair_hash, lang_pair, old_span, new_span, cohort_count, subnet_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run("hash_old_unpromoted", "en-sr", "retko", "drugo", 3, 2, old40d, old40d);
    sqlite
      .prepare(
        `INSERT INTO ngram_cohorts (pair_hash, cohort_proof, subnet_hash, created_at)
         VALUES (?, ?, ?, ?)`,
      )
      .run("hash_old_unpromoted", "proof1", "sub1", old40d);
    sqlite
      .prepare(
        `INSERT INTO ngram_candidates (pair_hash, lang_pair, old_span, new_span, cohort_count, subnet_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run("hash_old_promoted", "en-sr", "cesto", "bolje", 7, 4, old40d, old40d);
    sqlite
      .prepare(
        `INSERT INTO ngram_candidates (pair_hash, lang_pair, old_span, new_span, cohort_count, subnet_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run("hash_fresh_unpromoted", "en-sr", "novo", "novije", 2, 2, fresh10d, fresh10d);

    sqlite
      .prepare(
        `INSERT INTO training_edit_pairs (created_at, app_version, source_lang, target_lang, style, address_form, source_text, initial_ai_text, final_human_text)
         VALUES (?, '0.5.0', 'en', 'sr', 'natural', 'auto', 'old', 'stari', 'novi')`,
      )
      .run(old100d);
    sqlite
      .prepare(
        `INSERT INTO training_edit_pairs (created_at, app_version, source_lang, target_lang, style, address_form, source_text, initial_ai_text, final_human_text)
         VALUES (?, '0.5.0', 'en', 'sr', 'natural', 'auto', 'fresh', 'sveze', 'bolje')`,
      )
      .run(fresh10d);

    await runScheduledCleanup(env, nowMs);

    const remainingCandidates = sqlite
      .prepare("SELECT pair_hash FROM ngram_candidates ORDER BY pair_hash")
      .all() as Array<{ pair_hash: string }>;
    assert.deepEqual(
      remainingCandidates.map((r) => r.pair_hash),
      ["hash_fresh_unpromoted", "hash_old_promoted"],
    );

    const remainingCohorts = sqlite
      .prepare("SELECT pair_hash FROM ngram_cohorts")
      .all() as Array<{ pair_hash: string }>;
    assert.equal(remainingCohorts.length, 0);

    const remainingPairs = (
      sqlite
        .prepare("SELECT source_text FROM training_edit_pairs")
        .all() as Array<Record<string, unknown>>
    ).map((r) => ({ ...r }));
    assert.deepEqual(remainingPairs, [{ source_text: "fresh" }]);
  });
});
