import { useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import {
  formatBytes,
  formatDurationShort,
  isTauriRuntime,
} from "../lib/subtitleUtils";
import { openDialogPreservingMaximize } from "../lib/tauri";
import { useAppStore } from "../stores/appStore";
import type {
  CatalogItemKind,
  ExportFormat,
  SegmentPreset,
  SerbianScript,
  TranslationProviderKind,
} from "../types/models";

type SettingsTabId = "obrada" | "titlovi" | "modeli" | "izvoz" | "opste";

interface NumericFieldInputProps {
  id: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  isFloat?: boolean;
  onCommit: (nextValue: number) => void;
}

function NumericFieldInput({
  id,
  value,
  min,
  max,
  step = 1,
  isFloat = false,
  onCommit,
}: NumericFieldInputProps) {
  const [draft, setDraft] = useState(String(value));
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    if (!focused) {
      setDraft(String(value));
    }
  }, [value, focused]);

  const parseDraft = (raw: string): number | null => {
    const num = isFloat ? Number.parseFloat(raw) : Number.parseInt(raw, 10);
    return Number.isFinite(num) ? num : null;
  };

  const commitClamped = () => {
    setFocused(false);
    const parsed = parseDraft(draft);
    if (parsed === null) {
      setDraft(String(value));
      return;
    }
    const clamped = Math.max(min, Math.min(max, parsed));
    const normalized = isFloat ? Number(clamped.toFixed(1)) : Math.round(clamped);
    setDraft(String(normalized));
    if (normalized !== value) {
      onCommit(normalized);
    }
  };

  return (
    <input
      id={id}
      className="s-input mono-inp"
      type="number"
      min={min}
      max={max}
      step={step}
      value={draft}
      onFocus={() => setFocused(true)}
      onChange={(e) => {
        const raw = e.target.value;
        setDraft(raw);
        const parsed = parseDraft(raw);
        if (parsed !== null && parsed >= min && parsed <= max) {
          const normalized = isFloat
            ? Number(parsed.toFixed(1))
            : Math.round(parsed);
          if (normalized !== value) {
            onCommit(normalized);
          }
        }
      }}
      onBlur={commitClamped}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.currentTarget.blur();
        } else if (e.key === "Escape") {
          setDraft(String(value));
          setFocused(false);
          e.currentTarget.blur();
        }
      }}
    />
  );
}

export function SettingsPanel() {
  const config = useAppStore((s) => s.config);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const catalog = useAppStore((s) => s.catalog);
  const apiKeyStatus = useAppStore((s) => s.apiKeyStatus);
  const checkpoints = useAppStore((s) => s.checkpoints);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const activeProject = useAppStore((s) => s.activeProject);
  const isRunningBatch = useAppStore((s) => s.isRunningBatch);

  const setJobOptions = useAppStore((s) => s.setJobOptions);
  const setEffectiveProvider = useAppStore((s) => s.setEffectiveProvider);
  const setSegmentPreset = useAppStore((s) => s.setSegmentPreset);
  const updateSegmentRules = useAppStore((s) => s.updateSegmentRules);
  const applyResegmentToActiveProject = useAppStore(
    (s) => s.applyResegmentToActiveProject,
  );
  const convertActiveProjectScript = useAppStore(
    (s) => s.convertActiveProjectScript,
  );
  const condenseActiveProjectCps = useAppStore(
    (s) => s.condenseActiveProjectCps,
  );
  const updateConfig = useAppStore((s) => s.updateConfig);
  const setUiLanguage = useAppStore((s) => s.setUiLanguage);
  const setSetupOpen = useAppStore((s) => s.setSetupOpen);
  const setUpdaterOpen = useAppStore((s) => s.setUpdaterOpen);
  const setBugReportOpen = useAppStore((s) => s.setBugReportOpen);
  const appVersion = useAppStore((s) => s.appVersion);
  const availableUpdateVersion = useAppStore(
    (s) => s.availableUpdateVersion,
  );
  const runImportCustomModel = useAppStore((s) => s.runImportCustomModel);
  const runSetApiKey = useAppStore((s) => s.runSetApiKey);
  const runClearApiKey = useAppStore((s) => s.runClearApiKey);
  const runExportSubtitles = useAppStore((s) => s.runExportSubtitles);
  const loadCheckpointIntoEditor = useAppStore(
    (s) => s.loadCheckpointIntoEditor,
  );
  const removeCheckpoint = useAppStore((s) => s.removeCheckpoint);

  const [activeTab, setActiveTab] = useState<SettingsTabId>("obrada");
  const [glossaryInput, setGlossaryInput] = useState("");
  const [segFlash, setSegFlash] = useState(false);
  const [modelsDirDraft, setModelsDirDraft] = useState(
    config.modelsDirOverride ?? "",
  );
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [cloudEndpointDraft, setCloudEndpointDraft] = useState(
    config.translation.deepseekModel,
  );
  const [customModelDraft, setCustomModelDraft] = useState(
    config.translation.customModel,
  );
  const [exportDirDraft, setExportDirDraft] = useState(
    config.export.outputDir ?? "",
  );
  const [autoCheckpoints, setAutoCheckpoints] = useState(true);
  const [selectedCheckpointId, setSelectedCheckpointId] = useState<string>(
    checkpoints[0]?.jobId ?? "",
  );

  const segFlashTimerRef = useRef<number | null>(null);
  const modelImportInputRef = useRef<HTMLInputElement | null>(null);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const effectiveProvider: TranslationProviderKind =
    jobOptions.providerOverride ?? config.translation.provider;

  const [lastOnlineProvider, setLastOnlineProvider] = useState<
    "deepSeek" | "customOpenAi"
  >(effectiveProvider === "customOpenAi" ? "customOpenAi" : "deepSeek");

  useEffect(() => {
    if (
      effectiveProvider === "deepSeek" ||
      effectiveProvider === "customOpenAi"
    ) {
      setLastOnlineProvider(effectiveProvider);
    }
  }, [effectiveProvider]);

  useEffect(() => {
    setModelsDirDraft(config.modelsDirOverride ?? "");
  }, [config.modelsDirOverride]);

  useEffect(() => {
    setExportDirDraft(config.export.outputDir ?? "");
  }, [config.export.outputDir]);

  useEffect(() => {
    if (effectiveProvider === "deepSeek") {
      setCloudEndpointDraft(config.translation.deepseekModel || "deepseek-chat");
    } else if (effectiveProvider === "customOpenAi") {
      setCloudEndpointDraft(
        config.translation.customBaseUrl || "https://api.openai.com/v1",
      );
      setCustomModelDraft(config.translation.customModel || "gpt-4o-mini");
    } else {
      setCloudEndpointDraft("local llama-server");
    }
  }, [
    effectiveProvider,
    config.translation.deepseekModel,
    config.translation.customBaseUrl,
    config.translation.customModel,
  ]);

  useEffect(() => {
    if (
      checkpoints.length > 0 &&
      !checkpoints.some((c) => c.jobId === selectedCheckpointId)
    ) {
      setSelectedCheckpointId(checkpoints[0].jobId);
    }
  }, [checkpoints, selectedCheckpointId]);

  useEffect(() => {
    return () => {
      if (segFlashTimerRef.current !== null) {
        window.clearTimeout(segFlashTimerRef.current);
      }
    };
  }, []);

  const warningCount = useMemo(() => {
    if (!activeProject) return 0;
    const maxCps = jobOptions.segmentRules.maxCps;
    const maxChars = jobOptions.segmentRules.maxCharsPerLine;
    return activeProject.cues.filter(
      (cue) =>
        cue.cps > maxCps ||
        cue.warnings.length > 0 ||
        (cue.translatedText || cue.sourceText)
          .split("\n")
          .some((line) => [...line].length > maxChars),
    ).length;
  }, [
    activeProject,
    jobOptions.segmentRules.maxCps,
    jobOptions.segmentRules.maxCharsPerLine,
  ]);

  const catalogWhisperModels = useMemo(
    () => (catalog?.items ?? []).filter((i) => i.kind === "whisperModel"),
    [catalog],
  );
  const catalogLlmModels = useMemo(
    () => (catalog?.items ?? []).filter((i) => i.kind === "llmModel"),
    [catalog],
  );
  const customWhisperModels = useMemo(
    () => config.customModels.filter((m) => m.kind === "whisperModel"),
    [config.customModels],
  );
  const customLlmModels = useMemo(
    () => config.customModels.filter((m) => m.kind === "llmModel"),
    [config.customModels],
  );

  const glossaryEntries = Object.entries(jobOptions.glossary);

  const handleAddGlossaryTerm = () => {
    const raw = glossaryInput.trim();
    if (!raw) return;
    const arrowParts = raw.split(/\s*(?:->|=)\s*/);
    const key = arrowParts[0]?.trim();
    if (!key) return;
    const val = arrowParts[1]?.trim() || key;
    setJobOptions({
      glossary: {
        ...jobOptions.glossary,
        [key]: val,
      },
    });
    setGlossaryInput("");
  };

  const handleRemoveGlossaryTerm = (key: string) => {
    const next = { ...jobOptions.glossary };
    delete next[key];
    setJobOptions({ glossary: next });
  };

  const handlePresetClick = (preset: SegmentPreset) => {
    setSegmentPreset(preset);
  };

  const handleApplySeg = async () => {
    await applyResegmentToActiveProject();
    setSegFlash(true);
    if (segFlashTimerRef.current !== null) {
      window.clearTimeout(segFlashTimerRef.current);
    }
    segFlashTimerRef.current = window.setTimeout(() => {
      setSegFlash(false);
    }, 2200);
  };

  const handleBrowseModelsDir = async () => {
    if (isTauriRuntime()) {
      try {
        const dir = await openDialogPreservingMaximize({
          directory: true,
          multiple: false,
        });
        if (typeof dir === "string" && dir) {
          setModelsDirDraft(dir);
          await updateConfig({ modelsDirOverride: dir });
        }
        return;
      } catch {
        // Ignore dialog cancel
      }
    }
  };

  const handleImportModelClick = async () => {
    if (isTauriRuntime()) {
      try {
        const selected = await openDialogPreservingMaximize({
          multiple: false,
          filters: [
            {
              name: "AI Models (.bin, .gguf)",
              extensions: ["bin", "gguf"],
            },
          ],
        });
        if (typeof selected === "string" && selected) {
          const kind: CatalogItemKind = selected.toLowerCase().endsWith(".bin")
            ? "whisperModel"
            : "llmModel";
          await runImportCustomModel(selected, kind, null);
        }
        return;
      } catch {
        // Fallback below
      }
    }
    modelImportInputRef.current?.click();
  };

  const handleBrowserModelImport = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const path = (file as File & { path?: string }).path || file.name;
    const kind: CatalogItemKind = path.toLowerCase().endsWith(".bin")
      ? "whisperModel"
      : "llmModel";
    void runImportCustomModel(path, kind, file.name);
    e.target.value = "";
  };

  const handleSaveApiKey = async () => {
    const trimmed = apiKeyInput.trim();
    if (!trimmed) return;
    const targetProvider: TranslationProviderKind =
      effectiveProvider === "customOpenAi" ? "customOpenAi" : "deepSeek";
    await runSetApiKey(targetProvider, trimmed);
    setApiKeyInput("");
  };

  const commitCloudEndpoint = () => {
    const trimmed = cloudEndpointDraft.trim();
    if (effectiveProvider === "deepSeek") {
      if (trimmed && trimmed !== config.translation.deepseekModel) {
        void updateConfig({
          translation: {
            ...config.translation,
            deepseekModel: trimmed,
          },
        });
      }
    } else if (effectiveProvider === "customOpenAi") {
      if (trimmed && trimmed !== config.translation.customBaseUrl) {
        void updateConfig({
          translation: {
            ...config.translation,
            customBaseUrl: trimmed,
          },
        });
      }
    }
  };

  const commitCustomModel = () => {
    const trimmed = customModelDraft.trim();
    if (trimmed && trimmed !== config.translation.customModel) {
      void updateConfig({
        translation: {
          ...config.translation,
          customModel: trimmed,
        },
      });
    }
  };

  const toggleExportFormat = (fmt: ExportFormat) => {
    const current = config.export.formats;
    const exists = current.includes(fmt);
    if (exists && current.length <= 1) return;
    const nextFormats = exists
      ? current.filter((f) => f !== fmt)
      : [...current, fmt];
    void updateConfig({
      export: {
        ...config.export,
        formats: nextFormats,
      },
    });
  };

  const handleBrowseExportDir = async () => {
    if (isTauriRuntime()) {
      try {
        const dir = await openDialogPreservingMaximize({
          directory: true,
          multiple: false,
        });
        if (typeof dir === "string" && dir) {
          setExportDirDraft(dir);
          await updateConfig({
            export: {
              ...config.export,
              saveBesideAudio: false,
              outputDir: dir,
            },
          });
        }
      } catch {
        // Ignore dialog cancel
      }
    }
  };

  const isOnline =
    effectiveProvider === "deepSeek" || effectiveProvider === "customOpenAi";
  const modePillText =
    effectiveProvider === "localLlama"
      ? "OFFLINE"
      : effectiveProvider === "deepSeek"
        ? "ONLINE • DeepSeek"
        : "ONLINE • Custom";

  const hasCurrentApiKey =
    effectiveProvider === "customOpenAi"
      ? apiKeyStatus.hasCustomApiKey
      : apiKeyStatus.hasDeepSeekKey;

  const gpuBannerLabel =
    setupStatus?.gpu.hasNvidia && setupStatus.gpu.gpuName
      ? `${setupStatus.gpu.gpuName} • ${setupStatus.gpu.vramMb} MB VRAM`
      : t.setup.noNvidiaGpu;

  const storageMode = (setupStatus?.storage.mode ?? "installed").toUpperCase();
  const storageDataDir = setupStatus?.storage.dataDir ?? "./data";

  const tabs: Array<{ id: SettingsTabId; label: string }> = [
    { id: "obrada", label: lang === "en" ? "Processing" : "Obrada" },
    { id: "titlovi", label: lang === "en" ? "Subtitles" : "Titlovi" },
    { id: "modeli", label: lang === "en" ? "Models" : "Modeli" },
    { id: "izvoz", label: lang === "en" ? "Export" : "Izvoz" },
    { id: "opste", label: lang === "en" ? "General" : "Opšte" },
  ];

  return (
    <section
      id="settings-panel"
      aria-label={lang === "en" ? "Settings" : "Podešavanja"}
    >
      <header className="settings-head">
        <div
          className="settings-tabs"
          role="tablist"
          aria-label={
            lang === "en" ? "Settings categories" : "Kategorije podešavanja"
          }
        >
          {tabs.map((tab) => {
            const active = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                className={`s-tab${active ? " active" : ""}`}
                type="button"
                role="tab"
                aria-selected={active}
                data-tab={tab.id}
                onClick={() => setActiveTab(tab.id)}
              >
                {tab.label}
              </button>
            );
          })}
        </div>

        <button
          id="settings-mode-pill"
          type="button"
          className={`settings-mode-pill${isOnline ? " online" : ""}`}
          style={{ cursor: "pointer" }}
          title={
            lang === "en"
              ? "Switch Offline / Online mode in Models"
              : "Promeni Offline / Online režim u Modelima"
          }
          onClick={() => setActiveTab("modeli")}
        >
          <span className="mode-dot" />
          <span id="settings-mode-text">{modePillText}</span>
        </button>
      </header>

      <div className="settings-body">
        {/* TAB 1: OBRADA */}
        {activeTab === "obrada" && (
          <div id="pane-obrada" className="s-pane">
            <div className="s-grid-2">
              <div className="s-field">
                <label className="s-label" htmlFor="s-source-lang">
                  {t.workspace.sourceLanguage}
                </label>
                <select
                  id="s-source-lang"
                  className="s-select"
                  value={jobOptions.sourceLanguage}
                  onChange={(e) =>
                    setJobOptions({ sourceLanguage: e.target.value })
                  }
                >
                  <option value="auto">{t.languages.auto}</option>
                  <option value="en">{t.languages.en}</option>
                  <option value="sr">{t.languages.sr}</option>
                  <option value="de">{t.languages.de}</option>
                  <option value="es">{t.languages.es}</option>
                  <option value="fr">{t.languages.fr}</option>
                  <option value="ru">{t.languages.ru}</option>
                  <option value="it">{t.languages.it}</option>
                </select>
              </div>

              <div className="s-field">
                <label className="s-label" htmlFor="s-target-lang">
                  {t.workspace.targetLanguage}
                </label>
                <select
                  id="s-target-lang"
                  className="s-select"
                  value={jobOptions.targetLanguage}
                  onChange={(e) =>
                    setJobOptions({ targetLanguage: e.target.value })
                  }
                >
                  <option value="sr">{t.languages.sr}</option>
                  <option value="en">{t.languages.en}</option>
                  <option value="none">{t.languages.none}</option>
                </select>
              </div>
            </div>

            <div className="s-grid-2">
              <div className="s-field">
                <span className="s-label">
                  {lang === "en" ? "Serbian Script" : "Srpsko pismo"}
                </span>
                <div
                  className="s-seg"
                  role="group"
                  aria-label={t.workspace.serbianScript}
                >
                  <button
                    type="button"
                    className={`s-seg-btn js-script-btn${
                      jobOptions.script === "latin" ? " active-accent" : ""
                    }`}
                    data-script="latin"
                    onClick={() => void convertActiveProjectScript("latin")}
                  >
                    {t.workspace.scriptLatin}
                  </button>
                  <button
                    type="button"
                    className={`s-seg-btn js-script-btn${
                      jobOptions.script === "cyrillic" ? " active-accent" : ""
                    }`}
                    data-script="cyrillic"
                    onClick={() => void convertActiveProjectScript("cyrillic")}
                  >
                    {t.workspace.scriptCyrillic}
                  </button>
                </div>
              </div>

              <div className="s-field">
                <span className="s-label">
                  {lang === "en" ? "Language Polishing" : "Jezičko poliranje"}
                </span>
                <label className="s-toggle-card">
                  <span className="s-toggle-meta">
                    <span className="s-toggle-title">
                      {lang === "en" ? "LLM Polish" : "LLM poliranje"}
                    </span>
                    <span className="s-toggle-hint">Ekavski standard</span>
                  </span>
                  <input
                    id="s-polish-sr"
                    type="checkbox"
                    checked={jobOptions.polishSerbian}
                    onChange={(e) =>
                      setJobOptions({ polishSerbian: e.target.checked })
                    }
                    hidden
                  />
                  <span className="s-switch" aria-hidden="true" />
                </label>
              </div>
            </div>

            <label className="s-toggle-card">
              <span className="s-toggle-meta">
                <span className="s-toggle-title">
                  {t.workspace.correctSourceAsr}
                </span>
                <span className="s-toggle-hint">
                  {t.workspace.correctSourceAsrHint}
                </span>
              </span>
              <input
                id="s-correct-asr"
                type="checkbox"
                checked={Boolean(jobOptions.correctSourceAsr)}
                onChange={(e) =>
                  setJobOptions({ correctSourceAsr: e.target.checked })
                }
                hidden
              />
              <span className="s-switch" aria-hidden="true" />
            </label>

            <div className="s-glossary-box">
              <div className="s-glossary-top">
                <span className="s-label">
                  {lang === "en" ? "Glossary" : "Pojmovnik (Glossary)"}
                </span>
                <div className="s-inline-add">
                  <input
                    id="s-glossary-input"
                    className="s-input"
                    type="text"
                    value={glossaryInput}
                    onChange={(e) => setGlossaryInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        handleAddGlossaryTerm();
                      }
                    }}
                    placeholder={
                      lang === "en"
                        ? "Add term or A -> B..."
                        : "Dodaj pojam (ili A -> B)..."
                    }
                  />
                  <button
                    id="s-glossary-add"
                    className="s-mini-btn info compact"
                    type="button"
                    title={t.workspace.glossaryAddBtn}
                    onClick={handleAddGlossaryTerm}
                  >
                    +
                  </button>
                </div>
              </div>
              <div id="s-glossary-chips" className="s-glossary-chips">
                {glossaryEntries.map(([term, target]) => (
                  <span key={term} className="s-chip">
                    <span>
                      {term === target ? term : `${term} → ${target}`}
                    </span>
                    <button
                      type="button"
                      title={`Ukloni "${term}"`}
                      onClick={() => handleRemoveGlossaryTerm(term)}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* TAB 2: TITLOVI */}
        {activeTab === "titlovi" && (
          <div id="pane-titlovi" className="s-pane">
            <div className="s-field">
              <span className="s-label">
                {lang === "en" ? "Segmentation Preset" : "Profil segmentacije"}
              </span>
              <div
                className="s-seg three"
                role="group"
                aria-label={t.workspace.segmentPreset}
              >
                {(
                  [
                    ["standard", t.workspace.presetStandard],
                    ["reels", t.workspace.presetReels],
                    [
                      "custom",
                      lang === "en" ? "Custom" : "Prilagođeno",
                    ],
                  ] as const
                ).map(([presetKey, label]) => (
                  <button
                    key={presetKey}
                    type="button"
                    className={`s-seg-btn js-preset-btn${
                      jobOptions.segmentRules.preset === presetKey
                        ? " active-info"
                        : ""
                    }`}
                    data-preset={presetKey}
                    onClick={() => handlePresetClick(presetKey)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>

            <div className="s-grid-4">
              <div className="s-field">
                <label className="s-label" htmlFor="s-max-chars">
                  {lang === "en" ? "Max chars" : "Maks. znakova"}
                </label>
                <NumericFieldInput
                  id="s-max-chars"
                  min={10}
                  max={80}
                  value={jobOptions.segmentRules.maxCharsPerLine}
                  onCommit={(maxCharsPerLine) =>
                    updateSegmentRules({ maxCharsPerLine })
                  }
                />
              </div>

              <div className="s-field">
                <label className="s-label" htmlFor="s-max-lines">
                  {lang === "en" ? "Max lines" : "Maks. redova"}
                </label>
                <NumericFieldInput
                  id="s-max-lines"
                  min={1}
                  max={4}
                  value={jobOptions.segmentRules.maxLines}
                  onCommit={(maxLines) => updateSegmentRules({ maxLines })}
                />
              </div>

              <div className="s-field">
                <label className="s-label" htmlFor="s-max-cps">
                  {lang === "en" ? "Max CPS" : "Maks. CPS"}
                </label>
                <NumericFieldInput
                  id="s-max-cps"
                  min={8}
                  max={35}
                  value={jobOptions.segmentRules.maxCps}
                  onCommit={(maxCps) => updateSegmentRules({ maxCps })}
                />
              </div>

              <div className="s-field">
                <label className="s-label" htmlFor="s-max-dur">
                  {lang === "en" ? "Max dur. (s)" : "Maks. trajanje (s)"}
                </label>
                <NumericFieldInput
                  id="s-max-dur"
                  min={2}
                  max={15}
                  step={0.5}
                  isFloat
                  value={Number(
                    (jobOptions.segmentRules.maxDurationMs / 1000).toFixed(1),
                  )}
                  onCommit={(secs) =>
                    updateSegmentRules({
                      maxDurationMs: Math.round(secs * 1000),
                    })
                  }
                />
              </div>
            </div>

            <div className="s-action-row">
              <span
                id="s-seg-status"
                className="s-note"
                style={segFlash ? { color: "#6ee7b7" } : undefined}
              >
                {segFlash
                  ? lang === "en"
                    ? "✓ Segmentation rules applied to active project"
                    : "✓ Pravila segmentacije primenjena na aktivne titlove"
                  : lang === "en"
                    ? "Rules apply without re-transcribing"
                    : "Pravila se primenjuju bez ponovne transkripcije"}
              </span>

              <div className="s-action-btns">
                {warningCount > 0 && (
                  <button
                    type="button"
                    data-testid="btn-condense-cps"
                    disabled={isRunningBatch}
                    className="s-mini-btn amber"
                    onClick={condenseActiveProjectCps}
                    title={t.table.condenseCpsTitle}
                  >
                    {t.table.condenseCpsBtn} ({warningCount})
                  </button>
                )}

                <button
                  id="s-apply-seg"
                  className="s-mini-btn info"
                  type="button"
                  disabled={!activeProject}
                  onClick={() => void handleApplySeg()}
                >
                  {lang === "en" ? "Apply to subtitles" : "Primeni na titlove"}
                </button>
              </div>
            </div>
          </div>
        )}

        {/* TAB 3: MODELI (Offline lokalni modeli / Online Cloud API) */}
        {activeTab === "modeli" && (
          <div id="pane-modeli" className="s-pane">
            <div className="s-field">
              <span className="s-label">
                {lang === "en" ? "Active Mode" : "Aktivni režim (Mode)"}
              </span>
              <div
                className="s-seg"
                role="group"
                aria-label={
                  lang === "en" ? "Active Mode" : "Aktivni režim (Mode)"
                }
              >
                <button
                  type="button"
                  data-testid="mode-offline-btn"
                  className={`s-seg-btn${!isOnline ? " active-accent" : ""}`}
                  onClick={() => void setEffectiveProvider("localLlama")}
                >
                  {lang === "en"
                    ? "Offline (Local Models)"
                    : "Offline (Lokalni modeli)"}
                </button>
                <button
                  type="button"
                  data-testid="mode-online-btn"
                  className={`s-seg-btn${isOnline ? " active-info" : ""}`}
                  onClick={() => void setEffectiveProvider(lastOnlineProvider)}
                >
                  {lang === "en"
                    ? "Online (Cloud API)"
                    : "Online (Cloud API)"}
                </button>
              </div>
            </div>

            {!isOnline ? (
              <>
                <div
                  className={`s-gpu-banner${
                    setupStatus?.readyForOffline ? "" : " warn"
                  }`}
                >
                  <div className="s-gpu-left" title={gpuBannerLabel}>
                    <span className="s-gpu-dot" aria-hidden="true" />
                    <span>{gpuBannerLabel}</span>
                  </div>

                  <div className="s-action-btns">
                    <span
                      className={
                        setupStatus?.readyForOffline
                          ? "s-pill-green"
                          : "s-pill-amber"
                      }
                    >
                      {setupStatus?.readyForOffline
                        ? lang === "en"
                          ? "Ready"
                          : "Spremno"
                        : lang === "en"
                          ? "Setup needed"
                          : "Potrebni modeli"}
                    </span>
                    <button
                      type="button"
                      className="s-mini-btn compact info"
                      onClick={() => setSetupOpen(true)}
                    >
                      {lang === "en" ? "Manager" : "Instalacija"}
                    </button>
                  </div>
                </div>

                <div className="s-grid-2">
                  <div className="s-field">
                    <label className="s-label" htmlFor="s-whisper-model">
                      Whisper ASR model
                    </label>
                    <select
                      id="s-whisper-model"
                      className="s-select"
                      value={config.activeWhisperModelId ?? ""}
                      onChange={(e) =>
                        void updateConfig({
                          activeWhisperModelId: e.target.value || null,
                        })
                      }
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

                  <div className="s-field">
                    <label className="s-label" htmlFor="s-llm-model">
                      {lang === "en" ? "Local LLM Model" : "Lokalni LLM model"}
                    </label>
                    <select
                      id="s-llm-model"
                      className="s-select"
                      value={config.activeLlmModelId ?? ""}
                      onChange={(e) =>
                        void updateConfig({
                          activeLlmModelId: e.target.value || null,
                        })
                      }
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

                <div className="s-field">
                  <label className="s-label" htmlFor="s-models-dir">
                    {lang === "en" ? "Models Folder" : "Folder za modele"}
                  </label>
                  <div className="s-input-group">
                    <input
                      id="s-models-dir"
                      className="s-input mono-inp"
                      type="text"
                      value={modelsDirDraft}
                      placeholder={setupStatus?.storage.modelsDir ?? "./models"}
                      onChange={(e) => setModelsDirDraft(e.target.value)}
                      onBlur={() =>
                        void updateConfig({
                          modelsDirOverride: modelsDirDraft.trim() || null,
                        })
                      }
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.currentTarget.blur();
                        }
                      }}
                    />
                    <button
                      id="s-btn-browse-models"
                      className="s-mini-btn"
                      type="button"
                      onClick={() => void handleBrowseModelsDir()}
                    >
                      {t.settings.browseFolder}
                    </button>
                    <button
                      id="s-btn-import-model"
                      className="s-mini-btn info"
                      type="button"
                      onClick={() => void handleImportModelClick()}
                    >
                      + {lang === "en" ? "Import" : "Uvezi"} .bin / .gguf
                    </button>
                    <input
                      ref={modelImportInputRef}
                      type="file"
                      accept=".bin,.gguf"
                      onChange={handleBrowserModelImport}
                      hidden
                    />
                  </div>
                </div>
              </>
            ) : (
              <>
                <div className="s-field">
                  <span className="s-label">
                    {lang === "en" ? "Cloud Provider" : "Cloud provajder"}
                  </span>
                  <div
                    className="s-seg"
                    role="group"
                    aria-label={t.workspace.translationProvider}
                  >
                    <button
                      type="button"
                      className={`s-seg-btn js-provider-btn${
                        effectiveProvider === "deepSeek" ? " active-info" : ""
                      }`}
                      data-provider="deepSeek"
                      onClick={() => {
                        setLastOnlineProvider("deepSeek");
                        void setEffectiveProvider("deepSeek");
                      }}
                    >
                      DeepSeek API
                    </button>
                    <button
                      type="button"
                      className={`s-seg-btn js-provider-btn${
                        effectiveProvider === "customOpenAi"
                          ? " active-info"
                          : ""
                      }`}
                      data-provider="customOpenAi"
                      onClick={() => {
                        setLastOnlineProvider("customOpenAi");
                        void setEffectiveProvider("customOpenAi");
                      }}
                    >
                      Custom OpenAI
                    </button>
                  </div>
                </div>

                <div className="s-grid-2">
                  <div className="s-field">
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "space-between",
                        gap: "6px",
                      }}
                    >
                      <label className="s-label" htmlFor="s-api-key">
                        {t.settings.apiKeyLabel}
                      </label>
                      <span
                        id="api-key-badge"
                        className={
                          hasCurrentApiKey ? "s-pill-green" : "s-pill-muted"
                        }
                        style={{ padding: "0 6px", fontSize: "9.5px" }}
                      >
                        {hasCurrentApiKey
                          ? lang === "en"
                            ? "Saved"
                            : "Sačuvan"
                          : lang === "en"
                            ? "Not set"
                            : "Nije podešen"}
                      </span>
                    </div>
                    <div className="s-input-group">
                      <input
                        id="s-api-key"
                        className="s-input mono-inp"
                        type="password"
                        value={apiKeyInput}
                        onChange={(e) => setApiKeyInput(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            e.preventDefault();
                            void handleSaveApiKey();
                          }
                        }}
                        placeholder="sk-..."
                      />
                      <button
                        id="s-btn-save-key"
                        className="s-mini-btn accent"
                        type="button"
                        disabled={!apiKeyInput.trim()}
                        onClick={() => void handleSaveApiKey()}
                      >
                        {lang === "en" ? "Save" : "Sačuvaj"}
                      </button>
                      {hasCurrentApiKey && (
                        <button
                          type="button"
                          className="s-mini-btn danger"
                          title={t.settings.clearKey}
                          onClick={() =>
                            void runClearApiKey(
                              effectiveProvider === "customOpenAi"
                                ? "customOpenAi"
                                : "deepSeek",
                            )
                          }
                        >
                          ×
                        </button>
                      )}
                    </div>
                  </div>

                  <div className="s-field">
                    <label className="s-label" htmlFor="s-cloud-endpoint">
                      {effectiveProvider === "customOpenAi"
                        ? "Endpoint / Model"
                        : "Model / Endpoint"}
                    </label>
                    <div className="s-input-group">
                      <input
                        id="s-cloud-endpoint"
                        className="s-input mono-inp"
                        type="text"
                        value={cloudEndpointDraft}
                        onChange={(e) => setCloudEndpointDraft(e.target.value)}
                        onBlur={commitCloudEndpoint}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            e.currentTarget.blur();
                          }
                        }}
                        placeholder={
                          effectiveProvider === "deepSeek"
                            ? "deepseek-chat"
                            : "https://api.openai.com/v1"
                        }
                      />
                      {effectiveProvider === "customOpenAi" && (
                        <input
                          aria-label={t.settings.customModelLabel}
                          className="s-input mono-inp"
                          type="text"
                          style={{ maxWidth: "110px" }}
                          value={customModelDraft}
                          onChange={(e) => setCustomModelDraft(e.target.value)}
                          onBlur={commitCustomModel}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.currentTarget.blur();
                            }
                          }}
                          placeholder="gpt-4o-mini"
                        />
                      )}
                    </div>
                  </div>
                </div>

                <div className="s-grid-2">
                  <div className="s-field">
                    <label className="s-label" htmlFor="s-batch-size">
                      {lang === "en"
                        ? "Cues per batch (batchSize)"
                        : "Titlova po paketu (batchSize)"}
                    </label>
                    <NumericFieldInput
                      id="s-batch-size"
                      min={5}
                      max={60}
                      value={config.translation.batchSize}
                      onCommit={(batchSize) =>
                        void updateConfig({
                          translation: {
                            ...config.translation,
                            batchSize,
                          },
                        })
                      }
                    />
                  </div>

                  <div className="s-field">
                    <label className="s-label" htmlFor="s-ctx-win">
                      {lang === "en"
                        ? "Context window (contextWindow)"
                        : "Kontekstni prozor (contextWindow)"}
                    </label>
                    <NumericFieldInput
                      id="s-ctx-win"
                      min={2048}
                      max={32768}
                      step={1024}
                      value={config.translation.contextWindow}
                      onCommit={(contextWindow) =>
                        void updateConfig({
                          translation: {
                            ...config.translation,
                            contextWindow,
                          },
                        })
                      }
                    />
                  </div>
                </div>
              </>
            )}
          </div>
        )}

        {/* TAB 5: IZVOZ */}
        {activeTab === "izvoz" && (
          <div id="pane-izvoz" className="s-pane">
            <div className="s-field">
              <span className="s-label">{t.export.formatsLabel}</span>
              <div className="s-grid-3">
                {(
                  [
                    ["srt", ".SRT (SubRip)"],
                    ["vtt", ".VTT (WebVTT)"],
                    ["txt", ".TXT (Transkript)"],
                  ] as const
                ).map(([fmt, label]) => {
                  const active = config.export.formats.includes(fmt);
                  return (
                    <button
                      key={fmt}
                      type="button"
                      className={`s-check-chip js-export-fmt${
                        active ? " active" : ""
                      }`}
                      data-fmt={fmt}
                      onClick={() => toggleExportFormat(fmt)}
                    >
                      <span>{label}</span>
                      <span className="s-check-box">✓</span>
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="s-grid-2">
              <label className="s-toggle-card tall">
                <span className="s-toggle-meta">
                  <span className="s-toggle-title">
                    {lang === "en"
                      ? "Include source .SRT"
                      : "Izvezi i izvorni .SRT"}
                  </span>
                </span>
                <input
                  id="s-exp-source"
                  type="checkbox"
                  checked={config.export.includeSourceSrt}
                  onChange={(e) =>
                    void updateConfig({
                      export: {
                        ...config.export,
                        includeSourceSrt: e.target.checked,
                      },
                    })
                  }
                  hidden
                />
                <span className="s-switch" aria-hidden="true" />
              </label>

              <label className="s-toggle-card tall">
                <span className="s-toggle-meta">
                  <span className="s-toggle-title">
                    {lang === "en" ? "UTF-8 with BOM" : "UTF-8 sa BOM-om"}
                  </span>
                </span>
                <input
                  id="s-exp-bom"
                  type="checkbox"
                  checked={config.export.utf8Bom}
                  onChange={(e) =>
                    void updateConfig({
                      export: {
                        ...config.export,
                        utf8Bom: e.target.checked,
                      },
                    })
                  }
                  hidden
                />
                <span className="s-switch" aria-hidden="true" />
              </label>
            </div>

            <div className="s-grid-2">
              <div className="s-field">
                <span className="s-label">{t.export.locationLabel}</span>
                <div
                  className="s-seg"
                  role="group"
                  aria-label={t.export.locationLabel}
                >
                  <button
                    type="button"
                    className={`s-seg-btn js-exp-loc-btn${
                      config.export.saveBesideAudio ? " active-info" : ""
                    }`}
                    data-loc="beside"
                    onClick={() =>
                      void updateConfig({
                        export: { ...config.export, saveBesideAudio: true },
                      })
                    }
                  >
                    {t.export.saveBesideAudio}
                  </button>
                  <button
                    type="button"
                    className={`s-seg-btn js-exp-loc-btn${
                      !config.export.saveBesideAudio ? " active-info" : ""
                    }`}
                    data-loc="custom"
                    onClick={() =>
                      void updateConfig({
                        export: { ...config.export, saveBesideAudio: false },
                      })
                    }
                  >
                    {t.export.customFolder}
                  </button>
                </div>
              </div>

              <div className="s-field">
                <label className="s-label" htmlFor="s-exp-folder">
                  {lang === "en" ? "Output Folder" : "Izlazni folder"}
                </label>
                <div className="s-input-group">
                  <input
                    id="s-exp-folder"
                    className="s-input mono-inp"
                    type="text"
                    disabled={config.export.saveBesideAudio}
                    value={exportDirDraft}
                    placeholder="./exports"
                    onChange={(e) => setExportDirDraft(e.target.value)}
                    onBlur={() =>
                      void updateConfig({
                        export: {
                          ...config.export,
                          outputDir: exportDirDraft.trim() || null,
                        },
                      })
                    }
                  />
                  {!config.export.saveBesideAudio && (
                    <button
                      type="button"
                      className="s-mini-btn"
                      onClick={() => void handleBrowseExportDir()}
                    >
                      ...
                    </button>
                  )}
                </div>
              </div>
            </div>

            <div className="s-action-row">
              <span className="s-note">
                {activeProject
                  ? `${activeProject.audioFileName} • ${activeProject.cues.length} ${t.queue.cuesCount}`
                  : t.table.noProject}
              </span>
              <button
                id="s-btn-export-now"
                type="button"
                className="s-mini-btn accent"
                disabled={!activeProject || activeProject.cues.length === 0}
                onClick={() => void runExportSubtitles(config.export)}
              >
                {lang === "en" ? "Export Now" : "Izvezi sada"}
              </button>
            </div>
          </div>
        )}

        {/* TAB 6: OPŠTE */}
        {activeTab === "opste" && (
          <div id="pane-opste" className="s-pane">
            <div className="s-grid-2">
              <div className="s-field">
                <span className="s-label">{t.topBar.languageSwitch}</span>
                <div
                  className="s-seg"
                  role="group"
                  aria-label={t.topBar.languageSwitch}
                >
                  <button
                    type="button"
                    className={`s-seg-btn js-ui-lang-btn${
                      lang === "sr" ? " active-info" : ""
                    }`}
                    data-lang="sr"
                    onClick={() => void setUiLanguage("sr")}
                  >
                    Srpski
                  </button>
                  <button
                    type="button"
                    className={`s-seg-btn js-ui-lang-btn${
                      lang === "en" ? " active-info" : ""
                    }`}
                    data-lang="en"
                    onClick={() => void setUiLanguage("en")}
                  >
                    English
                  </button>
                </div>
              </div>

              <div className="s-field">
                <span className="s-label">
                  {lang === "en"
                    ? "Default Script"
                    : "Podrazumevano pismo"}
                </span>
                <div
                  className="s-seg"
                  role="group"
                  aria-label={t.settings.defaultScriptLabel}
                >
                  <button
                    type="button"
                    className={`s-seg-btn js-def-script-btn${
                      config.defaultScript === "latin" ? " active-accent" : ""
                    }`}
                    data-def-script="latin"
                    onClick={() =>
                      void updateConfig({
                        defaultScript: "latin" as SerbianScript,
                      })
                    }
                  >
                    {t.workspace.scriptLatin}
                  </button>
                  <button
                    type="button"
                    className={`s-seg-btn js-def-script-btn${
                      config.defaultScript === "cyrillic"
                        ? " active-accent"
                        : ""
                    }`}
                    data-def-script="cyrillic"
                    onClick={() =>
                      void updateConfig({
                        defaultScript: "cyrillic" as SerbianScript,
                      })
                    }
                  >
                    {t.workspace.scriptCyrillic}
                  </button>
                </div>
              </div>
            </div>

            <div className="s-grid-2">
              <div className="s-field">
                <span className="s-label">{t.setup.storageMode}</span>
                <div className="s-readonly-box" title={storageDataDir}>
                  <span className="s-readonly-path">
                    {storageMode} • {storageDataDir}
                  </span>
                  <span
                    className="s-pill-green"
                    style={{ padding: "0 6px", fontSize: "9.5px" }}
                  >
                    {lang === "en" ? "Active" : "Aktivan"}
                  </span>
                </div>
              </div>

              <div className="s-field">
                <span className="s-label">
                  {lang === "en" ? "Session Recovery" : "Oporavak sesije"}
                </span>
                <label className="s-toggle-card">
                  <span className="s-toggle-meta">
                    <span className="s-toggle-title">
                      {lang === "en"
                        ? "Auto checkpoints"
                        : "Automatski checkpoint-i"}
                    </span>
                  </span>
                  <input
                    id="s-auto-cp"
                    type="checkbox"
                    checked={autoCheckpoints}
                    onChange={(e) => setAutoCheckpoints(e.target.checked)}
                    hidden
                  />
                  <span className="s-switch" aria-hidden="true" />
                </label>
              </div>
            </div>

            {checkpoints.length > 0 && (
              <div className="s-action-row">
                <select
                  aria-label={t.workspace.checkpointsTitle}
                  className="s-select"
                  style={{ height: "26px", flex: 1, minWidth: 0 }}
                  value={selectedCheckpointId}
                  onChange={(e) => setSelectedCheckpointId(e.target.value)}
                >
                  {checkpoints.map((cp) => (
                    <option key={cp.jobId} value={cp.jobId}>
                      {cp.audioFileName} ({cp.cues.length} {t.queue.cuesCount} •{" "}
                      {formatDurationShort(cp.durationMs)})
                    </option>
                  ))}
                </select>
                <div className="s-action-btns">
                  <button
                    type="button"
                    className="s-mini-btn compact accent"
                    disabled={!selectedCheckpointId}
                    onClick={() =>
                      selectedCheckpointId &&
                      void loadCheckpointIntoEditor(selectedCheckpointId)
                    }
                  >
                    {t.workspace.loadCheckpoint}
                  </button>
                  <button
                    type="button"
                    className="s-mini-btn compact danger"
                    disabled={!selectedCheckpointId}
                    title={t.workspace.deleteCheckpoint}
                    onClick={() =>
                      selectedCheckpointId &&
                      void removeCheckpoint(selectedCheckpointId)
                    }
                  >
                    ×
                  </button>
                </div>
              </div>
            )}

            <label className="s-toggle-card">
              <span className="s-toggle-meta">
                <span className="s-toggle-title">
                  {t.settings.autoSendCrashReports}
                </span>
                <span className="s-toggle-hint">
                  {t.settings.autoSendCrashReportsHint}
                </span>
              </span>
              <input
                id="s-auto-crash-reports"
                type="checkbox"
                checked={config.autoSendCrashReports !== false}
                onChange={(e) =>
                  void updateConfig({
                    autoSendCrashReports: e.target.checked,
                  })
                }
                hidden
              />
              <span className="s-switch" aria-hidden="true" />
            </label>

            <div className="s-action-row">
              <span id="s-bug-report-status" className="s-note">
                {t.settings.bugReportRowLabel}
              </span>
              <button
                id="s-btn-report-bug"
                className="s-mini-btn info"
                type="button"
                onClick={() => setBugReportOpen(true)}
              >
                {t.settings.bugReportRowBtn}
              </button>
            </div>

            <div className="s-action-row">
              <span id="s-update-status" className="s-note">
                {availableUpdateVersion
                  ? lang === "en"
                    ? `Updates • EasySRT Studio v${appVersion} → v${availableUpdateVersion} available`
                    : `Ažuriranja • EasySRT Studio v${appVersion} → dostupna v${availableUpdateVersion}`
                  : lang === "en"
                    ? `Updates • EasySRT Studio v${appVersion}`
                    : `Ažuriranja • EasySRT Studio v${appVersion}`}
              </span>
              <button
                id="s-btn-check-updates"
                className="s-mini-btn info"
                type="button"
                onClick={() => setUpdaterOpen(true)}
              >
                {t.updater.checkBtn}
              </button>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
