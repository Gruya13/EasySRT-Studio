import {
  Cloud,
  Cpu,
  Download,
  FileDown,
  History,
  Package,
  RefreshCw,
  Settings,
  Subtitles,
} from "lucide-react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { useAppStore } from "../stores/appStore";

export function TopBar() {
  const config = useAppStore((s) => s.config);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const checkpoints = useAppStore((s) => s.checkpoints);
  const activeProject = useAppStore((s) => s.activeProject);
  const isCheckpointsOpen = useAppStore((s) => s.isCheckpointsOpen);

  const setSetupOpen = useAppStore((s) => s.setSetupOpen);
  const setCheckpointsOpen = useAppStore((s) => s.setCheckpointsOpen);
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen);
  const setUpdaterOpen = useAppStore((s) => s.setUpdaterOpen);
  const setExportOpen = useAppStore((s) => s.setExportOpen);
  const quickExportSrt = useAppStore((s) => s.quickExportSrt);
  const setUiLanguage = useAppStore((s) => s.setUiLanguage);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const effectiveProvider =
    jobOptions.providerOverride ?? config.translation.provider;
  const hasNvidia = setupStatus?.gpu.hasNvidia ?? false;
  const storageMode = setupStatus?.storage.mode ?? "portable";
  const readyForOffline = setupStatus?.readyForOffline ?? false;

  const renderModeBadge = () => {
    if (effectiveProvider === "localLlama") {
      return (
        <div
          className={`flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-semibold tracking-wide shadow-sm ${
            hasNvidia
              ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-300"
              : "border-cyan-500/40 bg-cyan-500/15 text-cyan-300"
          }`}
          title={
            setupStatus?.gpu.gpuName
              ? `${setupStatus.gpu.gpuName} (${setupStatus.gpu.vramMb} MB VRAM)`
              : t.setup.noNvidiaGpu
          }
        >
          <span
            className={`h-2 w-2 rounded-full ${
              hasNvidia ? "bg-emerald-400 animate-pulse" : "bg-cyan-400"
            }`}
          />
          <Cpu className="h-3.5 w-3.5" />
          <span>{hasNvidia ? t.app.offlineCuda : t.app.offlineCpu}</span>
        </div>
      );
    }

    if (effectiveProvider === "deepSeek") {
      return (
        <div className="flex items-center gap-2 rounded-full border border-amber-500/40 bg-amber-500/15 px-3 py-1 text-xs font-semibold tracking-wide text-amber-300 shadow-sm">
          <span className="h-2 w-2 rounded-full bg-amber-400" />
          <Cloud className="h-3.5 w-3.5" />
          <span>{t.app.onlineDeepSeek}</span>
        </div>
      );
    }

    return (
      <div className="flex items-center gap-2 rounded-full border border-violet-500/40 bg-violet-500/15 px-3 py-1 text-xs font-semibold tracking-wide text-violet-300 shadow-sm">
        <span className="h-2 w-2 rounded-full bg-violet-400" />
        <Cloud className="h-3.5 w-3.5" />
        <span>{t.app.onlineCustom}</span>
      </div>
    );
  };

  return (
    <header className="sticky top-0 z-30 flex flex-wrap items-center justify-between gap-3 border-b border-slate-800/90 bg-slate-950/95 px-4 py-2.5 backdrop-blur-md">
      {/* Left: Logo, Title & Storage Mode Pill */}
      <div className="flex items-center gap-3">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-emerald-500/30 bg-gradient-to-br from-emerald-500/20 to-cyan-500/10 text-emerald-400 shadow-inner">
          <Subtitles className="h-5 w-5" />
        </div>
        <div className="flex flex-col">
          <div className="flex items-center gap-2">
            <span className="text-base font-bold tracking-tight text-slate-100">
              {t.app.title}
            </span>
            <span
              className={`rounded-md border px-1.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wider ${
                storageMode === "portable"
                  ? "border-cyan-500/30 bg-cyan-500/10 text-cyan-300"
                  : "border-slate-700 bg-slate-800/80 text-slate-300"
              }`}
              title={setupStatus?.storage.rootDir}
            >
              {storageMode === "portable" ? t.app.portable : t.app.installed}
            </span>
          </div>
          <span className="hidden text-[11px] text-slate-400 sm:inline">
            {t.app.subtitle}
          </span>
        </div>
      </div>

      {/* Center: Prominent Mode Badge */}
      <div className="flex items-center gap-2">{renderModeBadge()}</div>

      {/* Right: Quick Action Buttons & SR | EN Language Switch */}
      <div className="flex flex-wrap items-center gap-1.5">
        {activeProject && activeProject.cues.length > 0 && (
          <>
            <button
              type="button"
              onClick={() => void quickExportSrt()}
              className="flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-2.5 py-1.5 text-xs font-medium text-emerald-300 transition hover:bg-emerald-500/20"
              title={t.topBar.quickSrt}
            >
              <Download className="h-3.5 w-3.5" />
              <span>{t.topBar.quickSrt}</span>
            </button>
            <button
              type="button"
              onClick={() => setExportOpen(true)}
              className="flex items-center gap-1.5 rounded-lg border border-cyan-500/30 bg-cyan-500/15 px-2.5 py-1.5 text-xs font-medium text-cyan-200 transition hover:bg-cyan-500/25"
            >
              <FileDown className="h-3.5 w-3.5" />
              <span>{t.topBar.export}</span>
            </button>
          </>
        )}

        <button
          type="button"
          onClick={() => setSetupOpen(true)}
          className="relative flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900/90 px-2.5 py-1.5 text-xs font-medium text-slate-200 transition hover:border-slate-700 hover:bg-slate-800"
        >
          <Package className="h-3.5 w-3.5 text-cyan-400" />
          <span className="hidden md:inline">{t.topBar.setupWizard}</span>
          {!readyForOffline && (
            <span
              className="h-2 w-2 rounded-full bg-amber-400"
              title={t.app.setupNeeded}
            />
          )}
        </button>

        <button
          type="button"
          onClick={() => setCheckpointsOpen(!isCheckpointsOpen)}
          className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition ${
            isCheckpointsOpen
              ? "border-cyan-500/50 bg-cyan-500/15 text-cyan-200"
              : "border-slate-800 bg-slate-900/90 text-slate-200 hover:border-slate-700 hover:bg-slate-800"
          }`}
        >
          <History className="h-3.5 w-3.5 text-emerald-400" />
          <span className="hidden md:inline">{t.topBar.checkpoints}</span>
          {checkpoints.length > 0 && (
            <span className="rounded-full bg-slate-800 px-1.5 py-0.2 font-mono text-[10px] text-slate-300">
              {checkpoints.length}
            </span>
          )}
        </button>

        <button
          type="button"
          onClick={() => setSettingsOpen(true)}
          className="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900/90 px-2.5 py-1.5 text-xs font-medium text-slate-200 transition hover:border-slate-700 hover:bg-slate-800"
          title={t.topBar.settings}
        >
          <Settings className="h-3.5 w-3.5 text-slate-300" />
          <span className="hidden lg:inline">{t.topBar.settings}</span>
        </button>

        <button
          type="button"
          onClick={() => setUpdaterOpen(true)}
          className="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900/90 px-2.5 py-1.5 text-xs font-medium text-slate-200 transition hover:border-slate-700 hover:bg-slate-800"
          title={t.topBar.updates}
        >
          <RefreshCw className="h-3.5 w-3.5 text-slate-300" />
          <span className="hidden xl:inline">{t.topBar.updates}</span>
        </button>

        {/* SR | EN Toggle */}
        <div
          className="ml-1 flex items-center rounded-lg border border-slate-800 bg-slate-900 p-0.5 text-xs font-semibold"
          aria-label={t.topBar.languageSwitch}
        >
          <button
            type="button"
            onClick={() => void setUiLanguage("sr")}
            className={`rounded-md px-2 py-1 transition ${
              lang === "sr"
                ? "bg-emerald-500/20 text-emerald-300"
                : "text-slate-400 hover:text-slate-200"
            }`}
          >
            SR
          </button>
          <button
            type="button"
            onClick={() => void setUiLanguage("en")}
            className={`rounded-md px-2 py-1 transition ${
              lang === "en"
                ? "bg-emerald-500/20 text-emerald-300"
                : "text-slate-400 hover:text-slate-200"
            }`}
          >
            EN
          </button>
        </div>
      </div>
    </header>
  );
}
