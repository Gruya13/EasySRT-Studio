// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";

import {
  BUG_REPORT_DESCRIPTION_MAX,
  BUG_REPORT_TITLE_MAX,
  MAX_SESSION_CRASH_REPORTS,
  buildCrashDedupeKey,
  isIgnoredBrowserError,
  normalizeUnhandledRejection,
  normalizeWindowErrorEvent,
  shouldSubmitSessionCrash,
  validateBugReportInput,
} from "../crashUtils.ts";

describe("Frontend Crash Normalization & Session Deduplication", () => {
  it("normalizes window error events with stack traces and source locations", () => {
    const err = new TypeError("Cannot read properties of undefined");
    const req = normalizeWindowErrorEvent({
      error: err,
      message: "Script error.",
      filename: "http://localhost:1420/src/App.tsx",
      lineno: 42,
      colno: 13,
    });

    assert.ok(req !== null);
    if (!req) return;
    assert.equal(req.crashKind, "window_error");
    assert.equal(req.message, "Cannot read properties of undefined");
    assert.ok(req.stackTrace && req.stackTrace.includes("TypeError"));
    assert.equal(req.source, "http://localhost:1420/src/App.tsx:42:13");
  });

  it("normalizes unhandled promise rejections across Error, string, object, and empty reasons", () => {
    const errReq = normalizeUnhandledRejection(new Error("IPC invoke timeout"));
    assert.ok(errReq !== null);
    if (!errReq) return;
    assert.equal(errReq.crashKind, "unhandled_rejection");
    assert.equal(errReq.message, "IPC invoke timeout");
    assert.equal(errReq.source, "window.onunhandledrejection");
    assert.ok(errReq.stackTrace);

    const strReq = normalizeUnhandledRejection("Backend unavailable");
    assert.ok(strReq !== null);
    if (!strReq) return;
    assert.equal(strReq.message, "Backend unavailable");
    assert.equal(strReq.stackTrace, null);

    const objReq = normalizeUnhandledRejection({ code: 503, detail: "busy" });
    assert.ok(objReq !== null);
    if (!objReq) return;
    assert.equal(objReq.message, '{"code":503,"detail":"busy"}');

    const emptyReq = normalizeUnhandledRejection(undefined);
    assert.ok(emptyReq !== null);
    if (!emptyReq) return;
    assert.equal(emptyReq.message, "Unhandled promise rejection");
  });

  it("ignores benign ResizeObserver browser notifications", () => {
    assert.equal(
      isIgnoredBrowserError("ResizeObserver loop limit exceeded"),
      true,
    );
    assert.equal(
      normalizeWindowErrorEvent({
        message:
          "ResizeObserver loop completed with undelivered notifications.",
      }),
      null,
    );
    assert.equal(
      normalizeUnhandledRejection("ResizeObserver loop limit exceeded"),
      null,
    );
  });

  it("deduplicates repeated crashes and enforces the 10-report per-session cap", () => {
    const seen = new Set<string>();
    const first = {
      crashKind: "window_error",
      message: "Boom",
      stackTrace: "Error: Boom\n    at foo (app.js:1:1)",
      source: "app.js:1:1",
    };

    assert.equal(
      buildCrashDedupeKey(first),
      "window_error::Boom::Error: Boom",
    );
    assert.equal(shouldSubmitSessionCrash(seen, first), true);
    assert.equal(shouldSubmitSessionCrash(seen, { ...first }), false);

    // Fill up to MAX_SESSION_CRASH_REPORTS (10)
    for (let i = 1; i < MAX_SESSION_CRASH_REPORTS; i += 1) {
      const unique = {
        crashKind: "unhandled_rejection",
        message: `Failure #${i}`,
        stackTrace: null,
        source: "window.onunhandledrejection",
      };
      assert.equal(shouldSubmitSessionCrash(seen, unique), true);
    }

    assert.equal(seen.size, MAX_SESSION_CRASH_REPORTS);

    // 11th distinct crash must be dropped by the session cap
    const overflow = {
      crashKind: "window_error",
      message: "Overflow crash",
      stackTrace: null,
      source: "window.onerror",
    };
    assert.equal(shouldSubmitSessionCrash(seen, overflow), false);
    assert.equal(seen.size, MAX_SESSION_CRASH_REPORTS);
  });
});

describe("Bug Report Client-Side Input Validation", () => {
  it("enforces trimmed length bounds matching Worker rules (title 3..180, description 5..8000)", () => {
    assert.deepEqual(validateBugReportInput("  ab  ", "1234"), {
      valid: false,
      titleError: "titleTooShort",
      descriptionError: "descriptionTooShort",
    });

    assert.deepEqual(validateBugReportInput("  abc  ", "  12345  "), {
      valid: true,
      titleError: null,
      descriptionError: null,
    });

    assert.deepEqual(
      validateBugReportInput(
        "x".repeat(BUG_REPORT_TITLE_MAX),
        "y".repeat(BUG_REPORT_DESCRIPTION_MAX),
      ),
      {
        valid: true,
        titleError: null,
        descriptionError: null,
      },
    );

    assert.deepEqual(
      validateBugReportInput(
        "x".repeat(BUG_REPORT_TITLE_MAX + 1),
        "y".repeat(BUG_REPORT_DESCRIPTION_MAX + 1),
      ),
      {
        valid: false,
        titleError: "titleTooLong",
        descriptionError: "descriptionTooLong",
      },
    );
  });
});
