import {
  AlertTriangle,
  CheckCircle2,
  Cpu,
  Download,
  HardDrive,
  Loader2,
  Sparkles,
  X,
} from "lucide-react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatBytes } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type { CatalogItem, VramTier } from "../types/models";

export function SetupWizard() {
  const isOpen = useAppStore((s) => s.isSetupOpen);
  const setSetupOpen = useAppStore((s) => s.setSetupOpen);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const catalog = useAppStore((s) => s.catalog);
  const config = useAppStore((s) => s.config);
  const downloadProgress = useAppStore((s) => s.downloadProgress);
  const installingBundle = useAppStore((s) => s.installingBundle);
  const installingItemIds = useAppStore((s) => s.installingItemIds);
  const runInstallComponent = useAppStore((s) => s.runInstallComponent);
  const runInstallRecommendedBundle = useAppStore(
    (s) => s.runInstallRecommendedBundle,
  );
  const updateConfig = useAppStore((s) => s.updateConfig);

  if (!isOpen) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const gpu = setupStatus?.gpu;
  const storage = setupStatus?.storage;
  const installedMap = new Map(
    (setupStatus?.components ?? []).map((c) => [c.id, c.installed]),
  );

  const formatTierLabel = (tier?: VramTier | null) => {
    switch (tier) {
      case "vram12GbPlus":
        return t.setup.tier12Gb;
      case "vram8Gb":
        return t.setup.tier8Gb;
      case "vram4Gb":
        return t.setup.tier4Gb;
      default:
        return t.setup.tierCpu;
    }
  };

  const items = catalog?.items ?? [];
  const binaryItems = items.filter(
    (i) =>
      i.kind === "ffmpeg" ||
      i.kind === "cudaRuntime" ||
      i.kind === "whisperBin" ||
      i.kind === "llamaBin" ||
      i.kind === "vadModel",
  );
  const whisperModels = items.filter((i) => i.kind === "whisperModel");
  const llmModels = items.filter((i) => i.kind === "llmModel");

  const handleClose = () => {
    void updateConfig({ hasCompletedSetup: true });
    setSetupOpen(false);
  };

  const renderItemCard = (item: CatalogItem) => {
    const isInstalled = installedMap.get(item.id) ?? false;
    const isInstalling = Boolean(installingItemIds[item.id]);
    const prog = downloadProgress[item.id];
    const isRecommendedTier =
      item.vramTier !== null && item.vramTier === gpu?.recommendedTier;

    return (
      <div
        key={item.id}
        className={`flex flex-col justify-between gap-3 rounded-xl border p-3.5 transition ${
          isInstalled
            ? "border-emerald-500/30 bg-emerald-950/15"
            : isRecommendedTier
              ? "border-cyan-500/35 bg-slate-900/90"
              : "border-slate-800 bg-slate-900/60"
        }`}
      >
        <div className="space-y-1.5">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-slate-100">
                {item.name}
              </span>
              {item.required && (
                <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-amber-300">
                  {t.setup.required}
                </span>
              )}
              {isRecommendedTier && (
                <span className="rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-cyan-300">
                  {t.setup.recommendedForGpu}
                </span>
              )}
            </div>
            <span className="font-mono text-xs text-slate-400">
              {formatBytes(item.sizeBytes)}
            </span>
          </div>

          <p className="text-xs leading-relaxed text-slate-400">
            {item.description}
          </p>
        </div>

        <div className="space-y-2 pt-1">
          {prog && (isInstalling || installingBundle) && (
            <div className="space-y-1">
              <div className="flex items-center justify-between font-mono text-[11px] text-cyan-300">
                <span>
                  {prog.stage === "extracting"
                    ? t.setup.extracting
                    : prog.stage === "verifying"
                      ? t.setup.verifying
                      : t.setup.downloading}
                </span>
                <span>
                  {Math.round(prog.percent)}% (
                  {formatBytes(prog.downloadedBytes)} /{" "}
                  {formatBytes(prog.totalBytes || item.sizeBytes)})
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                <div
                  className="h-full rounded-full bg-gradient-to-r from-emerald-400 to-cyan-400 transition-all duration-200"
                  style={{
                    width: `${Math.min(100, Math.max(4, prog.percent))}%`,
                  }}
                />
              </div>
            </div>
          )}

          <div className="flex items-center justify-between">
            <span className="font-mono text-[11px] text-slate-500">
              {item.platform} • v{item.version}
            </span>

            {isInstalled ? (
              <span className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-1 text-xs font-medium text-emerald-300">
                <CheckCircle2 className="h-3.5 w-3.5" />
                {t.setup.installed}
              </span>
            ) : (
              <button
                type="button"
                disabled={isInstalling || installingBundle}
                onClick={() => void runInstallComponent(item.id)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-cyan-500/40 bg-cyan-500/15 px-3 py-1 text-xs font-medium text-cyan-200 transition hover:bg-cyan-500/25 disabled:opacity-50"
              >
                {isInstalling ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    <span>{t.setup.downloading}</span>
                  </>
                ) : (
                  <>
                    <Download className="h-3.5 w-3.5" />
                    <span>{t.setup.download}</span>
                  </>
                )}
              </button>
            )}
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div className="flex max-h-[90vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl">
        {/* Modal Header */}
        <div className="flex items-start justify-between gap-4 border-b border-slate-800 bg-slate-900/60 px-6 py-4">
          <div>
            <h2 className="text-lg font-bold text-slate-100">
              {t.setup.title}
            </h2>
            <p className="mt-0.5 text-xs text-slate-400">{t.setup.subtitle}</p>
          </div>
          <button
            type="button"
            onClick={handleClose}
            className="rounded-lg p-1.5 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Modal Body */}
        <div className="flex-1 space-y-6 overflow-y-auto p-6">
          {/* Hardware & One-Click Recommended Bundle Card */}
          <div className="grid gap-4 rounded-2xl border border-emerald-500/30 bg-gradient-to-br from-emerald-950/30 via-slate-900 to-cyan-950/20 p-5 lg:grid-cols-3">
            <div className="space-y-2 lg:col-span-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-0.5 text-xs font-semibold text-emerald-300">
                  <Cpu className="h-3.5 w-3.5" />
                  {t.setup.detectedHardware}
                </span>
                {storage && (
                  <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-700 bg-slate-800/90 px-2.5 py-0.5 font-mono text-xs text-slate-300">
                    <HardDrive className="h-3.5 w-3.5 text-cyan-400" />
                    {t.setup.storageMode}: {storage.mode.toUpperCase()}
                  </span>
                )}
              </div>

              <div className="text-base font-bold text-slate-100">
                {gpu?.hasNvidia && gpu.gpuName
                  ? `${gpu.gpuName} • ${gpu.vramMb} MB VRAM`
                  : t.setup.noNvidiaGpu}
              </div>

              <div className="text-xs text-slate-300">
                <span className="text-slate-400">
                  {t.setup.recommendedTier}:{" "}
                </span>
                <span className="font-semibold text-cyan-300">
                  {formatTierLabel(gpu?.recommendedTier)}
                </span>
              </div>

              <div className="pt-1">
                {setupStatus?.readyForOffline ? (
                  <div className="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-400">
                    <CheckCircle2 className="h-4 w-4" />
                    <span>{t.setup.allRequiredReady}</span>
                  </div>
                ) : (
                  <div className="inline-flex items-center gap-1.5 text-xs font-medium text-amber-300">
                    <AlertTriangle className="h-4 w-4" />
                    <span>{t.setup.missingRequiredWarn}</span>
                  </div>
                )}
              </div>
            </div>

            <div className="flex flex-col justify-center gap-2">
              <button
                type="button"
                disabled={installingBundle}
                onClick={() => void runInstallRecommendedBundle()}
                className="flex w-full items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-3 text-sm font-bold text-slate-950 shadow-lg shadow-emerald-500/10 transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-60"
              >
                {installingBundle ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    <span>{t.setup.installingBundle}</span>
                  </>
                ) : (
                  <>
                    <Sparkles className="h-4 w-4" />
                    <span>{t.setup.installRecommendedBtn}</span>
                  </>
                )}
              </button>
            </div>
          </div>

          {/* Category 1: Binaries & CUDA */}
          <section className="space-y-3">
            <h3 className="text-xs font-bold uppercase tracking-wider text-slate-400">
              {t.setup.categoryBinaries}
            </h3>
            <div className="grid gap-3 md:grid-cols-2">
              {binaryItems.map(renderItemCard)}
            </div>
          </section>

          {/* Category 2: Whisper ASR Models */}
          <section className="space-y-3">
            <h3 className="text-xs font-bold uppercase tracking-wider text-slate-400">
              {t.setup.categoryWhisper}
            </h3>
            <div className="grid gap-3 md:grid-cols-2">
              {whisperModels.map(renderItemCard)}
            </div>
          </section>

          {/* Category 3: Local LLM Translators */}
          <section className="space-y-3">
            <h3 className="text-xs font-bold uppercase tracking-wider text-slate-400">
              {t.setup.categoryLlm}
            </h3>
            <div className="grid gap-3 md:grid-cols-2">
              {llmModels.map(renderItemCard)}
            </div>
          </section>
        </div>

        {/* Modal Footer */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-slate-800 bg-slate-900/70 px-6 py-3.5">
          <button
            type="button"
            onClick={handleClose}
            className="text-xs font-medium text-slate-400 underline-offset-4 hover:text-slate-200 hover:underline"
          >
            {t.setup.skipOrClose}
          </button>

          <button
            type="button"
            onClick={handleClose}
            className="rounded-xl border border-emerald-500/40 bg-emerald-500/20 px-4 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/30"
          >
            {t.setup.continueToStudio}
          </button>
        </div>
      </div>
    </div>
  );
}
