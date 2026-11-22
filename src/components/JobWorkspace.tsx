import { open } from "@tauri-apps/plugin-dialog";
import {
  BookOpen,
  ChevronDown,
  ChevronUp,
  FileText,
  FolderOpen,
  Languages,
  Loader2,
  Plus,
  RefreshCw,
  Sliders,
  Sparkles,
  Trash2,
  UploadCloud,
  Wand2,
  Zap,
} from "lucide-react";
import { useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime, parseBulkGlossary } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type {
  SegmentPreset,
  SerbianScript,
  TranslationProviderKind,
} from "../types/models";
import { BatchQueuePanel } from "./BatchQueuePanel";

const AUDIO_EXTENSIONS = ["mp3", "wav", "m4a", "flac", "ogg", "aac", "opus"];
const SUBTITLE_EXTENSIONS = ["srt", "vtt"];

function isSubtitleFileName(nameOrPath: string): boolean {
  return /\.(srt|vtt)$/i.test(nameOrPath.trim());
}

function readBrowserFileText(file: File): Promise<string> {
  if (typeof file.text === "function") {
    return file.text();
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () =>
      reject(reader.error ?? new Error("Failed to read file"));
    reader.readAsText(file, "utf-8");
  });
}

export function JobWorkspace() {
  const config = useAppStore((s) => s.config);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const activeProject = useAppStore((s) => s.activeProject);
  const batchQueue = useAppStore((s) => s.batchQueue);
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
  const enqueueAudioFiles = useAppStore((s) => s.enqueueAudioFiles);
  const importSubtitleFromPath = useAppStore((s) => s.importSubtitleFromPath);
  const importSubtitleFromText = useAppStore((s) => s.importSubtitleFromText);
  const startBatchPipeline = useAppStore((s) => s.startBatchPipeline);
  const translateActiveProject = useAppStore((s) => s.translateActiveProject);
  const polishActiveProject = useAppStore((s) => s.polishActiveProject);
  const loadCheckpointIntoEditor = useAppStore(
    (s) => s.loadCheckpointIntoEditor,
  );

  const [isDragging, setIsDragging] = useState(false);
  const [glossaryOpen, setGlossaryOpen] = useState(false);
  const [newTerm, setNewTerm] = useState("");
  const [newTranslation, setNewTranslation] = useState("");
  const [bulkGlossaryText, setBulkGlossaryText] = useState("");
  const fallbackInputRef = useRef<HTMLInputElement | null>(null);
  const fallbackSubtitleInputRef = useRef<HTMLInputElement | null>(null);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const effectiveProvider: TranslationProviderKind =
    jobOptions.providerOverride ?? config.translation.provider;

  const isSerbianInvolved =
    jobOptions.sourceLanguage === "sr" ||
    jobOptions.targetLanguage === "sr" ||
    jobOptions.sourceLanguage === "auto" ||
    activeProject?.detectedLanguage === "sr" ||
    activeProject?.options.targetLanguage === "sr";

  const handleBrowseFiles = async () => {
    if (isTauriRuntime()) {
      try {
        const selected = await open({
          multiple: true,
          filters: [
            {
              name: "Audio",
              extensions: AUDIO_EXTENSIONS,
            },
          ],
        });
        if (Array.isArray(selected) && selected.length > 0) {
          enqueueAudioFiles(selected);
        } else if (typeof selected === "string" && selected) {
          enqueueAudioFiles([selected]);
        }
        return;
      } catch {
        // Fallback to HTML input if dialog fails
      }
    }
    fallbackInputRef.current?.click();
  };

  const handleBrowseSubtitleFile = async () => {
    if (isTauriRuntime()) {
      try {
        const selected = await open({
          multiple: false,
          filters: [
            {
              name: "Subtitles (.srt, .vtt)",
              extensions: SUBTITLE_EXTENSIONS,
            },
          ],
        });
        const path = Array.isArray(selected) ? selected[0] : selected;
        if (typeof path === "string" && path) {
          await importSubtitleFromPath(path);
        }
        return;
      } catch {
        // Fallback to HTML input if dialog fails
      }
    }
    fallbackSubtitleInputRef.current?.click();
  };

  const handleFallbackFileInput = (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const files = e.target.files;
    if (!files || files.length === 0) {
      return;
    }
    const audioPaths: string[] = [];
    for (const file of Array.from(files)) {
      if (isSubtitleFileName(file.name)) {
        void readBrowserFileText(file).then((text) => {
          importSubtitleFromText(file.name, text);
        });
      } else {
        audioPaths.push(file.name);
      }
    }
    if (audioPaths.length > 0) {
      enqueueAudioFiles(audioPaths);
    }
    e.target.value = "";
  };

  const handleFallbackSubtitleInput = (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const file = e.target.files?.[0];
    if (!file) {
      return;
    }
    void readBrowserFileText(file).then((text) => {
      importSubtitleFromText(file.name, text);
    });
    e.target.value = "";
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length === 0) {
      return;
    }

    const audioPaths: string[] = [];
    for (const file of files) {
      const nativePath = (file as File & { path?: string }).path;
      const targetName = nativePath || file.name;
      if (!targetName) {
        continue;
      }

      if (isSubtitleFileName(targetName)) {
        if (isTauriRuntime() && nativePath) {
          void importSubtitleFromPath(nativePath);
        } else {
          void readBrowserFileText(file).then((text) => {
            importSubtitleFromText(file.name, text);
          });
        }
      } else {
        audioPaths.push(targetName);
      }
    }

    if (audioPaths.length > 0) {
      enqueueAudioFiles(audioPaths);
    }
  };

  const handleScriptClick = (script: SerbianScript) => {
    void convertActiveProjectScript(script);
  };

  const handlePresetChange = (preset: SegmentPreset) => {
    setSegmentPreset(preset);
    if (activeProject && preset !== "custom") {
      setTimeout(() => {
        void applyResegmentToActiveProject();
      }, 20);
    }
  };

  const handleAddGlossaryPair = () => {
    const key = newTerm.trim();
    if (!key) {
      return;
    }
    const val = newTranslation.trim() || key;
    setJobOptions({
      glossary: {
        ...jobOptions.glossary,
        [key]: val,
      },
    });
    setNewTerm("");
    setNewTranslation("");
  };

  const handleApplyBulkGlossary = () => {
    const parsed = parseBulkGlossary(bulkGlossaryText);
    if (Object.keys(parsed).length === 0) {
      return;
    }
    setJobOptions({
      glossary: {
        ...jobOptions.glossary,
        ...parsed,
      },
    });
    setBulkGlossaryText("");
  };

  const handleRemoveGlossaryKey = (key: string) => {
    const next = { ...jobOptions.glossary };
    delete next[key];
    setJobOptions({ glossary: next });
  };

  const glossaryEntries = Object.entries(jobOptions.glossary);

  return (
    <section className="space-y-4">
      {/* Hidden browser fallback file inputs */}
      <input
        ref={fallbackInputRef}
        type="file"
        multiple
        accept=".mp3,.wav,.m4a,.flac,.ogg,.aac,.opus,.srt,.vtt,audio/*"
        onChange={handleFallbackFileInput}
        className="hidden"
      />
      <input
        ref={fallbackSubtitleInputRef}
        type="file"
        accept=".srt,.vtt"
        onChange={handleFallbackSubtitleInput}
        className="hidden"
      />

      {/* Top Row: Drag & Drop Audio Zone + Batch Queue Panel */}
      <div className="grid gap-4 lg:grid-cols-12">
        {/* Drag-and-Drop Zone */}
        <div
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
          onClick={() => void handleBrowseFiles()}
          className={`flex cursor-pointer flex-col items-center justify-center gap-3 rounded-2xl border-2 border-dashed p-5 text-center transition lg:col-span-5 ${
            isDragging
              ? "border-emerald-400 bg-emerald-500/10"
              : "border-slate-800 bg-slate-900/50 hover:border-cyan-500/50 hover:bg-slate-900/80"
          }`}
        >
          <div className="flex h-11 w-11 items-center justify-center rounded-2xl border border-cyan-500/30 bg-cyan-500/10 text-cyan-300">
            <UploadCloud className="h-6 w-6" />
          </div>
          <div className="space-y-1">
            <h3 className="text-sm font-bold text-slate-100">
              {t.workspace.dropTitle}
            </h3>
            <p className="text-xs text-slate-400">{t.workspace.dropSubtitle}</p>
          </div>
          <div className="flex flex-wrap items-center justify-center gap-2 pt-1">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void handleBrowseFiles();
              }}
              className="inline-flex items-center gap-1.5 rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-3.5 py-1.5 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-500/25"
            >
              <FolderOpen className="h-3.5 w-3.5" />
              <span>{t.workspace.selectFilesBtn}</span>
            </button>

            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void handleBrowseSubtitleFile();
              }}
              className="inline-flex items-center gap-1.5 rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3.5 py-1.5 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25"
            >
              <FileText className="h-3.5 w-3.5" />
              <span>{t.workspace.importSubtitleBtn}</span>
            </button>

            {!isTauriRuntime() && !activeProject && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  void loadCheckpointIntoEditor("demo-session-01");
                }}
                className="inline-flex items-center gap-1.5 rounded-xl border border-slate-700 bg-slate-800 px-3 py-1.5 text-xs font-medium text-slate-300 transition hover:bg-slate-700"
              >
                <Sparkles className="h-3.5 w-3.5 text-emerald-400" />
                <span>{t.workspace.addDemoSampleBtn}</span>
              </button>
            )}
          </div>
        </div>

        {/* Batch Queue & Checkpoints Panel */}
        <div className="lg:col-span-7">
          <BatchQueuePanel />
        </div>
      </div>

      {/* Job Options Bar */}
      <div className="rounded-2xl border border-slate-800/90 bg-slate-900/70 p-4 shadow-md">
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {/* 1. Source & Target Languages */}
          <div className="space-y-2.5">
            <div>
              <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                {t.workspace.sourceLanguage}
              </label>
              <select
                value={jobOptions.sourceLanguage}
                onChange={(e) =>
                  setJobOptions({ sourceLanguage: e.target.value })
                }
                className="w-full rounded-xl border border-slate-800 bg-slate-950 px-3 py-2 text-xs font-medium text-slate-100 focus:border-cyan-500 focus:outline-none"
              >
                <option value="auto">{t.languages.auto}</option>
                <option value="sr">{t.languages.sr}</option>
                <option value="en">{t.languages.en}</option>
                <option value="de">{t.languages.de}</option>
                <option value="es">{t.languages.es}</option>
                <option value="fr">{t.languages.fr}</option>
                <option value="ru">{t.languages.ru}</option>
                <option value="it">{t.languages.it}</option>
              </select>
            </div>

            <div>
              <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                {t.workspace.targetLanguage}
              </label>
              <select
                value={jobOptions.targetLanguage}
                onChange={(e) =>
                  setJobOptions({ targetLanguage: e.target.value })
                }
                className="w-full rounded-xl border border-slate-800 bg-slate-950 px-3 py-2 text-xs font-medium text-slate-100 focus:border-cyan-500 focus:outline-none"
              >
                <option value="none">{t.languages.none}</option>
                <option value="sr">{t.languages.sr}</option>
                <option value="en">{t.languages.en}</option>
              </select>
            </div>
          </div>

          {/* 2. Serbian Script & Serbian LLM Polishing */}
          <div className="space-y-2.5">
            <div>
              <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                {t.workspace.serbianScript}
              </label>
              <div className="grid grid-cols-2 gap-1.5 rounded-xl border border-slate-800 bg-slate-950 p-1">
                <button
                  type="button"
                  onClick={() => handleScriptClick("latin")}
                  className={`rounded-lg py-1.5 text-xs font-semibold transition ${
                    jobOptions.script === "latin"
                      ? "bg-emerald-500/20 text-emerald-300 shadow-sm"
                      : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {t.workspace.scriptLatin}
                </button>
                <button
                  type="button"
                  onClick={() => handleScriptClick("cyrillic")}
                  className={`rounded-lg py-1.5 text-xs font-semibold transition ${
                    jobOptions.script === "cyrillic"
                      ? "bg-emerald-500/20 text-emerald-300 shadow-sm"
                      : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {t.workspace.scriptCyrillic}
                </button>
              </div>
            </div>

            {/* LLM Polish Serbian Checkbox */}
            <label
              className={`flex cursor-pointer items-start gap-2.5 rounded-xl border p-2.5 transition ${
                !isSerbianInvolved
                  ? "cursor-not-allowed border-slate-800/50 bg-slate-950/30 opacity-50"
                  : jobOptions.polishSerbian
                    ? "border-emerald-500/30 bg-emerald-950/20"
                    : "border-slate-800 bg-slate-950/60"
              }`}
            >
              <input
                type="checkbox"
                disabled={!isSerbianInvolved}
                checked={jobOptions.polishSerbian}
                onChange={(e) =>
                  setJobOptions({ polishSerbian: e.target.checked })
                }
                className="mt-0.5 h-4 w-4 rounded border-slate-700 bg-slate-900 text-emerald-500 focus:ring-emerald-500"
              />
              <div className="space-y-0.5">
                <div className="text-xs font-semibold text-slate-200">
                  {t.workspace.polishSerbian}
                </div>
                <div className="text-[10px] leading-snug text-slate-400">
                  {t.workspace.polishSerbianHint}
                </div>
              </div>
            </label>
          </div>

          {/* 3. Segmentation Preset & Custom Rules */}
          <div className="space-y-2.5">
            <div>
              <label className="mb-1 flex items-center justify-between text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                <span>{t.workspace.segmentPreset}</span>
                <span className="font-mono text-[10px] text-cyan-400">
                  {jobOptions.segmentRules.maxLines}×
                  {jobOptions.segmentRules.maxCharsPerLine} •{" "}
                  {jobOptions.segmentRules.maxCps} CPS
                </span>
              </label>
              <div className="grid grid-cols-3 gap-1 rounded-xl border border-slate-800 bg-slate-950 p-1">
                {(
                  [
                    ["standard", t.workspace.presetStandard],
                    ["reels", t.workspace.presetReels],
                    ["custom", t.workspace.presetCustom],
                  ] as const
                ).map(([presetKey, label]) => (
                  <button
                    key={presetKey}
                    type="button"
                    onClick={() => handlePresetChange(presetKey)}
                    className={`truncate rounded-lg px-2 py-1.5 text-[11px] font-semibold transition ${
                      jobOptions.segmentRules.preset === presetKey
                        ? "bg-cyan-500/20 text-cyan-300"
                        : "text-slate-400 hover:text-slate-200"
                    }`}
                    title={label}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>

            {/* Sliders / numeric controls for Custom or quick tuning */}
            <div className="grid grid-cols-3 gap-2 rounded-xl border border-slate-800/80 bg-slate-950/70 p-2">
              <div>
                <label className="block truncate text-[10px] text-slate-400">
                  {t.workspace.maxCharsPerLine}
                </label>
                <input
                  type="number"
                  min={10}
                  max={80}
                  value={jobOptions.segmentRules.maxCharsPerLine}
                  onChange={(e) =>
                    updateSegmentRules({
                      maxCharsPerLine: Math.max(
                        10,
                        Number.parseInt(e.target.value, 10) || 42,
                      ),
                    })
                  }
                  className="mt-0.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-xs text-slate-100"
                />
              </div>
              <div>
                <label className="block truncate text-[10px] text-slate-400">
                  {t.workspace.maxLines}
                </label>
                <input
                  type="number"
                  min={1}
                  max={4}
                  value={jobOptions.segmentRules.maxLines}
                  onChange={(e) =>
                    updateSegmentRules({
                      maxLines: Math.max(
                        1,
                        Math.min(4, Number.parseInt(e.target.value, 10) || 2),
                      ),
                    })
                  }
                  className="mt-0.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-xs text-slate-100"
                />
              </div>
              <div>
                <label className="block truncate text-[10px] text-slate-400">
                  {t.workspace.maxCps}
                </label>
                <input
                  type="number"
                  min={8}
                  max={35}
                  value={jobOptions.segmentRules.maxCps}
                  onChange={(e) =>
                    updateSegmentRules({
                      maxCps: Math.max(
                        8,
                        Number.parseInt(e.target.value, 10) || 17,
                      ),
                    })
                  }
                  className="mt-0.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-xs text-slate-100"
                />
              </div>
            </div>

            {activeProject && (
              <button
                type="button"
                onClick={() => void applyResegmentToActiveProject()}
                className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-cyan-500/30 bg-cyan-500/10 py-1 text-[11px] font-medium text-cyan-300 transition hover:bg-cyan-500/20"
              >
                <Sliders className="h-3 w-3" />
                <span>{t.workspace.resegmentNow}</span>
              </button>
            )}
          </div>

          {/* 4. Provider Selector & Collapsible Glossary Toggle */}
          <div className="space-y-2.5">
            <div>
              <label className="mb-1 block text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                {t.workspace.translationProvider}
              </label>
              <select
                value={effectiveProvider}
                onChange={(e) =>
                  void setEffectiveProvider(
                    e.target.value as TranslationProviderKind,
                  )
                }
                className="w-full rounded-xl border border-slate-800 bg-slate-950 px-3 py-2 text-xs font-medium text-slate-100 focus:border-cyan-500 focus:outline-none"
              >
                <option value="localLlama">{t.workspace.providerLocal}</option>
                <option value="deepSeek">{t.workspace.providerDeepSeek}</option>
                <option value="customOpenAi">
                  {t.workspace.providerCustom}
                </option>
              </select>
            </div>

            <button
              type="button"
              onClick={() => setGlossaryOpen(!glossaryOpen)}
              className={`flex w-full items-center justify-between rounded-xl border px-3 py-2.5 text-xs font-semibold transition ${
                glossaryOpen || glossaryEntries.length > 0
                  ? "border-cyan-500/40 bg-cyan-950/25 text-cyan-200"
                  : "border-slate-800 bg-slate-950/70 text-slate-300 hover:border-slate-700"
              }`}
            >
              <span className="flex items-center gap-2">
                <BookOpen className="h-3.5 w-3.5 text-cyan-400" />
                <span>{t.workspace.glossaryTitle}</span>
                {glossaryEntries.length > 0 && (
                  <span className="rounded-full bg-cyan-500/20 px-1.5 py-0.2 font-mono text-[10px] text-cyan-300">
                    {glossaryEntries.length}
                  </span>
                )}
              </span>
              {glossaryOpen ? (
                <ChevronUp className="h-4 w-4" />
              ) : (
                <ChevronDown className="h-4 w-4" />
              )}
            </button>
          </div>
        </div>

        {/* Collapsible Glossary Panel */}
        {glossaryOpen && (
          <div className="mt-4 space-y-3 rounded-xl border border-slate-800 bg-slate-950/80 p-3.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-xs text-slate-400">
                {t.workspace.glossarySubtitle}
              </p>
              {glossaryEntries.length > 0 && (
                <button
                  type="button"
                  onClick={() => setJobOptions({ glossary: {} })}
                  className="text-xs text-rose-400 hover:text-rose-300"
                >
                  {t.workspace.glossaryClear}
                </button>
              )}
            </div>

            <div className="grid gap-3 lg:grid-cols-2">
              {/* Pair Adder & Chips */}
              <div className="space-y-2.5">
                <div className="flex flex-wrap gap-2">
                  <input
                    type="text"
                    value={newTerm}
                    onChange={(e) => setNewTerm(e.target.value)}
                    placeholder={t.workspace.glossaryTermPlaceholder}
                    className="flex-1 rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1.5 text-xs text-slate-100 focus:border-cyan-500 focus:outline-none"
                  />
                  <input
                    type="text"
                    value={newTranslation}
                    onChange={(e) => setNewTranslation(e.target.value)}
                    placeholder={t.workspace.glossaryTranslationPlaceholder}
                    className="flex-1 rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1.5 text-xs text-slate-100 focus:border-cyan-500 focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={handleAddGlossaryPair}
                    className="inline-flex items-center gap-1 rounded-lg border border-emerald-500/40 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/25"
                  >
                    <Plus className="h-3.5 w-3.5" />
                    <span>{t.workspace.glossaryAddBtn}</span>
                  </button>
                </div>

                {glossaryEntries.length > 0 && (
                  <div className="flex max-h-28 flex-wrap gap-1.5 overflow-y-auto pt-1">
                    {glossaryEntries.map(([term, target]) => (
                      <span
                        key={term}
                        className="inline-flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-900 px-2.5 py-1 font-mono text-xs text-slate-200"
                      >
                        <span className="text-cyan-300">{term}</span>
                        <span className="text-slate-500">→</span>
                        <span className="text-emerald-300">{target}</span>
                        <button
                          type="button"
                          onClick={() => handleRemoveGlossaryKey(term)}
                          className="ml-1 text-slate-500 hover:text-rose-400"
                        >
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </span>
                    ))}
                  </div>
                )}
              </div>

              {/* Bulk Paste Box */}
              <div className="space-y-1.5">
                <label className="block text-[11px] font-medium text-slate-400">
                  {t.workspace.glossaryBulkLabel}
                </label>
                <div className="flex gap-2">
                  <textarea
                    rows={2}
                    value={bulkGlossaryText}
                    onChange={(e) => setBulkGlossaryText(e.target.value)}
                    placeholder={t.workspace.glossaryBulkPlaceholder}
                    className="flex-1 rounded-lg border border-slate-800 bg-slate-900 p-2 font-mono text-xs text-slate-200 focus:border-cyan-500 focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={handleApplyBulkGlossary}
                    className="self-end rounded-lg border border-cyan-500/40 bg-cyan-500/15 px-3 py-2 text-xs font-semibold text-cyan-200 hover:bg-cyan-500/25"
                  >
                    {t.workspace.glossaryApplyBulk}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Primary Action Buttons Row */}
        <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-slate-800/80 pt-3.5">
          <div className="flex flex-wrap items-center gap-2.5">
            {/* Button 1: 1-Click Transcribe + Translate */}
            <button
              type="button"
              disabled={
                isRunningBatch ||
                (batchQueue.length === 0 && !activeProject)
              }
              onClick={() => startBatchPipeline("full")}
              className="inline-flex items-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2.5 text-xs font-bold text-slate-950 shadow-lg shadow-emerald-500/10 transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-50"
            >
              {isRunningBatch ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Zap className="h-4 w-4" />
              )}
              <span>{t.workspace.oneClickPipeline}</span>
            </button>

            {/* Button 2: Transcribe Only (2-step review) */}
            <button
              type="button"
              disabled={
                isRunningBatch ||
                (batchQueue.length === 0 && !activeProject)
              }
              onClick={() => startBatchPipeline("transcribeOnly")}
              className="inline-flex items-center gap-2 rounded-xl border border-slate-700 bg-slate-800/90 px-4 py-2.5 text-xs font-semibold text-slate-100 transition hover:border-slate-600 hover:bg-slate-800 disabled:opacity-50"
            >
              <RefreshCw className="h-3.5 w-3.5 text-cyan-400" />
              <span>{t.workspace.transcribeOnly}</span>
            </button>
          </div>

          {/* 2-Step Follow-up Actions on Active Project */}
          {activeProject && activeProject.cues.length > 0 && (
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                disabled={isRunningBatch}
                onClick={translateActiveProject}
                className="inline-flex items-center gap-1.5 rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-3.5 py-2 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-500/25 disabled:opacity-50"
              >
                <Languages className="h-3.5 w-3.5" />
                <span>{t.workspace.translateExisting}</span>
              </button>

              {isSerbianInvolved && (
                <button
                  type="button"
                  disabled={isRunningBatch}
                  onClick={polishActiveProject}
                  className="inline-flex items-center gap-1.5 rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3.5 py-2 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:opacity-50"
                >
                  <Wand2 className="h-3.5 w-3.5" />
                  <span>{t.workspace.polishExisting}</span>
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
