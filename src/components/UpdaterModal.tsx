import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  CheckCircle2,
  Download,
  Loader2,
  RefreshCw,
  RotateCcw,
  Sparkles,
  X,
} from "lucide-react";
import { useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatBytes, isTauriRuntime } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";

export function UpdaterModal() {
  const isOpen = useAppStore((s) => s.isUpdaterOpen);
  const setUpdaterOpen = useAppStore((s) => s.setUpdaterOpen);
  const config = useAppStore((s) => s.config);

  const [checking, setChecking] = useState(false);
  const [checkedOnce, setCheckedOnce] = useState(false);
  const [updateObj, setUpdateObj] = useState<Update | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [downloadedBytes, setDownloadedBytes] = useState(0);
  const [totalBytes, setTotalBytes] = useState(0);
  const [readyToRestart, setReadyToRestart] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  if (!isOpen) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const handleCheckUpdates = async () => {
    setChecking(true);
    setErrorMessage(null);
    try {
      if (!isTauriRuntime()) {
        await new Promise((r) => setTimeout(r, 350));
        setUpdateObj(null);
        setCheckedOnce(true);
        return;
      }
      const update = await check();
      setUpdateObj(update);
      setCheckedOnce(true);
    } catch (err) {
      // Friendly handling when placeholder key or no GitHub release published yet
      const msg = err instanceof Error ? err.message : String(err);
      setUpdateObj(null);
      setCheckedOnce(true);
      if (msg && !msg.toLowerCase().includes("up to date")) {
        setErrorMessage(msg);
      }
    } finally {
      setChecking(false);
    }
  };

  const handleDownloadAndInstall = async () => {
    if (!updateObj) {
      return;
    }
    setDownloading(true);
    setErrorMessage(null);
    let accumulated = 0;
    try {
      await updateObj.downloadAndInstall((event) => {
        if (event.event === "Started") {
          setTotalBytes(event.data.contentLength ?? 0);
          accumulated = 0;
          setDownloadedBytes(0);
        } else if (event.event === "Progress") {
          accumulated += event.data.chunkLength;
          setDownloadedBytes(accumulated);
        } else if (event.event === "Finished") {
          setReadyToRestart(true);
        }
      });
      setReadyToRestart(true);
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : String(err));
    } finally {
      setDownloading(false);
    }
  };

  const handleRelaunch = async () => {
    if (isTauriRuntime()) {
      await relaunch();
    }
  };

  const percent =
    totalBytes > 0
      ? Math.min(100, Math.round((downloadedBytes / totalBytes) * 100))
      : 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div className="w-full max-w-md overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-slate-800 bg-slate-900/60 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-emerald-500/30 bg-emerald-500/15 text-emerald-300">
              <RefreshCw className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-slate-100">
                {t.updater.title}
              </h3>
              <p className="text-xs text-slate-400">{t.updater.subtitle}</p>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setUpdaterOpen(false)}
            className="rounded-lg p-1 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <div className="space-y-4 p-5 text-xs">
          <div className="flex items-center justify-between rounded-xl border border-slate-800 bg-slate-900/70 px-3.5 py-2.5">
            <span className="text-slate-400">{t.updater.currentVersion}</span>
            <span className="font-mono font-bold text-emerald-300">v0.1.1</span>
          </div>

          {!updateObj && checkedOnce && !errorMessage && (
            <div className="flex items-center gap-2 rounded-xl border border-emerald-500/30 bg-emerald-950/20 p-3 text-emerald-300">
              <CheckCircle2 className="h-4 w-4 shrink-0" />
              <span className="font-medium">{t.updater.upToDate}</span>
            </div>
          )}

          {errorMessage && (
            <div className="rounded-xl border border-amber-500/30 bg-amber-950/20 p-3 font-mono text-[11px] text-amber-200">
              {errorMessage}
            </div>
          )}

          {updateObj && (
            <div className="space-y-3 rounded-xl border border-cyan-500/30 bg-cyan-950/20 p-3.5">
              <div className="flex items-center justify-between">
                <span className="inline-flex items-center gap-1.5 font-bold text-cyan-300">
                  <Sparkles className="h-4 w-4" />
                  <span>
                    {t.updater.updateAvailable}: v{updateObj.version}
                  </span>
                </span>
              </div>

              {updateObj.body && (
                <div className="space-y-1">
                  <div className="text-[11px] font-semibold text-slate-400">
                    {t.updater.releaseNotes}:
                  </div>
                  <div className="max-h-32 overflow-y-auto whitespace-pre-wrap rounded-lg border border-slate-800 bg-slate-950 p-2.5 text-[11px] text-slate-300">
                    {updateObj.body}
                  </div>
                </div>
              )}

              {downloading && (
                <div className="space-y-1">
                  <div className="flex justify-between font-mono text-[11px] text-cyan-300">
                    <span>{t.updater.downloading}</span>
                    <span>
                      {percent}% ({formatBytes(downloadedBytes)}
                      {totalBytes ? ` / ${formatBytes(totalBytes)}` : ""})
                    </span>
                  </div>
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-emerald-400 to-cyan-400 transition-all"
                      style={{ width: `${Math.max(5, percent)}%` }}
                    />
                  </div>
                </div>
              )}

              {readyToRestart ? (
                <button
                  type="button"
                  onClick={() => void handleRelaunch()}
                  className="flex w-full items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 py-2.5 text-xs font-bold text-slate-950 shadow"
                >
                  <RotateCcw className="h-4 w-4" />
                  <span>{t.updater.relaunchNow}</span>
                </button>
              ) : (
                <button
                  type="button"
                  disabled={downloading}
                  onClick={() => void handleDownloadAndInstall()}
                  className="flex w-full items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 py-2.5 text-xs font-bold text-slate-950 shadow disabled:opacity-50"
                >
                  {downloading ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Download className="h-4 w-4" />
                  )}
                  <span>{t.updater.downloadAndInstall}</span>
                </button>
              )}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-slate-800 bg-slate-900/70 px-5 py-3.5">
          <button
            type="button"
            onClick={() => setUpdaterOpen(false)}
            className="rounded-xl border border-slate-700 bg-slate-800 px-3.5 py-2 text-xs font-semibold text-slate-200 hover:bg-slate-700"
          >
            {t.updater.close}
          </button>

          <button
            type="button"
            disabled={checking || downloading}
            onClick={() => void handleCheckUpdates()}
            className="inline-flex items-center gap-2 rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-4 py-2 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/25 disabled:opacity-50"
          >
            {checking ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                <span>{t.updater.checking}</span>
              </>
            ) : (
              <>
                <RefreshCw className="h-3.5 w-3.5" />
                <span>{t.updater.checkBtn}</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
