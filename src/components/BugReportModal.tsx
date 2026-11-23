import {
  AlertTriangle,
  Bug,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  HardDriveDownload,
  Loader2,
  Send,
  ShieldCheck,
  X,
} from "lucide-react";
import { type FormEvent, useCallback, useEffect, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import {
  BUG_REPORT_DESCRIPTION_MAX,
  BUG_REPORT_DESCRIPTION_MIN,
  BUG_REPORT_TITLE_MAX,
  BUG_REPORT_TITLE_MIN,
  validateBugReportInput,
} from "../lib/crashUtils";
import {
  collectDiagnosticsPreview,
  saveDiagnosticsLocal,
  submitBugReport,
} from "../lib/tauri";
import { useAppStore } from "../stores/appStore";
import type { DiagnosticsPreview, ReportSubmitResult } from "../types/models";

export function BugReportModal() {
  const isOpen = useAppStore((s) => s.isBugReportOpen);
  const setBugReportOpen = useAppStore((s) => s.setBugReportOpen);
  const config = useAppStore((s) => s.config);

  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [titleTouched, setTitleTouched] = useState(false);
  const [descriptionTouched, setDescriptionTouched] = useState(false);
  const [steps, setSteps] = useState("");
  const [contact, setContact] = useState("");
  const [includeDiagnostics, setIncludeDiagnostics] = useState(true);

  const [showPreview, setShowPreview] = useState(false);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewData, setPreviewData] = useState<DiagnosticsPreview | null>(
    null,
  );
  const [previewError, setPreviewError] = useState<string | null>(null);

  const [submitting, setSubmitting] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [submitResult, setSubmitResult] = useState<ReportSubmitResult | null>(
    null,
  );
  const [submitException, setSubmitException] = useState<string | null>(null);

  const [savingLocal, setSavingLocal] = useState(false);
  const [localSavedPath, setLocalSavedPath] = useState<string | null>(null);
  const [localSaveError, setLocalSaveError] = useState<string | null>(null);

  const resetFormAndResults = useCallback(() => {
    setTitle("");
    setDescription("");
    setTitleTouched(false);
    setDescriptionTouched(false);
    setSteps("");
    setContact("");
    setIncludeDiagnostics(true);
    setValidationError(null);
    setSubmitResult(null);
    setSubmitException(null);
    setLocalSavedPath(null);
    setLocalSaveError(null);
  }, []);

  useEffect(() => {
    if (isOpen) {
      setValidationError(null);
      setTitleTouched(false);
      setDescriptionTouched(false);
      setSubmitException(null);
      setLocalSavedPath(null);
      setLocalSaveError(null);
      setPreviewError(null);
      if (submitResult?.sent) {
        resetFormAndResults();
      } else {
        setSubmitResult(null);
      }
      setShowPreview(false);
      setPreviewData(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !submitting) {
        e.preventDefault();
        setBugReportOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, submitting, setBugReportOpen]);

  if (!isOpen) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang).bugReport;

  const handleTogglePreview = async () => {
    const nextOpen = !showPreview;
    setShowPreview(nextOpen);
    if (nextOpen && !previewData && !previewLoading) {
      setPreviewLoading(true);
      setPreviewError(null);
      try {
        const preview = await collectDiagnosticsPreview();
        setPreviewData(preview);
      } catch (err) {
        setPreviewError(err instanceof Error ? err.message : String(err));
      } finally {
        setPreviewLoading(false);
      }
    }
  };

  const validation = validateBugReportInput(title, description);
  const trimmedTitleLen = title.trim().length;
  const trimmedDescLen = description.trim().length;
  const showTitleHint =
    (title.length > 0 || titleTouched) && validation.titleError !== null;
  const showDescriptionHint =
    (description.length > 0 || descriptionTouched) &&
    validation.descriptionError !== null;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setTitleTouched(true);
    setDescriptionTouched(true);

    if (!validation.valid) {
      if (validation.titleError && validation.descriptionError) {
        setValidationError(t.validationRequired);
      } else if (validation.titleError) {
        setValidationError(t[validation.titleError]);
      } else if (validation.descriptionError) {
        setValidationError(t[validation.descriptionError]);
      } else {
        setValidationError(t.validationRequired);
      }
      return;
    }

    const trimmedTitle = title.trim();
    const trimmedDescription = description.trim();

    setValidationError(null);
    setSubmitException(null);
    setSubmitResult(null);
    setSubmitting(true);

    try {
      const result = await submitBugReport({
        title: trimmedTitle,
        description: trimmedDescription,
        steps: steps.trim() || null,
        contact: contact.trim() || null,
        includeDiagnostics,
      });
      setSubmitResult(result);
    } catch (err) {
      setSubmitException(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const handleSaveLocal = async () => {
    setSavingLocal(true);
    setLocalSaveError(null);
    try {
      const path = await saveDiagnosticsLocal();
      setLocalSavedPath(path);
    } catch (err) {
      setLocalSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSavingLocal(false);
    }
  };

  const canSubmit = !submitting && validation.valid;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="bug-report-modal-title"
        className="flex max-h-[90vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl"
      >
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-slate-800 bg-slate-900/60 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-cyan-500/30 bg-cyan-500/15 text-cyan-300">
              <Bug className="h-5 w-5" />
            </div>
            <div>
              <h3
                id="bug-report-modal-title"
                className="text-base font-bold text-slate-100"
              >
                {t.title}
              </h3>
              <p className="text-xs text-slate-400">{t.subtitle}</p>
            </div>
          </div>

          <button
            type="button"
            disabled={submitting}
            onClick={() => setBugReportOpen(false)}
            className="rounded-lg p-1 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200 disabled:opacity-50"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <form
          id="bug-report-form"
          onSubmit={(e) => void handleSubmit(e)}
          className="flex-1 space-y-3.5 overflow-y-auto p-5 text-xs"
        >
          {/* Result banners */}
          {submitResult?.sent && (
            <div className="space-y-2 rounded-xl border border-emerald-500/35 bg-emerald-950/25 p-3.5 text-emerald-200">
              <div className="flex items-center gap-2 font-bold text-emerald-300">
                <CheckCircle2 className="h-4 w-4 shrink-0" />
                <span>{t.successTitle}</span>
              </div>
              {submitResult.deduplicated && (
                <p className="text-[11px] text-emerald-200/90">
                  {t.successDeduplicated}
                </p>
              )}
              {(submitResult.issueNumber !== null || submitResult.issueUrl) && (
                <div className="flex flex-wrap items-center gap-2 rounded-lg border border-emerald-500/25 bg-slate-950/70 px-3 py-2 font-mono text-[11px]">
                  <span className="font-semibold text-emerald-300">
                    {t.issueLinkLabel}
                    {submitResult.issueNumber !== null
                      ? ` #${submitResult.issueNumber}`
                      : ""}
                  </span>
                  {submitResult.issueUrl && (
                    <a
                      href={submitResult.issueUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-1 break-all text-cyan-300 hover:underline select-text"
                    >
                      <span>{submitResult.issueUrl}</span>
                      <ExternalLink className="h-3 w-3 shrink-0" />
                    </a>
                  )}
                </div>
              )}
            </div>
          )}

          {submitResult && !submitResult.sent && (
            <div
              className={`space-y-2 rounded-xl border p-3.5 ${
                submitResult.savedLocalPath
                  ? "border-amber-500/35 bg-amber-950/25 text-amber-200"
                  : "border-rose-500/35 bg-rose-950/25 text-rose-200"
              }`}
            >
              <div className="flex items-center gap-2 font-semibold">
                <AlertTriangle className="h-4 w-4 shrink-0" />
                <span>
                  {submitResult.savedLocalPath
                    ? t.offlineSavedTitle
                    : t.submitFailedTitle}
                </span>
              </div>
              {submitResult.savedLocalPath && (
                <div className="space-y-1">
                  <div className="text-[11px] text-amber-200/80">
                    {t.savedPathLabel}
                  </div>
                  <div className="break-all rounded-lg border border-slate-800 bg-slate-950/80 px-2.5 py-1.5 font-mono text-[11px] text-slate-200 select-text">
                    {submitResult.savedLocalPath}
                  </div>
                </div>
              )}
              {submitResult.error && (
                <div className="font-mono text-[11px] opacity-85 select-text">
                  {submitResult.error}
                </div>
              )}
            </div>
          )}

          {submitException && (
            <div className="flex items-start gap-2 rounded-xl border border-rose-500/35 bg-rose-950/25 p-3 font-mono text-[11px] text-rose-200 select-text">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-rose-400" />
              <span>{submitException}</span>
            </div>
          )}

          {validationError && (
            <div className="flex items-center gap-2 rounded-xl border border-amber-500/35 bg-amber-950/25 p-3 text-amber-200">
              <AlertTriangle className="h-4 w-4 shrink-0 text-amber-400" />
              <span>{validationError}</span>
            </div>
          )}

          {localSavedPath && (
            <div className="space-y-1 rounded-xl border border-cyan-500/35 bg-cyan-950/25 p-3 text-cyan-200">
              <div className="flex items-center gap-2 font-semibold">
                <CheckCircle2 className="h-4 w-4 shrink-0 text-cyan-300" />
                <span>{t.localDiagSavedTitle}</span>
              </div>
              <div className="break-all font-mono text-[11px] text-slate-200 select-text">
                {localSavedPath}
              </div>
            </div>
          )}

          {localSaveError && (
            <div className="rounded-xl border border-rose-500/35 bg-rose-950/25 p-3 font-mono text-[11px] text-rose-200 select-text">
              {localSaveError}
            </div>
          )}

          {/* Title */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <label
                htmlFor="bug-report-title"
                className="block text-[11px] font-bold uppercase tracking-wider text-slate-400"
              >
                {t.titleLabel} <span className="text-rose-400">*</span>
              </label>
              <span className="font-mono text-[10px] text-slate-500">
                {t.titleMinHint} • {trimmedTitleLen}/{BUG_REPORT_TITLE_MAX}
              </span>
            </div>
            <input
              id="bug-report-title"
              type="text"
              required
              minLength={BUG_REPORT_TITLE_MIN}
              maxLength={BUG_REPORT_TITLE_MAX}
              value={title}
              onBlur={() => setTitleTouched(true)}
              onChange={(e) => {
                setTitle(e.target.value);
                if (validationError) setValidationError(null);
              }}
              placeholder={t.titlePlaceholder}
              aria-invalid={showTitleHint}
              className={`w-full rounded-xl border bg-slate-900 px-3 py-2 text-xs text-slate-100 placeholder:text-slate-500 focus:outline-none ${
                showTitleHint
                  ? "border-amber-500/60 focus:border-amber-400"
                  : "border-slate-800 focus:border-cyan-500"
              }`}
            />
            {showTitleHint && validation.titleError && (
              <p className="text-[11px] text-amber-300">
                {t[validation.titleError]}
              </p>
            )}
          </div>

          {/* Description */}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <label
                htmlFor="bug-report-description"
                className="block text-[11px] font-bold uppercase tracking-wider text-slate-400"
              >
                {t.descriptionLabel} <span className="text-rose-400">*</span>
              </label>
              <span className="font-mono text-[10px] text-slate-500">
                {t.descriptionMinHint} • {trimmedDescLen}/
                {BUG_REPORT_DESCRIPTION_MAX}
              </span>
            </div>
            <textarea
              id="bug-report-description"
              required
              minLength={BUG_REPORT_DESCRIPTION_MIN}
              maxLength={BUG_REPORT_DESCRIPTION_MAX}
              rows={4}
              value={description}
              onBlur={() => setDescriptionTouched(true)}
              onChange={(e) => {
                setDescription(e.target.value);
                if (validationError) setValidationError(null);
              }}
              placeholder={t.descriptionPlaceholder}
              aria-invalid={showDescriptionHint}
              className={`w-full resize-y rounded-xl border bg-slate-900 px-3 py-2 text-xs leading-relaxed text-slate-100 placeholder:text-slate-500 focus:outline-none ${
                showDescriptionHint
                  ? "border-amber-500/60 focus:border-amber-400"
                  : "border-slate-800 focus:border-cyan-500"
              }`}
            />
            {showDescriptionHint && validation.descriptionError && (
              <p className="text-[11px] text-amber-300">
                {t[validation.descriptionError]}
              </p>
            )}
          </div>

          {/* Steps to reproduce */}
          <div className="space-y-1.5">
            <label
              htmlFor="bug-report-steps"
              className="block text-[11px] font-bold uppercase tracking-wider text-slate-400"
            >
              {t.stepsLabel}
            </label>
            <textarea
              id="bug-report-steps"
              rows={3}
              value={steps}
              onChange={(e) => setSteps(e.target.value)}
              placeholder={t.stepsPlaceholder}
              className="w-full resize-y rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs leading-relaxed text-slate-100 placeholder:text-slate-500 focus:border-cyan-500 focus:outline-none"
            />
          </div>

          {/* Contact */}
          <div className="space-y-1.5">
            <label
              htmlFor="bug-report-contact"
              className="block text-[11px] font-bold uppercase tracking-wider text-slate-400"
            >
              {t.contactLabel}
            </label>
            <input
              id="bug-report-contact"
              type="text"
              maxLength={120}
              value={contact}
              onChange={(e) => setContact(e.target.value)}
              placeholder={t.contactPlaceholder}
              className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs text-slate-100 placeholder:text-slate-500 focus:border-cyan-500 focus:outline-none"
            />
          </div>

          {/* Sanitized diagnostics checkbox + privacy note */}
          <div className="space-y-2 rounded-xl border border-slate-800 bg-slate-900/50 p-3">
            <label className="flex cursor-pointer items-center gap-2.5 text-slate-200">
              <input
                id="bug-report-include-diag"
                type="checkbox"
                checked={includeDiagnostics}
                onChange={(e) => setIncludeDiagnostics(e.target.checked)}
                className="h-4 w-4 rounded border-slate-700 bg-slate-950 text-emerald-500"
              />
              <span className="font-semibold">{t.includeDiagnostics}</span>
            </label>

            <div className="flex items-start gap-2 pl-6 text-[11px] leading-relaxed text-slate-400">
              <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-400" />
              <span>{t.privacyNote}</span>
            </div>

            {/* Collapsible diagnostics preview */}
            <div className="border-t border-slate-800/80 pt-2">
              <button
                type="button"
                onClick={() => void handleTogglePreview()}
                className="inline-flex items-center gap-1.5 text-[11px] font-semibold text-cyan-300 transition hover:text-cyan-200"
              >
                {showPreview ? (
                  <ChevronDown className="h-3.5 w-3.5" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5" />
                )}
                <span>{showPreview ? t.hidePreview : t.showPreview}</span>
              </button>

              {showPreview && (
                <div className="mt-2.5 space-y-2.5 rounded-xl border border-slate-800 bg-slate-950 p-3 text-[11px]">
                  {previewLoading && (
                    <div className="flex items-center gap-2 text-cyan-300">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      <span>{t.loadingPreview}</span>
                    </div>
                  )}

                  {previewError && (
                    <div className="font-mono text-rose-300">
                      {previewError}
                    </div>
                  )}

                  {previewData && !previewLoading && (
                    <>
                      <div className="grid gap-1.5 sm:grid-cols-2">
                        <div>
                          <span className="text-slate-500">
                            {t.previewAppVersion}:{" "}
                          </span>
                          <span className="font-mono font-semibold text-emerald-300 select-text">
                            v{previewData.appVersion}
                          </span>
                        </div>
                        <div>
                          <span className="text-slate-500">
                            {t.previewPlatform}:{" "}
                          </span>
                          <span className="font-mono text-slate-200 select-text">
                            {previewData.platform} • {previewData.osInfo}
                          </span>
                        </div>
                      </div>

                      <div>
                        <div className="text-slate-500">{t.previewGpu}:</div>
                        <div className="mt-0.5 font-mono text-cyan-200 select-text">
                          {previewData.gpuSummary}
                        </div>
                      </div>

                      <div>
                        <div className="text-slate-500">
                          {t.previewComponents}:
                        </div>
                        {previewData.installedComponents.length > 0 ? (
                          <div className="mt-1 flex flex-wrap gap-1">
                            {previewData.installedComponents.map((comp) => (
                              <span
                                key={comp}
                                className="rounded-md border border-slate-800 bg-slate-900 px-2 py-0.5 font-mono text-[10px] text-slate-300"
                              >
                                {comp}
                              </span>
                            ))}
                          </div>
                        ) : (
                          <div className="mt-0.5 text-slate-500">
                            {t.previewNoComponents}
                          </div>
                        )}
                      </div>

                      <div>
                        <div className="text-slate-500">{t.previewLogs}:</div>
                        <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-slate-800 bg-slate-900/80 p-2 font-mono text-[10px] text-slate-300 select-text">
                          {previewData.logExcerpt.trim() || t.previewNoLogs}
                        </pre>
                      </div>
                    </>
                  )}
                </div>
              )}
            </div>
          </div>
        </form>

        {/* Footer */}
        <div className="flex flex-wrap items-center justify-between gap-2.5 border-t border-slate-800 bg-slate-900/70 px-5 py-3.5">
          <button
            type="button"
            disabled={submitting}
            onClick={() => setBugReportOpen(false)}
            className="rounded-xl border border-slate-700 bg-slate-800 px-3.5 py-2 text-xs font-semibold text-slate-200 transition hover:bg-slate-700 disabled:opacity-50"
          >
            {t.close}
          </button>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={savingLocal || submitting}
              onClick={() => void handleSaveLocal()}
              className="inline-flex items-center gap-1.5 rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-3.5 py-2 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-500/25 disabled:opacity-50"
            >
              {savingLocal ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  <span>{t.savingLocal}</span>
                </>
              ) : (
                <>
                  <HardDriveDownload className="h-3.5 w-3.5" />
                  <span>{t.saveLocalBtn}</span>
                </>
              )}
            </button>

            {submitResult?.sent ? (
              <button
                type="button"
                onClick={resetFormAndResults}
                className="inline-flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400"
              >
                <span>{t.newReport}</span>
              </button>
            ) : (
              <button
                type="submit"
                form="bug-report-form"
                disabled={!canSubmit}
                className="inline-flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-50"
              >
                {submitting ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    <span>{t.submitting}</span>
                  </>
                ) : (
                  <>
                    <Send className="h-3.5 w-3.5" />
                    <span>{t.submitBtn}</span>
                  </>
                )}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
