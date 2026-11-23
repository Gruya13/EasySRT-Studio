import type { CrashReportRequest } from "../types/models";

export const MAX_SESSION_CRASH_REPORTS = 10;

export const BUG_REPORT_TITLE_MIN = 3;
export const BUG_REPORT_TITLE_MAX = 180;
export const BUG_REPORT_DESCRIPTION_MIN = 5;
export const BUG_REPORT_DESCRIPTION_MAX = 8000;

export type BugReportFieldError =
  | "titleTooShort"
  | "titleTooLong"
  | "descriptionTooShort"
  | "descriptionTooLong";

export interface BugReportValidationResult {
  valid: boolean;
  titleError: "titleTooShort" | "titleTooLong" | null;
  descriptionError: "descriptionTooShort" | "descriptionTooLong" | null;
}

export function validateBugReportInput(
  title: string,
  description: string,
): BugReportValidationResult {
  const trimmedTitleLen = title.trim().length;
  const trimmedDescLen = description.trim().length;

  let titleError: "titleTooShort" | "titleTooLong" | null = null;
  if (trimmedTitleLen < BUG_REPORT_TITLE_MIN) {
    titleError = "titleTooShort";
  } else if (trimmedTitleLen > BUG_REPORT_TITLE_MAX) {
    titleError = "titleTooLong";
  }

  let descriptionError: "descriptionTooShort" | "descriptionTooLong" | null =
    null;
  if (trimmedDescLen < BUG_REPORT_DESCRIPTION_MIN) {
    descriptionError = "descriptionTooShort";
  } else if (trimmedDescLen > BUG_REPORT_DESCRIPTION_MAX) {
    descriptionError = "descriptionTooLong";
  }

  return {
    valid: titleError === null && descriptionError === null,
    titleError,
    descriptionError,
  };
}

const IGNORED_ERROR_PATTERNS = [
  "ResizeObserver loop limit exceeded",
  "ResizeObserver loop completed with undelivered notifications",
];

export function isIgnoredBrowserError(message: string): boolean {
  const normalized = message.trim();
  if (!normalized) return false;
  return IGNORED_ERROR_PATTERNS.some((pattern) => normalized.includes(pattern));
}

export function buildCrashDedupeKey(request: CrashReportRequest): string {
  const firstStackLine =
    (request.stackTrace ?? "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line.length > 0) ?? "";
  return `${request.crashKind}::${request.message.trim()}::${firstStackLine}`;
}

export function normalizeWindowErrorEvent(event: {
  error?: unknown;
  message?: string;
  filename?: string;
  lineno?: number;
  colno?: number;
}): CrashReportRequest | null {
  const err = event.error;
  const message =
    err instanceof Error
      ? err.message || String(err)
      : typeof event.message === "string" && event.message.trim().length > 0
        ? event.message.trim()
        : err !== undefined && err !== null
          ? String(err)
          : "Unknown window error";

  if (isIgnoredBrowserError(message)) {
    return null;
  }

  const stackTrace =
    err instanceof Error && typeof err.stack === "string" && err.stack.trim()
      ? err.stack
      : null;

  const sourceParts: string[] = [];
  if (event.filename) {
    sourceParts.push(event.filename);
    if (typeof event.lineno === "number") {
      sourceParts.push(String(event.lineno));
      if (typeof event.colno === "number") {
        sourceParts.push(String(event.colno));
      }
    }
  }

  return {
    crashKind: "window_error",
    message,
    stackTrace,
    source: sourceParts.length > 0 ? sourceParts.join(":") : "window.onerror",
  };
}

export function normalizeUnhandledRejection(
  reason: unknown,
): CrashReportRequest | null {
  let message: string;
  let stackTrace: string | null = null;

  if (reason instanceof Error) {
    message = reason.message || String(reason);
    stackTrace =
      typeof reason.stack === "string" && reason.stack.trim()
        ? reason.stack
        : null;
  } else if (typeof reason === "string") {
    message = reason.trim() || "Unhandled promise rejection";
  } else if (reason === null || reason === undefined) {
    message = "Unhandled promise rejection";
  } else {
    try {
      message = JSON.stringify(reason);
    } catch {
      message = String(reason);
    }
    if (!message || message === "{}") {
      message = String(reason);
    }
  }

  if (isIgnoredBrowserError(message)) {
    return null;
  }

  return {
    crashKind: "unhandled_rejection",
    message,
    stackTrace,
    source: "window.onunhandledrejection",
  };
}

export function shouldSubmitSessionCrash(
  seenKeys: Set<string>,
  request: CrashReportRequest,
  maxReports = MAX_SESSION_CRASH_REPORTS,
): boolean {
  if (seenKeys.size >= maxReports) {
    return false;
  }
  const key = buildCrashDedupeKey(request);
  if (seenKeys.has(key)) {
    return false;
  }
  seenKeys.add(key);
  return true;
}
