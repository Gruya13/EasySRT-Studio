import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import {
  MAX_SESSION_CRASH_REPORTS,
  normalizeUnhandledRejection,
  normalizeWindowErrorEvent,
  shouldSubmitSessionCrash,
} from "./lib/crashUtils";
import { submitCrashReport } from "./lib/tauri";
import { useAppStore } from "./stores/appStore";
import type { CrashReportRequest } from "./types/models";
import "./index.css";

const sessionCrashKeys = new Set<string>();

function autoReportCrash(request: CrashReportRequest | null): void {
  if (!request) {
    return;
  }
  try {
    if (useAppStore.getState().config.autoSendCrashReports === false) {
      return;
    }
  } catch {
    // Store not ready yet — fall through with default enabled
  }
  if (
    !shouldSubmitSessionCrash(
      sessionCrashKeys,
      request,
      MAX_SESSION_CRASH_REPORTS,
    )
  ) {
    return;
  }
  void submitCrashReport(request).catch(() => {
    // Never let crash telemetry failures trigger another unhandled rejection
  });
}

window.addEventListener("error", (event) => {
  autoReportCrash(normalizeWindowErrorEvent(event));
});

window.addEventListener("unhandledrejection", (event) => {
  autoReportCrash(normalizeUnhandledRejection(event.reason));
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
