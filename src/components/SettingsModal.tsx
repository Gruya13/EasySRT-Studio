import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  Cloud,
  Cpu,
  FolderOpen,
  KeyRound,
  Plus,
  Settings,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatBytes, isTauriRuntime } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type {
  AppConfig,
  CatalogItemKind,
  SerbianScript,
} from "../types/models";

export function SettingsModal() {
  const isOpen = useAppStore((s) => s.isSettingsOpen);
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen);
  const config = useAppStore((s) => s.config);
  const catalog = useAppStore((s) => s.catalog);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const apiKeyStatus = useAppStore((s) => s.apiKeyStatus);

  const updateConfig = useAppStore((s) => s.updateConfig);
  const setUiLanguage = useAppStore((s) => s.setUiLanguage);
  const runImportCustomModel = useAppStore((s) => s.runImportCustomModel);
  const runDeleteModel = useAppStore((s) => s.runDeleteModel);
  const runSetApiKey = useAppStore((s) => s.runSetApiKey);
  const runClearApiKey = useAppStore((s) => s.runClearApiKey);

  const [activeTab, setActiveTab] = useState<"models" | "cloud">("models");
  const [draftConfig, setDraftConfig] = useState<AppConfig>(config);
  const [deepSeekKeyInput, setDeepSeekKeyInput] = useState("");
  const [customKeyInput, setCustomKeyInput] = useState("");

  useEffect(() => {
    if (isOpen) {
      setDraftConfig(config);
      setDeepSeekKeyInput("");
      setCustomKeyInput("");
    }
  }, [isOpen, config]);

  if (!isOpen) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const installedMap = new Map(
    (setupStatus?.components ?? []).map((c) => [c.id, c]),
  );

  const catalogWhisperModels = (catalog?.items ?? []).filter(
    (i) => i.kind === "whisperModel",
  );
  const catalogLlmModels = (catalog?.items ?? []).filter(
    (i) => i.kind === "llmModel",
  );
  const customWhisperModels = config.customModels.filter(
    (m) => m.kind === "whisperModel",
  );
  const customLlmModels = config.customModels.filter(
    (m) => m.kind === "llmModel",
  );

  const handleImportModelDialog = async (kind: CatalogItemKind) => {
    const ext = kind === "whisperModel" ? ["bin"] : ["gguf"];
    if (isTauriRuntime()) {
      try {
        const selected = await open({
          multiple: false,
          filters: [
            {
              name: kind === "whisperModel" ? "Whisper GGML (.bin)" : "LLM GGUF (.gguf)",
              extensions: ext,
            },
          ],
        });
        if (typeof selected === "string" && selected) {
          await runImportCustomModel(selected, kind, null);
        }
        return;
      } catch {
        // Fallback below
      }
    }
    const demoPath =
      kind === "whisperModel"
        ? "C:/Models/ggml-custom-whisper.bin"
        : "C:/Models/custom-translator-Q4_K_M.gguf";
    await runImportCustomModel(demoPath, kind, null);
  };

  const handleBrowseModelsDir = async () => {
    if (isTauriRuntime()) {
      try {
        const dir = await open({
          directory: true,
          multiple: false,
        });
        if (typeof dir === "string" && dir) {
          setDraftConfig({
            ...draftConfig,
            modelsDirOverride: dir,
          });
        }
      } catch {
        // Ignore
      }
    }
  };

  const handleSaveSettings = async () => {
    await updateConfig(draftConfig);
    setSettingsOpen(false);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div className="flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-slate-800 bg-slate-900/60 px-6 py-4">
          <div className="flex items-center gap-2.5">
            <Settings className="h-5 w-5 text-cyan-400" />
            <h2 className="text-base font-bold text-slate-100">
              {t.settings.title}
            </h2>
          </div>

          <button
            type="button"
            onClick={() => setSettingsOpen(false)}
            className="rounded-lg p-1 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-slate-800 bg-slate-900/30 px-6 pt-2">
          <button
            type="button"
            onClick={() => setActiveTab("models")}
            className={`flex items-center gap-2 border-b-2 px-4 py-2.5 text-xs font-bold transition ${
              activeTab === "models"
                ? "border-emerald-400 text-emerald-300"
                : "border-transparent text-slate-400 hover:text-slate-200"
            }`}
          >
            <Cpu className="h-4 w-4" />
            <span>{t.settings.tabModels}</span>
          </button>

          <button
            type="button"
            onClick={() => setActiveTab("cloud")}
            className={`flex items-center gap-2 border-b-2 px-4 py-2.5 text-xs font-bold transition ${
              activeTab === "cloud"
                ? "border-amber-400 text-amber-300"
                : "border-transparent text-slate-400 hover:text-slate-200"
            }`}
          >
            <Cloud className="h-4 w-4" />
            <span>{t.settings.tabCloud}</span>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 space-y-5 overflow-y-auto p-6 text-xs">
          {activeTab === "models" ? (
            <>
              {/* Active Whisper & Local LLM Selectors */}
              <div className="grid gap-4 sm:grid-cols-2">
                <div>
                  <label className="mb-1.5 block text-[11px] font-bold uppercase tracking-wider text-slate-400">
                    {t.settings.activeWhisperModel}
                  </label>
                  <select
                    value={draftConfig.activeWhisperModelId ?? ""}
                    onChange={(e) =>
                      setDraftConfig({
                        ...draftConfig,
                        activeWhisperModelId: e.target.value || null,
                      })
                    }
                    className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs text-slate-100 focus:border-cyan-500 focus:outline-none"
                  >
                    <option value="">
                      {t.settings.autoSelectRecommended}
                    </option>
                    {catalogWhisperModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.name} ({formatBytes(m.sizeBytes)})
                      </option>
                    ))}
                    {customWhisperModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        [Custom] {m.name}
                      </option>
                    ))}
                  </select>
                </div>

                <div>
                  <label className="mb-1.5 block text-[11px] font-bold uppercase tracking-wider text-slate-400">
                    {t.settings.activeLlmModel}
                  </label>
                  <select
                    value={draftConfig.activeLlmModelId ?? ""}
                    onChange={(e) =>
                      setDraftConfig({
                        ...draftConfig,
                        activeLlmModelId: e.target.value || null,
                      })
                    }
                    className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs text-slate-100 focus:border-cyan-500 focus:outline-none"
                  >
                    <option value="">
                      {t.settings.autoSelectRecommended}
                    </option>
                    {catalogLlmModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.name} ({formatBytes(m.sizeBytes)})
                      </option>
                    ))}
                    {customLlmModels.map((m) => (
                      <option key={m.id} value={m.id}>
                        [Custom] {m.name}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              {/* Import Custom .bin / .gguf */}
              <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <div className="font-bold text-slate-200">
                      {t.settings.importCustomModel}
                    </div>
                    <p className="mt-0.5 text-[11px] text-slate-400">
                      GGML Whisper (.bin) / llama.cpp (.gguf)
                    </p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      onClick={() =>
                        void handleImportModelDialog("whisperModel")
                      }
                      className="inline-flex items-center gap-1.5 rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-3 py-1.5 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/25"
                    >
                      <Plus className="h-3.5 w-3.5" />
                      <span>{t.settings.importWhisperBin}</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => void handleImportModelDialog("llmModel")}
                      className="inline-flex items-center gap-1.5 rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-200 hover:bg-emerald-500/25"
                    >
                      <Plus className="h-3.5 w-3.5" />
                      <span>{t.settings.importLlmGguf}</span>
                    </button>
                  </div>
                </div>
              </div>

              {/* Installed & Custom Models List with Delete action */}
              <div className="space-y-2">
                <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">
                  {t.settings.installedModelsList}
                </div>
                <div className="divide-y divide-slate-800/80 rounded-xl border border-slate-800 bg-slate-900/50">
                  {[...catalogWhisperModels, ...catalogLlmModels].map((m) => {
                    const status = installedMap.get(m.id);
                    if (!status?.installed) {
                      return null;
                    }
                    return (
                      <div
                        key={m.id}
                        className="flex items-center justify-between gap-3 px-3.5 py-2.5"
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 font-semibold text-slate-200">
                            <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />
                            <span>{m.name}</span>
                            <span className="font-mono text-[11px] text-slate-400">
                              ({formatBytes(m.sizeBytes)})
                            </span>
                          </div>
                          {status.path && (
                            <div className="truncate font-mono text-[10px] text-slate-500">
                              {status.path}
                            </div>
                          )}
                        </div>
                        <button
                          type="button"
                          onClick={() => void runDeleteModel(m.id)}
                          className="inline-flex items-center gap-1 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1 text-[11px] font-medium text-rose-300 hover:bg-rose-500/20"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                          <span>{t.settings.deleteModel}</span>
                        </button>
                      </div>
                    );
                  })}

                  {config.customModels.map((cm) => (
                    <div
                      key={cm.id}
                      className="flex items-center justify-between gap-3 px-3.5 py-2.5"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-2 font-semibold text-cyan-200">
                          <span>[Custom] {cm.name}</span>
                          <span className="font-mono text-[11px] text-slate-400">
                            ({formatBytes(cm.sizeBytes)})
                          </span>
                        </div>
                        <div className="truncate font-mono text-[10px] text-slate-500">
                          {cm.path}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => void runDeleteModel(cm.id)}
                        className="inline-flex items-center gap-1 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-1 text-[11px] font-medium text-rose-300 hover:bg-rose-500/20"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                        <span>{t.settings.deleteModel}</span>
                      </button>
                    </div>
                  ))}
                </div>
              </div>

              {/* Models Directory Override + Default Script & UI Language */}
              <div className="space-y-3 border-t border-slate-800 pt-4">
                <div>
                  <label className="mb-1 block text-[11px] font-bold uppercase tracking-wider text-slate-400">
                    {t.settings.modelsDirectory}
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={draftConfig.modelsDirOverride ?? ""}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          modelsDirOverride: e.target.value.trim() || null,
                        })
                      }
                      placeholder={
                        setupStatus?.storage.modelsDir ??
                        t.settings.modelsDirPlaceholder
                      }
                      className="flex-1 rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 font-mono text-xs text-slate-200 focus:border-cyan-500 focus:outline-none"
                    />
                    <button
                      type="button"
                      onClick={() => void handleBrowseModelsDir()}
                      className="inline-flex items-center gap-1.5 rounded-xl border border-slate-700 bg-slate-800 px-3 py-2 text-xs font-semibold text-slate-200 hover:bg-slate-700"
                    >
                      <FolderOpen className="h-3.5 w-3.5" />
                      <span>{t.settings.browseFolder}</span>
                    </button>
                    {draftConfig.modelsDirOverride && (
                      <button
                        type="button"
                        onClick={() =>
                          setDraftConfig({
                            ...draftConfig,
                            modelsDirOverride: null,
                          })
                        }
                        className="rounded-xl border border-slate-700 bg-slate-900 px-3 py-2 text-xs text-slate-400 hover:text-slate-200"
                      >
                        {t.settings.resetDefaultDir}
                      </button>
                    )}
                  </div>
                </div>

                <div className="grid gap-4 sm:grid-cols-2">
                  <div>
                    <label className="mb-1 block text-[11px] font-bold uppercase tracking-wider text-slate-400">
                      {t.settings.defaultScriptLabel}
                    </label>
                    <select
                      value={draftConfig.defaultScript}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          defaultScript: e.target.value as SerbianScript,
                        })
                      }
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs text-slate-100"
                    >
                      <option value="latin">{t.workspace.scriptLatin}</option>
                      <option value="cyrillic">
                        {t.workspace.scriptCyrillic}
                      </option>
                    </select>
                  </div>

                  <div>
                    <label className="mb-1 block text-[11px] font-bold uppercase tracking-wider text-slate-400">
                      {t.topBar.languageSwitch}
                    </label>
                    <select
                      value={lang}
                      onChange={(e) => {
                        const nextLang =
                          e.target.value === "sr" ? "sr" : "en";
                        setDraftConfig({
                          ...draftConfig,
                          uiLanguage: nextLang,
                        });
                        void setUiLanguage(nextLang);
                      }}
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-xs text-slate-100"
                    >
                      <option value="en">English</option>
                      <option value="sr">Srpski (Latinica)</option>
                    </select>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <>
              {/* DeepSeek API Section */}
              <div className="space-y-3 rounded-xl border border-amber-500/30 bg-amber-950/10 p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2 font-bold text-amber-200">
                    <KeyRound className="h-4 w-4 text-amber-400" />
                    <span>{t.settings.deepSeekSection}</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="rounded-md border border-slate-700 bg-slate-900 px-2 py-0.5 font-mono text-[10px] text-slate-300">
                      {t.settings.storageBackend}:{" "}
                      {apiKeyStatus.storageBackend}
                    </span>
                    <span
                      className={`rounded-full border px-2 py-0.5 text-[10px] font-semibold ${
                        apiKeyStatus.hasDeepSeekKey
                          ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-300"
                          : "border-slate-700 bg-slate-800 text-slate-400"
                      }`}
                    >
                      {apiKeyStatus.hasDeepSeekKey
                        ? t.settings.apiKeySaved
                        : t.settings.apiKeyMissing}
                    </span>
                  </div>
                </div>

                <div className="grid gap-3 sm:grid-cols-2">
                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.apiKeyLabel}
                    </label>
                    <div className="flex gap-2">
                      <input
                        type="password"
                        value={deepSeekKeyInput}
                        onChange={(e) => setDeepSeekKeyInput(e.target.value)}
                        placeholder={t.settings.apiKeyPlaceholder}
                        className="flex-1 rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                      />
                      <button
                        type="button"
                        disabled={!deepSeekKeyInput.trim()}
                        onClick={() => {
                          void runSetApiKey("deepSeek", deepSeekKeyInput);
                          setDeepSeekKeyInput("");
                        }}
                        className="rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/25 disabled:opacity-40"
                      >
                        {t.settings.saveKey}
                      </button>
                      {apiKeyStatus.hasDeepSeekKey && (
                        <button
                          type="button"
                          onClick={() => void runClearApiKey("deepSeek")}
                          className="rounded-xl border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-300 hover:bg-rose-500/20"
                        >
                          {t.settings.clearKey}
                        </button>
                      )}
                    </div>
                  </div>

                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.deepSeekModelLabel}
                    </label>
                    <input
                      type="text"
                      value={draftConfig.translation.deepseekModel}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          translation: {
                            ...draftConfig.translation,
                            deepseekModel: e.target.value,
                          },
                        })
                      }
                      placeholder="deepseek-chat"
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                  </div>
                </div>
              </div>

              {/* Custom OpenAI-Compatible API Section */}
              <div className="space-y-3 rounded-xl border border-slate-800 bg-slate-900/60 p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-bold text-slate-200">
                    {t.settings.customApiSection}
                  </span>
                  <span
                    className={`rounded-full border px-2 py-0.5 text-[10px] font-semibold ${
                      apiKeyStatus.hasCustomApiKey
                        ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-300"
                        : "border-slate-700 bg-slate-800 text-slate-400"
                    }`}
                  >
                    {apiKeyStatus.hasCustomApiKey
                      ? t.settings.apiKeySaved
                      : t.settings.apiKeyMissing}
                  </span>
                </div>

                <div className="grid gap-3 sm:grid-cols-2">
                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.customBaseUrlLabel}
                    </label>
                    <input
                      type="text"
                      value={draftConfig.translation.customBaseUrl}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          translation: {
                            ...draftConfig.translation,
                            customBaseUrl: e.target.value,
                          },
                        })
                      }
                      placeholder="https://api.openai.com/v1"
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                  </div>

                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.customModelLabel}
                    </label>
                    <input
                      type="text"
                      value={draftConfig.translation.customModel}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          translation: {
                            ...draftConfig.translation,
                            customModel: e.target.value,
                          },
                        })
                      }
                      placeholder="gpt-4o-mini"
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                  </div>
                </div>

                <div>
                  <label className="mb-1 block text-[11px] text-slate-400">
                    {t.settings.apiKeyLabel} (Custom API)
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="password"
                      value={customKeyInput}
                      onChange={(e) => setCustomKeyInput(e.target.value)}
                      placeholder={t.settings.apiKeyPlaceholder}
                      className="flex-1 rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                    <button
                      type="button"
                      disabled={!customKeyInput.trim()}
                      onClick={() => {
                        void runSetApiKey("customOpenAi", customKeyInput);
                        setCustomKeyInput("");
                      }}
                      className="rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/25 disabled:opacity-40"
                    >
                      {t.settings.saveKey}
                    </button>
                    {apiKeyStatus.hasCustomApiKey && (
                      <button
                        type="button"
                        onClick={() => void runClearApiKey("customOpenAi")}
                        className="rounded-xl border border-rose-500/30 bg-rose-500/10 px-2.5 py-1.5 text-xs text-rose-300 hover:bg-rose-500/20"
                      >
                        {t.settings.clearKey}
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {/* Batch Size & Context Window */}
              <div className="space-y-3 rounded-xl border border-slate-800 bg-slate-900/60 p-4">
                <div className="font-bold text-slate-200">
                  {t.settings.batchParamsSection}
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.batchSizeLabel}
                    </label>
                    <input
                      type="number"
                      min={5}
                      max={60}
                      value={draftConfig.translation.batchSize}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          translation: {
                            ...draftConfig.translation,
                            batchSize: Math.max(
                              5,
                              Number.parseInt(e.target.value, 10) || 20,
                            ),
                          },
                        })
                      }
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                  </div>

                  <div>
                    <label className="mb-1 block text-[11px] text-slate-400">
                      {t.settings.contextWindowLabel}
                    </label>
                    <input
                      type="number"
                      min={2048}
                      max={32768}
                      step={1024}
                      value={draftConfig.translation.contextWindow}
                      onChange={(e) =>
                        setDraftConfig({
                          ...draftConfig,
                          translation: {
                            ...draftConfig.translation,
                            contextWindow: Math.max(
                              2048,
                              Number.parseInt(e.target.value, 10) || 4096,
                            ),
                          },
                        })
                      }
                      className="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200"
                    />
                  </div>
                </div>
              </div>
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2.5 border-t border-slate-800 bg-slate-900/70 px-6 py-3.5">
          <button
            type="button"
            onClick={() => setSettingsOpen(false)}
            className="rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 transition hover:bg-slate-700"
          >
            {t.settings.close}
          </button>

          <button
            type="button"
            onClick={() => void handleSaveSettings()}
            className="rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400"
          >
            {t.settings.saveSettings}
          </button>
        </div>
      </div>
    </div>
  );
}
