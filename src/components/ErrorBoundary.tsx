import {
  AlertTriangle,
  CheckCircle2,
  ExternalLink,
  HardDriveDownload,
  Loader2,
  RotateCcw,
  ShieldAlert,
} from "lucide-react";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { saveDiagnosticsLocal, submitCrashReport } from "../lib/tauri";
import { useAppStore } from "../stores/appStore";
import type { ReportSubmitResult } from "../types/models";

interface ErrorBoundaryProps {
  children: ReactNode;
}

type AutoReportStatus =
  | "idle"
  | "sending"
  | "sent"
  | "saved"
  | "disabled"
  | "failed";

interface ErrorBoundaryState {
  hasError: boolean;
  errorMessage: string;
  reportStatus: AutoReportStatus;
  reportResult: ReportSubmitResult | null;
  reportError: string | null;
  savingLocal: boolean;
  localSavedPath: string | null;
  localSaveError: string | null;
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      errorMessage: "",
      reportStatus: "idle",
      reportResult: null,
      reportError: null,
      savingLocal: false,
      localSavedPath: null,
      localSaveError: null,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return {
      hasError: true,
      errorMessage: error?.message || String(error),
    };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    let autoSend = true;
    try {
      autoSend = useAppStore.getState().config.autoSendCrashReports !== false;
    } catch {
      autoSend = true;
    }

    if (!autoSend) {
      this.setState({ reportStatus: "disabled" });
      return;
    }

    const stackTrace =
      [error?.stack, errorInfo?.componentStack].filter(Boolean).join("\n") ||
      null;

    this.setState({ reportStatus: "sending", reportError: null });

    void submitCrashReport({
      crashKind: "react_error",
      message: error?.message || String(error),
      stackTrace,
      source: "ErrorBoundary",
    })
      .then((result) => {
        if (result.sent) {
          this.setState({
            reportStatus: "sent",
            reportResult: result,
            reportError: null,
          });
        } else if (result.savedLocalPath) {
          this.setState({
            reportStatus: "saved",
            reportResult: result,
            reportError: result.error,
          });
        } else {
          this.setState({
            reportStatus: "failed",
            reportResult: result,
            reportError: result.error,
          });
        }
      })
      .catch((err) => {
        this.setState({
          reportStatus: "failed",
          reportError: err instanceof Error ? err.message : String(err),
        });
      });
  }

  private handleReload = () => {
    window.location.reload();
  };

  private handleSaveDiagnosticsLocal = async () => {
    this.setState({ savingLocal: true, localSaveError: null });
    try {
      const savedPath = await saveDiagnosticsLocal();
      this.setState({
        savingLocal: false,
        localSavedPath: savedPath,
        localSaveError: null,
      });
    } catch (err) {
      this.setState({
        savingLocal: false,
        localSaveError: err instanceof Error ? err.message : String(err),
      });
    }
  };

  render(): ReactNode {
    if (!this.state.hasError) {
      return this.props.children;
    }

    let uiLanguage = "en";
    try {
      uiLanguage = useAppStore.getState().config.uiLanguage;
    } catch {
      uiLanguage = "en";
    }
    const lang = normalizeUiLanguage(uiLanguage);
    const t = getDictionary(lang).crashRecovery;

    const {
      errorMessage,
      reportStatus,
      reportResult,
      reportError,
      savingLocal,
      localSavedPath,
      localSaveError,
    } = this.state;

    return (
      <div className="flex min-h-screen w-full items-center justify-center bg-[#06070a] p-6 text-slate-100">
        <div className="w-full max-w-xl overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl">
          {/* Header */}
          <div className="flex items-start gap-3.5 border-b border-slate-800 bg-rose-950/20 px-6 py-5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-rose-500/40 bg-rose-500/15 text-rose-300">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div className="min-w-0 space-y-1">
              <span className="inline-flex items-center rounded-full border border-rose-500/30 bg-rose-500/15 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wider text-rose-300">
                {t.badge}
              </span>
              <h1 className="text-base font-bold text-slate-100">{t.title}</h1>
              <p className="text-xs leading-relaxed text-slate-400">
                {t.subtitle}
              </p>
            </div>
          </div>

          {/* Body */}
          <div className="space-y-4 p-6 text-xs">
            {/* Error summary */}
            <div className="space-y-1.5">
              <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">
                {t.errorDetails}
              </div>
              <pre className="max-h-28 overflow-auto whitespace-pre-wrap break-words rounded-xl border border-slate-800 bg-slate-900/80 p-3 font-mono text-[11px] text-rose-200 select-text">
                {errorMessage || "Unknown rendering error"}
              </pre>
            </div>

            {/* Automatic crash report status */}
            {reportStatus === "sending" && (
              <div className="flex items-center gap-2.5 rounded-xl border border-cyan-500/30 bg-cyan-950/25 p-3 text-cyan-200">
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-cyan-400" />
                <span>{t.statusSending}</span>
              </div>
            )}

            {reportStatus === "sent" && (
              <div className="space-y-1.5 rounded-xl border border-emerald-500/35 bg-emerald-950/25 p-3 text-emerald-200">
                <div className="flex items-center gap-2 font-semibold">
                  <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-400" />
                  <span>
                    {reportResult?.deduplicated
                      ? t.statusSentDeduplicated
                      : t.statusSent}
                  </span>
                </div>
                {reportResult?.issueNumber !== null &&
                  reportResult?.issueNumber !== undefined && (
                    <div className="flex items-center gap-2 pl-6 font-mono text-[11px]">
                      <span className="text-emerald-300">
                        Issue #{reportResult.issueNumber}
                      </span>
                      {reportResult.issueUrl && (
                        <a
                          href={reportResult.issueUrl}
                          target="_blank"
                          rel="noreferrer"
                          className="inline-flex items-center gap-1 text-cyan-300 hover:underline"
                        >
                          <span>{reportResult.issueUrl}</span>
                          <ExternalLink className="h-3 w-3 shrink-0" />
                        </a>
                      )}
                    </div>
                  )}
              </div>
            )}

            {reportStatus === "saved" && reportResult?.savedLocalPath && (
              <div className="space-y-1.5 rounded-xl border border-amber-500/35 bg-amber-950/25 p-3 text-amber-200">
                <div className="flex items-center gap-2 font-medium">
                  <AlertTriangle className="h-4 w-4 shrink-0 text-amber-400" />
                  <span>{t.statusSaved}</span>
                </div>
                <div className="break-all rounded-lg border border-slate-800 bg-slate-950/80 px-2.5 py-1.5 font-mono text-[11px] text-slate-300 select-text">
                  {reportResult.savedLocalPath}
                </div>
              </div>
            )}

            {reportStatus === "disabled" && (
              <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-3 text-slate-400">
                {t.statusDisabled}
              </div>
            )}

            {reportStatus === "failed" && (
              <div className="space-y-1 rounded-xl border border-amber-500/30 bg-amber-950/20 p-3 text-amber-200">
                <div className="flex items-center gap-2 font-medium">
                  <AlertTriangle className="h-4 w-4 shrink-0 text-amber-400" />
                  <span>{t.statusFailed}</span>
                </div>
                {reportError && (
                  <div className="pl-6 font-mono text-[11px] text-amber-300/90 select-text">
                    {reportError}
                  </div>
                )}
              </div>
            )}

            {/* Local diagnostics export feedback */}
            {localSavedPath && (
              <div className="space-y-1 rounded-xl border border-emerald-500/30 bg-emerald-950/20 p-3 text-emerald-200">
                <div className="flex items-center gap-2 font-semibold">
                  <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-400" />
                  <span>{t.localSavedPrefix}</span>
                </div>
                <div className="break-all font-mono text-[11px] text-slate-300 select-text">
                  {localSavedPath}
                </div>
              </div>
            )}

            {localSaveError && (
              <div className="rounded-xl border border-rose-500/35 bg-rose-950/25 p-3 font-mono text-[11px] text-rose-200 select-text">
                {localSaveError}
              </div>
            )}
          </div>

          {/* Actions */}
          <div className="flex flex-wrap items-center justify-end gap-2.5 border-t border-slate-800 bg-slate-900/70 px-6 py-4">
            <button
              type="button"
              disabled={savingLocal}
              onClick={() => void this.handleSaveDiagnosticsLocal()}
              className="inline-flex items-center gap-2 rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 transition hover:bg-slate-700 disabled:opacity-50"
            >
              {savingLocal ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>{t.savingLocal}</span>
                </>
              ) : (
                <>
                  <HardDriveDownload className="h-4 w-4" />
                  <span>{t.saveLocalBtn}</span>
                </>
              )}
            </button>

            <button
              type="button"
              onClick={this.handleReload}
              className="inline-flex items-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400"
            >
              <RotateCcw className="h-4 w-4" />
              <span>{t.reloadBtn}</span>
            </button>
          </div>
        </div>
      </div>
    );
  }
}
