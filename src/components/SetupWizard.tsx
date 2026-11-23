import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Cloud,
  Cpu,
  Download,
  KeyRound,
  Loader2,
  Sparkles,
  X,
} from "lucide-react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatBytes } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type {
  CatalogItem,
  TranslationProviderKind,
  VramTier,
} from "../types/models";

export function SetupWizard() {
  const isOpen = useAppStore((s) => s.isSetupOpen);
  const setSetupOpen = useAppStore((s) => s.setSetupOpen);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const catalog = useAppStore((s) => s.catalog);
  const config = useAppStore((s) => s.config);
  const apiKeyStatus = useAppStore((s) => s.apiKeyStatus);
  const downloadProgress = useAppStore((s) => s.downloadProgress);
  const installingBundle = useAppStore((s) => s.installingBundle);
  const installingItemIds = useAppStore((s) => s.installingItemIds);
  const runInstallComponent = useAppStore((s) => s.runInstallComponent);
  const runInstallCoreRuntime = useAppStore((s) => s.runInstallCoreRuntime);
  const runSetApiKey = useAppStore((s) => s.runSetApiKey);
  const setEffectiveProvider = useAppStore((s) => s.setEffectiveProvider);
  const updateConfig = useAppStore((s) => s.updateConfig);

  const [showAdvanced, setShowAdvanced] = useState(false);
  const [cloudProviderDraft, setCloudProviderDraft] = useState<
    "deepSeek" | "customOpenAi"
  >(
    config.translation.provider === "customOpenAi"
      ? "customOpenAi"
      : "deepSeek",
  );
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [customBaseUrlDraft, setCustomBaseUrlDraft] = useState(
    config.translation.customBaseUrl,
  );
  const [customModelDraft, setCustomModelDraft] = useState(
    config.translation.customModel,
  );

  if (!isOpen) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const gpu = setupStatus?.gpu;
  const currentPlatform = setupStatus?.platform ?? "windows-x64";
  const coreReady = setupStatus?.coreRuntimeReady ?? false;
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

  // Strictly filter catalog to current OS platform + cross-platform models ("all")
  const platformItems = (catalog?.items ?? []).filter(
    (i) => i.platform === "all" || i.platform === currentPlatform,
  );
  const binaryItems = platformItems.filter(
    (i) =>
      i.kind === "ffmpeg" ||
      i.kind === "cudaRuntime" ||
      i.kind === "whisperBin" ||
      i.kind === "llamaBin" ||
      i.kind === "vadModel",
  );
  const whisperModels = platformItems.filter((i) => i.kind === "whisperModel");
  const llmModels = platformItems.filter((i) => i.kind === "llmModel");
  const coreAdvancedItems = [...binaryItems, ...whisperModels];
  const installedCoreCount = coreAdvancedItems.filter(
    (i) => installedMap.get(i.id) ?? false,
  ).length;
  const isAdvancedExpanded = showAdvanced || !coreReady;

  const hasSavedSelectedKey =
    cloudProviderDraft === "customOpenAi"
      ? apiKeyStatus.hasCustomApiKey
      : apiKeyStatus.hasDeepSeekKey;
  const isSelectedCloudActive =
    config.translation.provider === cloudProviderDraft;

  const handleClose = () => {
    void updateConfig({ hasCompletedSetup: true });
    setSetupOpen(false);
  };

  const handleActivateLocalLlm = (modelId: string) => {
    void updateConfig({
      activeLlmModelId: modelId,
      translation: {
        ...config.translation,
        provider: "localLlama",
      },
    });
    void setEffectiveProvider("localLlama");
  };

  const handleSaveCloudProvider = async () => {
    const provider: TranslationProviderKind = cloudProviderDraft;
    if (cloudProviderDraft === "customOpenAi") {
      await updateConfig({
        translation: {
          ...config.translation,
          provider,
          customBaseUrl:
            customBaseUrlDraft.trim() || config.translation.customBaseUrl,
          customModel:
            customModelDraft.trim() || config.translation.customModel,
        },
      });
    }
    if (apiKeyDraft.trim()) {
      await runSetApiKey(provider, apiKeyDraft.trim());
      setApiKeyDraft("");
    }
    await setEffectiveProvider(provider);
  };

  const renderItemCard = (item: CatalogItem, allowModelActivation = false) => {
    const isInstalled = installedMap.get(item.id) ?? false;
    const isInstalling = Boolean(installingItemIds[item.id]);
    const prog = downloadProgress[item.id];
    const isRecommendedTier =
      item.vramTier !== null && item.vramTier === gpu?.recommendedTier;
    const isActiveLlm =
      allowModelActivation &&
      item.kind === "llmModel" &&
      config.translation.provider === "localLlama" &&
      config.activeLlmModelId === item.id;

    return (
      <div
        key={item.id}
        className={`flex flex-col justify-between gap-3 rounded-xl border p-3.5 transition ${
          isActiveLlm
            ? "border-emerald-400/60 bg-emerald-950/25"
            : isInstalled
              ? "border-emerald-500/30 bg-emerald-950/15"
              : isRecommendedTier
                ? "border-cyan-500/35 bg-slate-900/90"
                : "border-slate-800 bg-slate-900/60"
        }`}
      >
        <div className="space-y-1.5">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-semibold text-slate-100">
                {item.name}
              </span>
              {item.required && !allowModelActivation && (
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

          <div className="flex items-center justify-between gap-2">
            <span className="font-mono text-[11px] text-slate-500">
              {item.platform} • v{item.version}
            </span>

            {isInstalled ? (
              allowModelActivation ? (
                isActiveLlm ? (
                  <span className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-400/40 bg-emerald-500/20 px-2.5 py-1 text-xs font-semibold text-emerald-200">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    {t.setup.activeModel}
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={() => handleActivateLocalLlm(item.id)}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-cyan-500/40 bg-cyan-500/15 px-3 py-1 text-xs font-medium text-cyan-200 transition hover:bg-cyan-500/25"
                  >
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    <span>{t.setup.selectModel}</span>
                  </button>
                )
              ) : (
                <span className="inline-flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-1 text-xs font-medium text-emerald-300">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  {t.setup.installed}
                </span>
              )
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
          {/* Core Runtime Status Card */}
          <div
            className={`grid gap-4 rounded-2xl border p-5 lg:grid-cols-3 ${
              coreReady
                ? "border-emerald-500/30 bg-gradient-to-br from-emerald-950/30 via-slate-900 to-cyan-950/20"
                : "border-amber-500/35 bg-gradient-to-br from-amber-950/25 via-slate-900 to-slate-950"
            }`}
          >
            <div className="space-y-2 lg:col-span-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-0.5 text-xs font-semibold text-emerald-300">
                  <Cpu className="h-3.5 w-3.5" />
                  {t.setup.detectedHardware}
                </span>
                <span className="rounded-full border border-slate-700 bg-slate-800/90 px-2.5 py-0.5 font-mono text-[11px] text-slate-300">
                  {currentPlatform}
                </span>
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

              {gpu?.hasNvidia &&
                gpu.cudaSupported === false &&
                gpu.cudaStatusNote && (
                  <div className="mt-1.5 flex items-start gap-2 rounded-xl border border-amber-500/40 bg-amber-950/35 px-3 py-2 text-xs leading-relaxed text-amber-200">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" />
                    <span>{gpu.cudaStatusNote}</span>
                  </div>
                )}

              <div className="pt-1">
                {coreReady ? (
                  <div className="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-400">
                    <CheckCircle2 className="h-4 w-4 shrink-0" />
                    <span>{t.setup.coreReady}</span>
                  </div>
                ) : (
                  <div className="inline-flex items-center gap-1.5 text-xs font-medium text-amber-300">
                    <AlertTriangle className="h-4 w-4 shrink-0" />
                    <span>{t.setup.coreMissing}</span>
                  </div>
                )}
              </div>
            </div>

            <div className="flex flex-col justify-center gap-2">
              {coreReady ? (
                <div className="rounded-xl border border-emerald-500/25 bg-emerald-950/30 px-4 py-3 text-center text-xs font-medium text-emerald-200">
                  {setupStatus?.hasLlmModel ||
                  (config.translation.provider !== "localLlama" &&
                    (apiKeyStatus.hasDeepSeekKey ||
                      apiKeyStatus.hasCustomApiKey))
                    ? t.setup.allRequiredReady
                    : t.setup.missingRequiredWarn}
                </div>
              ) : (
                <button
                  type="button"
                  disabled={installingBundle}
                  onClick={() => void runInstallCoreRuntime()}
                  className="flex w-full items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-3 text-sm font-bold text-slate-950 shadow-lg shadow-emerald-500/10 transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-60"
                >
                  {installingBundle ? (
                    <>
                      <Loader2 className="h-4 w-4 animate-spin" />
                      <span>{t.setup.installingCore}</span>
                    </>
                  ) : (
                    <>
                      <Sparkles className="h-4 w-4" />
                      <span>{t.setup.installCoreBtn}</span>
                    </>
                  )}
                </button>
              )}
            </div>
          </div>

          {/* Primary Section 1: Local LLM Translation Models */}
          <section className="space-y-3">
            <div>
              <h3 className="text-xs font-bold uppercase tracking-wider text-slate-200">
                {t.setup.translationHeading}
              </h3>
              <p className="mt-0.5 text-xs text-slate-400">
                {t.setup.translationSubtitle}
              </p>
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              {llmModels.map((item) => renderItemCard(item, true))}
            </div>
          </section>

          {/* Primary Section 2: Cloud API Provider Key */}
          <section className="space-y-3 rounded-2xl border border-slate-800 bg-slate-900/50 p-4">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div>
                <h3 className="flex items-center gap-2 text-xs font-bold uppercase tracking-wider text-slate-200">
                  <Cloud className="h-4 w-4 text-cyan-400" />
                  <span>{t.setup.cloudSectionTitle}</span>
                </h3>
                <p className="mt-0.5 text-xs text-slate-400">
                  {t.setup.cloudSectionSubtitle}
                </p>
              </div>
              {isSelectedCloudActive && hasSavedSelectedKey && (
                <span className="inline-flex items-center gap-1 rounded-full border border-emerald-500/35 bg-emerald-500/15 px-2.5 py-0.5 text-[11px] font-semibold text-emerald-300">
                  <CheckCircle2 className="h-3 w-3" />
                  {t.setup.cloudActiveBadge}
                </span>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-2">
              {(["deepSeek", "customOpenAi"] as const).map((prov) => (
                <button
                  key={prov}
                  type="button"
                  onClick={() => setCloudProviderDraft(prov)}
                  className={`rounded-lg border px-3 py-1.5 text-xs font-semibold transition ${
                    cloudProviderDraft === prov
                      ? "border-cyan-500/50 bg-cyan-500/20 text-cyan-200"
                      : "border-slate-700 bg-slate-900 text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {prov === "deepSeek" ? "DeepSeek API" : "Custom OpenAI API"}
                </button>
              ))}
              <span
                className={`ml-auto rounded-full border px-2.5 py-0.5 text-[11px] font-medium ${
                  hasSavedSelectedKey
                    ? "border-emerald-500/30 bg-emerald-500/15 text-emerald-300"
                    : "border-slate-700 bg-slate-800 text-slate-400"
                }`}
              >
                {hasSavedSelectedKey
                  ? t.settings.apiKeySaved
                  : t.settings.apiKeyMissing}
              </span>
            </div>

            {cloudProviderDraft === "customOpenAi" && (
              <div className="grid gap-3 md:grid-cols-2">
                <input
                  type="text"
                  value={customBaseUrlDraft}
                  onChange={(e) => setCustomBaseUrlDraft(e.target.value)}
                  placeholder="https://api.openai.com/v1"
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-xs text-slate-100 placeholder:text-slate-500"
                />
                <input
                  type="text"
                  value={customModelDraft}
                  onChange={(e) => setCustomModelDraft(e.target.value)}
                  placeholder="gpt-4o-mini"
                  className="rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-xs text-slate-100 placeholder:text-slate-500"
                />
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2">
              <div className="relative flex-1 min-w-[220px]">
                <KeyRound className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
                <input
                  type="password"
                  value={apiKeyDraft}
                  onChange={(e) => setApiKeyDraft(e.target.value)}
                  placeholder={t.settings.apiKeyPlaceholder}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void handleSaveCloudProvider();
                    }
                  }}
                  className="w-full rounded-lg border border-slate-700 bg-slate-950 py-2 pl-9 pr-3 font-mono text-xs text-slate-100 placeholder:text-slate-500"
                />
              </div>
              <button
                type="button"
                disabled={!apiKeyDraft.trim() && !hasSavedSelectedKey}
                onClick={() => void handleSaveCloudProvider()}
                className="rounded-lg border border-cyan-500/40 bg-cyan-500/20 px-4 py-2 text-xs font-semibold text-cyan-100 transition hover:bg-cyan-500/30 disabled:opacity-50"
              >
                {t.setup.cloudActivateBtn}
              </button>
            </div>
          </section>

          {/* Collapsible Advanced Section: Platform Core Binaries & Whisper ASR Models */}
          <section className="rounded-2xl border border-slate-800/90 bg-slate-900/35">
            <button
              type="button"
              onClick={() => setShowAdvanced((prev) => !prev)}
              className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left transition hover:bg-slate-900/60"
            >
              <div>
                <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-wider text-slate-300">
                  {isAdvancedExpanded ? (
                    <ChevronDown className="h-4 w-4 text-slate-400" />
                  ) : (
                    <ChevronRight className="h-4 w-4 text-slate-400" />
                  )}
                  <span>{t.setup.advancedSectionTitle}</span>
                </div>
                <p className="mt-0.5 pl-6 text-xs text-slate-500">
                  {t.setup.advancedSectionSubtitle}
                </p>
              </div>
              <span className="rounded-full border border-slate-700 bg-slate-900 px-2.5 py-0.5 font-mono text-[11px] text-slate-300">
                {installedCoreCount}/{coreAdvancedItems.length}
              </span>
            </button>

            {isAdvancedExpanded && (
              <div className="space-y-5 border-t border-slate-800/80 p-4">
                <div className="space-y-2.5">
                  <h4 className="text-[11px] font-bold uppercase tracking-wider text-slate-400">
                    {t.setup.categoryBinaries}
                  </h4>
                  <div className="grid gap-3 md:grid-cols-2">
                    {binaryItems.map((item) => renderItemCard(item, false))}
                  </div>
                </div>

                <div className="space-y-2.5">
                  <h4 className="text-[11px] font-bold uppercase tracking-wider text-slate-400">
                    {t.setup.categoryWhisper}
                  </h4>
                  <div className="grid gap-3 md:grid-cols-2">
                    {whisperModels.map((item) => renderItemCard(item, false))}
                  </div>
                </div>
              </div>
            )}
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
