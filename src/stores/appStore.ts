import { create } from "zustand";
import { normalizeUiLanguage, type UiLanguage } from "../i18n";
import {
  calculateCueMetrics,
  convertSerbianScriptText,
  getPresetSegmentRules,
  parseSubtitleFileText,
  recalculateCue,
} from "../lib/subtitleUtils";
import {
  cancelJob,
  clearApiKey,
  convertCuesScript,
  deleteCheckpoint,
  deleteModel,
  exportSubtitles,
  getApiKeyStatus,
  getCatalog,
  getConfig,
  getSetupStatus,
  importCustomModel,
  importSubtitleFile,
  installComponent,
  installRecommendedBundle,
  listCheckpoints,
  loadCheckpoint,
  onDownloadProgress,
  onJobProgress,
  polishProject,
  resegmentProject,
  runFullPipeline,
  saveConfig,
  setApiKey,
  transcribeAudio,
  translateProject,
} from "../lib/tauri";
import type {
  ApiKeyStatus,
  AppConfig,
  CatalogItemKind,
  CatalogManifest,
  DownloadProgressEvent,
  ExportOptions,
  ExportResult,
  JobOptions,
  JobPhase,
  SegmentPreset,
  SegmentRules,
  SerbianScript,
  SetupStatus,
  SubtitleCue,
  SubtitleProject,
  TranslationProviderKind,
} from "../types/models";

export interface BatchQueueItem {
  jobId: string;
  audioPath: string;
  audioFileName: string;
  phase: JobPhase;
  percent: number;
  detail: string;
  project: SubtitleProject | null;
  errorMessage: string | null;
}

export interface OnlineConfirmState {
  open: boolean;
  provider: TranslationProviderKind;
  actionLabel: string;
  onConfirm: (() => void) | null;
}

export interface PlaybackRequest {
  cueId: number;
  startMs: number;
  endMs: number;
  nonce: number;
}

export interface BannerNotification {
  type: "info" | "success" | "error";
  message: string;
}

export type ReplaceTargetColumn = "both" | "source" | "translated";

export interface ReplaceInCuesOptions {
  searchText: string;
  replaceText: string;
  targetColumn: ReplaceTargetColumn;
  caseSensitive: boolean;
  mode: "next" | "all";
  startFromCueId?: number | null;
}

const MAX_UNDO_HISTORY = 50;

const DEFAULT_SEGMENT_RULES: SegmentRules = getPresetSegmentRules("standard");

const DEFAULT_CONFIG: AppConfig = {
  uiLanguage: "sr",
  modelsDirOverride: null,
  activeWhisperModelId: "whisper-large-v3-turbo-q5_0",
  activeLlmModelId: "llm-gemma4-e4b-qat",
  customModels: [],
  translation: {
    provider: "localLlama",
    deepseekModel: "deepseek-chat",
    customBaseUrl: "https://api.openai.com/v1",
    customModel: "gpt-4o-mini",
    batchSize: 20,
    contextWindow: 4096,
  },
  defaultSegmentRules: DEFAULT_SEGMENT_RULES,
  defaultScript: "latin",
  export: {
    outputDir: null,
    saveBesideAudio: true,
    includeSourceSrt: true,
    formats: ["srt"],
    utf8Bom: true,
  },
  hasCompletedSetup: false,
};

const DEFAULT_JOB_OPTIONS: JobOptions = {
  sourceLanguage: "auto",
  targetLanguage: "sr",
  script: "latin",
  segmentRules: DEFAULT_SEGMENT_RULES,
  glossary: {},
  polishSerbian: true,
  providerOverride: null,
};

interface AppStoreState {
  initialized: boolean;
  setupStatus: SetupStatus | null;
  catalog: CatalogManifest | null;
  config: AppConfig;
  apiKeyStatus: ApiKeyStatus;
  checkpoints: SubtitleProject[];

  jobOptions: JobOptions;
  batchQueue: BatchQueueItem[];
  activeJobId: string | null;
  activeProject: SubtitleProject | null;
  isRunningBatch: boolean;

  downloadProgress: Record<string, DownloadProgressEvent>;
  installingBundle: boolean;
  installingItemIds: Record<string, boolean>;

  activeCueId: number | null;
  playbackRequest: PlaybackRequest | null;

  undoStack: SubtitleCue[][];
  redoStack: SubtitleCue[][];
  canUndo: boolean;
  canRedo: boolean;

  isSetupOpen: boolean;
  isSettingsOpen: boolean;
  isExportOpen: boolean;
  isUpdaterOpen: boolean;
  isCheckpointsOpen: boolean;
  onlineConfirm: OnlineConfirmState;
  onlineApprovedForSession: boolean;

  banner: BannerNotification | null;

  initApp: () => Promise<void>;
  setBanner: (banner: BannerNotification | null) => void;
  setUiLanguage: (lang: UiLanguage) => Promise<void>;
  updateConfig: (patch: Partial<AppConfig>) => Promise<void>;
  refreshSetupAndCatalog: () => Promise<void>;

  setSetupOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setExportOpen: (open: boolean) => void;
  setUpdaterOpen: (open: boolean) => void;
  setCheckpointsOpen: (open: boolean) => void;

  runInstallComponent: (itemId: string) => Promise<void>;
  runInstallRecommendedBundle: () => Promise<void>;
  runImportCustomModel: (
    sourcePath: string,
    kind: CatalogItemKind,
    name?: string | null,
  ) => Promise<void>;
  runDeleteModel: (modelId: string) => Promise<void>;
  runSetApiKey: (
    provider: TranslationProviderKind,
    apiKey: string,
  ) => Promise<void>;
  runClearApiKey: (provider: TranslationProviderKind) => Promise<void>;

  setJobOptions: (patch: Partial<JobOptions>) => void;
  setEffectiveProvider: (provider: TranslationProviderKind) => Promise<void>;
  setSegmentPreset: (preset: SegmentPreset) => void;
  updateSegmentRules: (patch: Partial<SegmentRules>) => void;
  applyResegmentToActiveProject: () => Promise<void>;
  convertActiveProjectScript: (targetScript: SerbianScript) => Promise<void>;

  enqueueAudioFiles: (paths: string[]) => void;
  importSubtitleFromPath: (path: string) => Promise<void>;
  importSubtitleFromText: (fileName: string, content: string) => void;
  removeQueueItem: (jobId: string) => void;
  clearFinishedQueueItems: () => void;
  selectQueueItem: (jobId: string) => void;

  loadCheckpointIntoEditor: (jobId: string) => Promise<void>;
  removeCheckpoint: (jobId: string) => Promise<void>;

  closeOnlineConfirm: () => void;
  confirmOnlineAction: (rememberForSession: boolean) => void;

  startBatchPipeline: (mode: "full" | "transcribeOnly") => void;
  translateActiveProject: () => void;
  polishActiveProject: () => void;
  cancelJobById: (jobId: string) => Promise<void>;

  updateCue: (
    cueId: number,
    patch: Partial<
      Pick<SubtitleCue, "startMs" | "endMs" | "sourceText" | "translatedText">
    >,
  ) => void;
  nudgeCueTime: (
    cueId: number,
    edge: "start" | "end",
    deltaMs: number,
  ) => void;
  splitCueAtMidpoint: (cueId: number) => void;
  mergeCueWithNext: (cueId: number) => void;
  deleteCueById: (cueId: number) => void;
  addCueAtEnd: () => void;
  replaceInCues: (options: ReplaceInCuesOptions) => number;
  undoCueEdit: () => void;
  redoCueEdit: () => void;

  playCueSegment: (cue: SubtitleCue) => void;
  setActiveCueId: (cueId: number | null) => void;

  runExportSubtitles: (options: ExportOptions) => Promise<ExportResult | null>;
  quickExportSrt: () => Promise<void>;
}

let listenersRegistered = false;

export const useAppStore = create<AppStoreState>((set, get) => {
  const withOnlineGuard = (
    requiresLlm: boolean,
    actionLabel: string,
    action: () => Promise<void>,
  ) => {
    const state = get();
    const provider =
      state.jobOptions.providerOverride ?? state.config.translation.provider;
    const isCloud = provider === "deepSeek" || provider === "customOpenAi";

    if (requiresLlm && isCloud && !state.onlineApprovedForSession) {
      set({
        onlineConfirm: {
          open: true,
          provider,
          actionLabel,
          onConfirm: () => {
            void action();
          },
        },
      });
      return;
    }

    void action();
  };

  const syncProjectIntoQueueAndState = (project: SubtitleProject) => {
    set((state) => {
      const existsInQueue = state.batchQueue.some(
        (item) => item.jobId === project.jobId,
      );
      const updatedQueue = existsInQueue
        ? state.batchQueue.map((item) =>
            item.jobId === project.jobId
              ? {
                  ...item,
                  phase: project.phase,
                  percent: project.phase === "completed" ? 100 : item.percent,
                  project,
                  errorMessage: project.errorMessage,
                }
              : item,
          )
        : [
            {
              jobId: project.jobId,
              audioPath: project.audioPath,
              audioFileName: project.audioFileName,
              phase: project.phase,
              percent: project.phase === "completed" ? 100 : 0,
              detail: project.phase,
              project,
              errorMessage: project.errorMessage,
            },
            ...state.batchQueue,
          ];

      const updatedCheckpoints = [
        project,
        ...state.checkpoints.filter((c) => c.jobId !== project.jobId),
      ];

      return {
        activeProject: project,
        activeJobId: project.jobId,
        batchQueue: updatedQueue,
        checkpoints: updatedCheckpoints,
      };
    });
  };

  const cloneCuesSnapshot = (cues: SubtitleCue[]): SubtitleCue[] =>
    cues.map((c) => ({ ...c, warnings: [...c.warnings] }));

  const pushUndoSnapshot = (currentCues: SubtitleCue[]) => {
    set((state) => {
      const nextUndo = [
        ...state.undoStack.slice(-(MAX_UNDO_HISTORY - 1)),
        cloneCuesSnapshot(currentCues),
      ];
      return {
        undoStack: nextUndo,
        redoStack: [],
        canUndo: nextUndo.length > 0,
        canRedo: false,
      };
    });
  };

  const resetUndoHistory = () => {
    set({
      undoStack: [],
      redoStack: [],
      canUndo: false,
      canRedo: false,
    });
  };

  return {
    initialized: false,
    setupStatus: null,
    catalog: null,
    config: DEFAULT_CONFIG,
    apiKeyStatus: {
      hasDeepSeekKey: false,
      hasCustomApiKey: false,
      storageBackend: "keyring",
    },
    checkpoints: [],

    jobOptions: DEFAULT_JOB_OPTIONS,
    batchQueue: [],
    activeJobId: null,
    activeProject: null,
    isRunningBatch: false,

    downloadProgress: {},
    installingBundle: false,
    installingItemIds: {},

    activeCueId: null,
    playbackRequest: null,

    undoStack: [],
    redoStack: [],
    canUndo: false,
    canRedo: false,

    isSetupOpen: false,
    isSettingsOpen: false,
    isExportOpen: false,
    isUpdaterOpen: false,
    isCheckpointsOpen: false,
    onlineConfirm: {
      open: false,
      provider: "deepSeek",
      actionLabel: "",
      onConfirm: null,
    },
    onlineApprovedForSession: false,

    banner: null,

    initApp: async () => {
      if (get().initialized) {
        return;
      }

      if (!listenersRegistered) {
        listenersRegistered = true;
        void onDownloadProgress((evt) => {
          set((state) => ({
            downloadProgress: {
              ...state.downloadProgress,
              [evt.itemId]: evt,
            },
          }));
        });

        void onJobProgress((evt) => {
          set((state) => ({
            batchQueue: state.batchQueue.map((item) =>
              item.jobId === evt.jobId
                ? {
                    ...item,
                    phase: evt.phase,
                    percent: evt.percent,
                    detail: evt.detail,
                  }
                : item,
            ),
            activeProject:
              state.activeProject && state.activeProject.jobId === evt.jobId
                ? {
                    ...state.activeProject,
                    phase: evt.phase,
                  }
                : state.activeProject,
          }));
        });
      }

      try {
        const [setupStatus, catalog, loadedConfig, apiKeyStatus, checkpoints] =
          await Promise.all([
            getSetupStatus(),
            getCatalog(),
            getConfig(),
            getApiKeyStatus(),
            listCheckpoints(),
          ]);

        const config: AppConfig = {
          ...DEFAULT_CONFIG,
          ...loadedConfig,
          uiLanguage: normalizeUiLanguage(loadedConfig.uiLanguage),
        };

        const jobOptions: JobOptions = {
          ...DEFAULT_JOB_OPTIONS,
          script: config.defaultScript,
          segmentRules: config.defaultSegmentRules,
          providerOverride: config.translation.provider,
        };

        const shouldShowSetup =
          !config.hasCompletedSetup && !setupStatus.readyForOffline;

        set({
          initialized: true,
          setupStatus,
          catalog,
          config,
          apiKeyStatus,
          checkpoints,
          jobOptions,
          isSetupOpen: shouldShowSetup,
        });
      } catch (err) {
        set({
          initialized: true,
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    setBanner: (banner) => set({ banner }),

    setUiLanguage: async (lang) => {
      const nextConfig: AppConfig = {
        ...get().config,
        uiLanguage: lang,
      };
      set({ config: nextConfig });
      try {
        const saved = await saveConfig(nextConfig);
        set({ config: saved });
      } catch {
        // Keep optimistic UI language
      }
    },

    updateConfig: async (patch) => {
      const nextConfig: AppConfig = {
        ...get().config,
        ...patch,
      };
      set({ config: nextConfig });
      try {
        const saved = await saveConfig(nextConfig);
        set({ config: saved });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    refreshSetupAndCatalog: async () => {
      try {
        const [setupStatus, catalog, config, apiKeyStatus] = await Promise.all([
          getSetupStatus(),
          getCatalog(),
          getConfig(),
          getApiKeyStatus(),
        ]);
        set({ setupStatus, catalog, config, apiKeyStatus });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    setSetupOpen: (open) => set({ isSetupOpen: open }),
    setSettingsOpen: (open) => set({ isSettingsOpen: open }),
    setExportOpen: (open) => set({ isExportOpen: open }),
    setUpdaterOpen: (open) => set({ isUpdaterOpen: open }),
    setCheckpointsOpen: (open) => set({ isCheckpointsOpen: open }),

    runInstallComponent: async (itemId) => {
      set((state) => ({
        installingItemIds: { ...state.installingItemIds, [itemId]: true },
      }));
      try {
        await installComponent(itemId);
        const setupStatus = await getSetupStatus();
        set({ setupStatus });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      } finally {
        set((state) => {
          const next = { ...state.installingItemIds };
          delete next[itemId];
          return { installingItemIds: next };
        });
      }
    },

    runInstallRecommendedBundle: async () => {
      set({ installingBundle: true });
      try {
        const setupStatus = await installRecommendedBundle();
        const nextConfig = await saveConfig({
          ...get().config,
          hasCompletedSetup: true,
        });
        set({
          setupStatus,
          config: nextConfig,
          banner: {
            type: "success",
            message:
              nextConfig.uiLanguage === "en"
                ? "Recommended GPU bundle installed successfully!"
                : "Preporučeni paket za vaš GPU je uspešno instaliran!",
          },
        });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      } finally {
        set({ installingBundle: false });
      }
    },

    runImportCustomModel: async (sourcePath, kind, name) => {
      try {
        await importCustomModel(sourcePath, kind, name);
        const [config, setupStatus] = await Promise.all([
          getConfig(),
          getSetupStatus(),
        ]);
        set({
          config,
          setupStatus,
          banner: {
            type: "success",
            message:
              config.uiLanguage === "en"
                ? "Custom model imported successfully."
                : "Prilagođeni model je uspešno uvezen.",
          },
        });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    runDeleteModel: async (modelId) => {
      try {
        await deleteModel(modelId);
        const [config, setupStatus] = await Promise.all([
          getConfig(),
          getSetupStatus(),
        ]);
        set({ config, setupStatus });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    runSetApiKey: async (provider, apiKey) => {
      try {
        const apiKeyStatus = await setApiKey(provider, apiKey);
        set({
          apiKeyStatus,
          banner: {
            type: "success",
            message:
              get().config.uiLanguage === "en"
                ? "API key saved securely."
                : "API ključ je bezbedno sačuvan.",
          },
        });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    runClearApiKey: async (provider) => {
      try {
        const apiKeyStatus = await clearApiKey(provider);
        set({ apiKeyStatus });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    setJobOptions: (patch) => {
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          ...patch,
        },
      }));
    },

    setEffectiveProvider: async (provider) => {
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          providerOverride: provider,
        },
      }));
      await get().updateConfig({
        translation: {
          ...get().config.translation,
          provider,
        },
      });
    },

    setSegmentPreset: (preset) => {
      const rules = getPresetSegmentRules(
        preset,
        get().jobOptions.segmentRules,
      );
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          segmentRules: rules,
        },
      }));
    },

    updateSegmentRules: (patch) => {
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          segmentRules: {
            ...state.jobOptions.segmentRules,
            ...patch,
            preset: patch.preset ?? "custom",
          },
        },
      }));
    },

    applyResegmentToActiveProject: async () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      try {
        const updated = await resegmentProject(
          activeProject,
          jobOptions.segmentRules,
        );
        syncProjectIntoQueueAndState(updated);
        set({
          banner: {
            type: "success",
            message:
              get().config.uiLanguage === "en"
                ? "Project re-segmented with new rules."
                : "Titlovi su ponovo segmentirani prema novim pravilima.",
          },
        });
      } catch (err) {
        // Fallback local recalculation if words array is empty
        const updatedCues = activeProject.cues.map((cue) =>
          recalculateCue(cue, jobOptions.segmentRules),
        );
        syncProjectIntoQueueAndState({
          ...activeProject,
          options: {
            ...activeProject.options,
            segmentRules: jobOptions.segmentRules,
          },
          cues: updatedCues,
        });
        if (err instanceof Error && err.message) {
          set({
            banner: {
              type: "info",
              message: err.message,
            },
          });
        }
      }
    },

    convertActiveProjectScript: async (targetScript) => {
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          script: targetScript,
        },
      }));
      const { activeProject, jobOptions } = get();
      if (!activeProject || activeProject.cues.length === 0) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      try {
        const convertedCues = await convertCuesScript(
          activeProject.cues,
          targetScript,
        );
        const recalculated = convertedCues.map((c) =>
          recalculateCue(c, jobOptions.segmentRules),
        );
        const updated: SubtitleProject = {
          ...activeProject,
          options: {
            ...activeProject.options,
            script: targetScript,
          },
          cues: recalculated,
          updatedAt: new Date().toISOString(),
        };
        syncProjectIntoQueueAndState(updated);
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    importSubtitleFromPath: async (path) => {
      try {
        const project = await importSubtitleFile(path, get().jobOptions);
        resetUndoHistory();
        syncProjectIntoQueueAndState(project);
        set({
          activeCueId: project.cues[0]?.id ?? null,
          banner: {
            type: "success",
            message:
              get().config.uiLanguage === "en"
                ? `Imported subtitle file: ${project.audioFileName} (${project.cues.length} cues)`
                : `Uvezen fajl titla: ${project.audioFileName} (${project.cues.length} titlova)`,
          },
        });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    importSubtitleFromText: (fileName, content) => {
      const { jobOptions, config } = get();
      const parsed = parseSubtitleFileText(content, jobOptions.segmentRules);
      if (parsed.length === 0) {
        set({
          banner: {
            type: "error",
            message:
              config.uiLanguage === "en"
                ? "Subtitle file is empty or has no valid timestamp cues."
                : "Fajl titla je prazan ili ne sadrži ispravne vremenske kodove.",
          },
        });
        return;
      }

      const cues = parsed.map((c) =>
        jobOptions.sourceLanguage === "sr"
          ? recalculateCue(
              {
                ...c,
                sourceText: convertSerbianScriptText(
                  c.sourceText,
                  jobOptions.script,
                ),
              },
              jobOptions.segmentRules,
            )
          : c,
      );

      const durationMs = cues.reduce((max, c) => Math.max(max, c.endMs), 0);
      const cleanName = fileName.split(/[\\/]/).pop() || fileName;
      const jobId = `import-${Date.now()}-${Math.random()
        .toString(36)
        .slice(2, 7)}`;

      const project: SubtitleProject = {
        jobId,
        audioPath: fileName,
        audioFileName: cleanName,
        durationMs,
        detectedLanguage:
          jobOptions.sourceLanguage === "auto"
            ? null
            : jobOptions.sourceLanguage,
        options: { ...jobOptions },
        words: [],
        cues,
        phase: "completed",
        completedTranslationBatches: 0,
        errorMessage: null,
        updatedAt: new Date().toISOString(),
      };

      resetUndoHistory();
      syncProjectIntoQueueAndState(project);
      set({
        activeCueId: cues[0]?.id ?? null,
        banner: {
          type: "success",
          message:
            config.uiLanguage === "en"
              ? `Imported subtitle file: ${cleanName} (${cues.length} cues)`
              : `Uvezen fajl titla: ${cleanName} (${cues.length} titlova)`,
        },
      });
    },

    enqueueAudioFiles: (paths) => {
      if (paths.length === 0) {
        return;
      }
      set((state) => {
        const existingPaths = new Set(
          state.batchQueue.map((item) => item.audioPath),
        );
        const newItems: BatchQueueItem[] = [];
        for (const audioPath of paths) {
          if (existingPaths.has(audioPath)) {
            continue;
          }
          const audioFileName =
            audioPath.split(/[\\/]/).pop() || "audio-input.wav";
          const jobId = `job-${Date.now()}-${Math.random()
            .toString(36)
            .slice(2, 7)}`;
          newItems.push({
            jobId,
            audioPath,
            audioFileName,
            phase: "queued",
            percent: 0,
            detail: "",
            project: null,
            errorMessage: null,
          });
        }
        const combined = [...state.batchQueue, ...newItems];
        const nextActiveId =
          state.activeJobId ?? combined[0]?.jobId ?? null;
        return {
          batchQueue: combined,
          activeJobId: nextActiveId,
        };
      });
    },

    removeQueueItem: (jobId) => {
      set((state) => {
        const nextQueue = state.batchQueue.filter(
          (item) => item.jobId !== jobId,
        );
        const isRemovingActive = state.activeJobId === jobId;
        const fallbackItem = nextQueue[0] ?? null;
        return {
          batchQueue: nextQueue,
          activeJobId: isRemovingActive
            ? (fallbackItem?.jobId ?? null)
            : state.activeJobId,
          activeProject: isRemovingActive
            ? (fallbackItem?.project ?? null)
            : state.activeProject,
        };
      });
    },

    clearFinishedQueueItems: () => {
      set((state) => ({
        batchQueue: state.batchQueue.filter(
          (item) =>
            item.phase !== "completed" &&
            item.phase !== "cancelled" &&
            item.phase !== "failed",
        ),
      }));
    },

    selectQueueItem: (jobId) => {
      const item = get().batchQueue.find((q) => q.jobId === jobId);
      if (!item) {
        return;
      }
      resetUndoHistory();
      set({
        activeJobId: jobId,
        activeProject: item.project ?? get().activeProject,
        activeCueId: null,
      });
    },

    loadCheckpointIntoEditor: async (jobId) => {
      try {
        const project = await loadCheckpoint(jobId);
        resetUndoHistory();
        syncProjectIntoQueueAndState(project);
        set({
          jobOptions: { ...project.options },
          isCheckpointsOpen: false,
          activeCueId: null,
        });
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    removeCheckpoint: async (jobId) => {
      try {
        await deleteCheckpoint(jobId);
        set((state) => ({
          checkpoints: state.checkpoints.filter((c) => c.jobId !== jobId),
        }));
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    closeOnlineConfirm: () => {
      set({
        onlineConfirm: {
          open: false,
          provider: "deepSeek",
          actionLabel: "",
          onConfirm: null,
        },
      });
    },

    confirmOnlineAction: (rememberForSession) => {
      const callback = get().onlineConfirm.onConfirm;
      set({
        onlineApprovedForSession:
          rememberForSession || get().onlineApprovedForSession,
        onlineConfirm: {
          open: false,
          provider: "deepSeek",
          actionLabel: "",
          onConfirm: null,
        },
      });
      if (callback) {
        callback();
      }
    },

    startBatchPipeline: (mode) => {
      const state = get();
      const requiresLlm =
        mode === "full" &&
        (state.jobOptions.targetLanguage !== "none" ||
          state.jobOptions.polishSerbian);

      withOnlineGuard(
        requiresLlm,
        mode === "full" ? "1-Click Pipeline" : "Transcribe",
        async () => {
          let queue = get().batchQueue;
          if (queue.length === 0 && get().activeProject) {
            const active = get().activeProject!;
            get().enqueueAudioFiles([active.audioPath]);
            queue = get().batchQueue;
          }
          if (queue.length === 0) {
            set({
              banner: {
                type: "info",
                message:
                  get().config.uiLanguage === "en"
                    ? "Please add at least one audio file first."
                    : "Dodajte bar jedan audio fajl pre pokretanja obrade.",
              },
            });
            return;
          }

          set({ isRunningBatch: true });

          const runnable = queue.filter(
            (item) =>
              item.phase === "queued" ||
              item.phase === "failed" ||
              item.phase === "cancelled" ||
              item.phase === "completed",
          );

          for (const item of runnable) {
            const currentOptions = { ...get().jobOptions };
            set((s) => ({
              activeJobId: item.jobId,
              batchQueue: s.batchQueue.map((q) =>
                q.jobId === item.jobId
                  ? {
                      ...q,
                      phase: "convertingAudio",
                      percent: 5,
                      errorMessage: null,
                    }
                  : q,
              ),
            }));

            try {
              const project =
                mode === "full"
                  ? await runFullPipeline(
                      item.audioPath,
                      currentOptions,
                      item.jobId,
                    )
                  : await transcribeAudio(
                      item.audioPath,
                      currentOptions,
                      item.jobId,
                    );
              syncProjectIntoQueueAndState(project);
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);
              set((s) => ({
                batchQueue: s.batchQueue.map((q) =>
                  q.jobId === item.jobId
                    ? {
                        ...q,
                        phase: "failed",
                        errorMessage: msg,
                      }
                    : q,
                ),
                banner: {
                  type: "error",
                  message: `${item.audioFileName}: ${msg}`,
                },
              }));
            }
          }

          const checkpoints = await listCheckpoints().catch(
            () => get().checkpoints,
          );
          set({ isRunningBatch: false, checkpoints });
        },
      );
    },

    translateActiveProject: () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      withOnlineGuard(true, "Translate Project", async () => {
        const projectWithUpdatedOptions: SubtitleProject = {
          ...activeProject,
          options: {
            ...activeProject.options,
            ...jobOptions,
            targetLanguage:
              jobOptions.targetLanguage === "none"
                ? "sr"
                : jobOptions.targetLanguage,
          },
          phase: "translating",
        };
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const translated = await translateProject(projectWithUpdatedOptions);
          syncProjectIntoQueueAndState(translated);
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          set({
            banner: {
              type: "error",
              message: msg,
            },
          });
        }
      });
    },

    polishActiveProject: () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      withOnlineGuard(true, "Polish Serbian Transcript", async () => {
        const projectWithUpdatedOptions: SubtitleProject = {
          ...activeProject,
          options: {
            ...activeProject.options,
            ...jobOptions,
            polishSerbian: true,
          },
          phase: "polishing",
        };
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const polished = await polishProject(projectWithUpdatedOptions);
          syncProjectIntoQueueAndState(polished);
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          set({
            banner: {
              type: "error",
              message: msg,
            },
          });
        }
      });
    },

    cancelJobById: async (jobId) => {
      try {
        await cancelJob(jobId);
      } finally {
        set((state) => ({
          batchQueue: state.batchQueue.map((item) =>
            item.jobId === jobId
              ? { ...item, phase: "cancelled", detail: "Cancelled" }
              : item,
          ),
        }));
      }
    },

    updateCue: (cueId, patch) => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      const existing = activeProject.cues.find((c) => c.id === cueId);
      if (!existing) {
        return;
      }
      const hasChange =
        (patch.startMs !== undefined && patch.startMs !== existing.startMs) ||
        (patch.endMs !== undefined && patch.endMs !== existing.endMs) ||
        (patch.sourceText !== undefined &&
          patch.sourceText !== existing.sourceText) ||
        (patch.translatedText !== undefined &&
          patch.translatedText !== existing.translatedText);
      if (!hasChange) {
        return;
      }

      pushUndoSnapshot(activeProject.cues);
      const rules = jobOptions.segmentRules;
      const updatedCues = activeProject.cues.map((cue) => {
        if (cue.id !== cueId) {
          return cue;
        }
        const merged: SubtitleCue = {
          ...cue,
          ...patch,
        };
        if (merged.endMs <= merged.startMs) {
          merged.endMs = merged.startMs + 500;
        }
        return recalculateCue(merged, rules);
      });

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: updatedCues,
        updatedAt: new Date().toISOString(),
      });
    },

    nudgeCueTime: (cueId, edge, deltaMs) => {
      const { activeProject } = get();
      if (!activeProject) {
        return;
      }
      const target = activeProject.cues.find((c) => c.id === cueId);
      if (!target) {
        return;
      }
      if (edge === "start") {
        const nextStart = Math.max(
          0,
          Math.min(target.endMs - 200, target.startMs + deltaMs),
        );
        get().updateCue(cueId, { startMs: nextStart });
      } else {
        const nextEnd = Math.max(target.startMs + 200, target.endMs + deltaMs);
        get().updateCue(cueId, { endMs: nextEnd });
      }
    },

    splitCueAtMidpoint: (cueId) => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      const idx = activeProject.cues.findIndex((c) => c.id === cueId);
      if (idx === -1) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      const cue = activeProject.cues[idx];
      const midMs = Math.round((cue.startMs + cue.endMs) / 2);

      const splitWords = (text: string): [string, string] => {
        const trimmed = text.trim();
        if (!trimmed) {
          return ["", ""];
        }
        const lines = trimmed.split("\n");
        if (lines.length >= 2) {
          const half = Math.ceil(lines.length / 2);
          return [
            lines.slice(0, half).join("\n"),
            lines.slice(half).join("\n"),
          ];
        }
        const words = trimmed.split(/\s+/);
        if (words.length <= 1) {
          return [trimmed, ""];
        }
        const half = Math.ceil(words.length / 2);
        return [words.slice(0, half).join(" "), words.slice(half).join(" ")];
      };

      const [src1, src2] = splitWords(cue.sourceText);
      const [tr1, tr2] = splitWords(cue.translatedText);

      const firstHalf = recalculateCue(
        {
          ...cue,
          endMs: midMs,
          sourceText: src1,
          translatedText: tr1,
        },
        jobOptions.segmentRules,
      );

      const secondHalf = recalculateCue(
        {
          ...cue,
          startMs: midMs,
          sourceText: src2,
          translatedText: tr2,
        },
        jobOptions.segmentRules,
      );

      const nextCues = [
        ...activeProject.cues.slice(0, idx),
        firstHalf,
        secondHalf,
        ...activeProject.cues.slice(idx + 1),
      ].map((item, i) => ({ ...item, id: i + 1 }));

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      });
    },

    mergeCueWithNext: (cueId) => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      const idx = activeProject.cues.findIndex((c) => c.id === cueId);
      if (idx === -1 || idx + 1 >= activeProject.cues.length) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      const current = activeProject.cues[idx];
      const next = activeProject.cues[idx + 1];

      const merged = recalculateCue(
        {
          id: current.id,
          startMs: current.startMs,
          endMs: next.endMs,
          sourceText: [current.sourceText.trim(), next.sourceText.trim()]
            .filter(Boolean)
            .join(" "),
          translatedText: [
            current.translatedText.trim(),
            next.translatedText.trim(),
          ]
            .filter(Boolean)
            .join("\n"),
          cps: 0,
          warnings: [],
        },
        jobOptions.segmentRules,
      );

      const nextCues = [
        ...activeProject.cues.slice(0, idx),
        merged,
        ...activeProject.cues.slice(idx + 2),
      ].map((item, i) => ({ ...item, id: i + 1 }));

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      });
    },

    deleteCueById: (cueId) => {
      const { activeProject } = get();
      if (!activeProject) {
        return;
      }
      if (!activeProject.cues.some((c) => c.id === cueId)) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      const nextCues = activeProject.cues
        .filter((c) => c.id !== cueId)
        .map((item, i) => ({ ...item, id: i + 1 }));

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      });
    },

    addCueAtEnd: () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      const lastCue = activeProject.cues[activeProject.cues.length - 1];
      const startMs = lastCue ? lastCue.endMs + 200 : 0;
      const endMs = startMs + 2500;
      const baseCue = {
        id: activeProject.cues.length + 1,
        startMs,
        endMs,
        sourceText: "",
        translatedText: "",
      };
      const metrics = calculateCueMetrics(baseCue, jobOptions.segmentRules);
      const newCue: SubtitleCue = {
        ...baseCue,
        ...metrics,
      };

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: [...activeProject.cues, newCue],
        updatedAt: new Date().toISOString(),
      });
    },

    replaceInCues: (options) => {
      const { activeProject, jobOptions } = get();
      const {
        searchText,
        replaceText,
        targetColumn,
        caseSensitive,
        mode,
        startFromCueId,
      } = options;

      if (!activeProject || !searchText) {
        return 0;
      }

      const escaped = searchText.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const checkSource =
        targetColumn === "both" || targetColumn === "source";
      const checkTranslated =
        targetColumn === "both" || targetColumn === "translated";

      if (mode === "all") {
        const globalRegex = new RegExp(escaped, caseSensitive ? "g" : "gi");
        let totalReplaced = 0;

        const nextCues = activeProject.cues.map((cue) => {
          let nextSource = cue.sourceText;
          let nextTranslated = cue.translatedText;
          let cueChanged = false;

          if (checkSource && nextSource) {
            const matches = nextSource.match(globalRegex);
            if (matches && matches.length > 0) {
              totalReplaced += matches.length;
              nextSource = nextSource.replace(globalRegex, () => replaceText);
              cueChanged = true;
            }
          }

          if (checkTranslated && nextTranslated) {
            const matches = nextTranslated.match(globalRegex);
            if (matches && matches.length > 0) {
              totalReplaced += matches.length;
              nextTranslated = nextTranslated.replace(
                globalRegex,
                () => replaceText,
              );
              cueChanged = true;
            }
          }

          if (!cueChanged) {
            return cue;
          }

          return recalculateCue(
            {
              ...cue,
              sourceText: nextSource,
              translatedText: nextTranslated,
            },
            jobOptions.segmentRules,
          );
        });

        if (totalReplaced > 0) {
          pushUndoSnapshot(activeProject.cues);
          syncProjectIntoQueueAndState({
            ...activeProject,
            cues: nextCues,
            updatedAt: new Date().toISOString(),
          });
        }

        return totalReplaced;
      }

      // mode === "next"
      const singleRegex = new RegExp(escaped, caseSensitive ? "" : "i");
      const cues = activeProject.cues;
      if (cues.length === 0) {
        return 0;
      }

      const startIdx =
        startFromCueId != null
          ? Math.max(
              0,
              cues.findIndex((c) => c.id === startFromCueId),
            )
          : 0;

      for (let offset = 0; offset < cues.length; offset++) {
        const idx = (startIdx + offset) % cues.length;
        const cue = cues[idx];

        const sourceMatches =
          checkSource && Boolean(cue.sourceText) && singleRegex.test(cue.sourceText);
        const translatedMatches =
          checkTranslated &&
          Boolean(cue.translatedText) &&
          singleRegex.test(cue.translatedText);

        if (!sourceMatches && !translatedMatches) {
          continue;
        }

        pushUndoSnapshot(cues);
        const updatedCue = recalculateCue(
          {
            ...cue,
            sourceText: sourceMatches
              ? cue.sourceText.replace(singleRegex, () => replaceText)
              : cue.sourceText,
            translatedText:
              !sourceMatches && translatedMatches
                ? cue.translatedText.replace(singleRegex, () => replaceText)
                : cue.translatedText,
          },
          jobOptions.segmentRules,
        );

        const nextCues = cues.map((item, i) =>
          i === idx ? updatedCue : item,
        );

        set({ activeCueId: cue.id });
        syncProjectIntoQueueAndState({
          ...activeProject,
          cues: nextCues,
          updatedAt: new Date().toISOString(),
        });
        return 1;
      }

      return 0;
    },

    undoCueEdit: () => {
      const { activeProject, undoStack, redoStack } = get();
      if (!activeProject || undoStack.length === 0) {
        return;
      }
      const previousCues = undoStack[undoStack.length - 1];
      const nextUndo = undoStack.slice(0, -1);
      const nextRedo = [
        ...redoStack.slice(-(MAX_UNDO_HISTORY - 1)),
        cloneCuesSnapshot(activeProject.cues),
      ];

      set({
        undoStack: nextUndo,
        redoStack: nextRedo,
        canUndo: nextUndo.length > 0,
        canRedo: nextRedo.length > 0,
      });

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: previousCues,
        updatedAt: new Date().toISOString(),
      });
    },

    redoCueEdit: () => {
      const { activeProject, undoStack, redoStack } = get();
      if (!activeProject || redoStack.length === 0) {
        return;
      }
      const restoredCues = redoStack[redoStack.length - 1];
      const nextRedo = redoStack.slice(0, -1);
      const nextUndo = [
        ...undoStack.slice(-(MAX_UNDO_HISTORY - 1)),
        cloneCuesSnapshot(activeProject.cues),
      ];

      set({
        undoStack: nextUndo,
        redoStack: nextRedo,
        canUndo: nextUndo.length > 0,
        canRedo: nextRedo.length > 0,
      });

      syncProjectIntoQueueAndState({
        ...activeProject,
        cues: restoredCues,
        updatedAt: new Date().toISOString(),
      });
    },

    playCueSegment: (cue) => {
      set({
        activeCueId: cue.id,
        playbackRequest: {
          cueId: cue.id,
          startMs: cue.startMs,
          endMs: cue.endMs,
          nonce: Date.now() + Math.random(),
        },
      });
    },

    setActiveCueId: (cueId) => set({ activeCueId: cueId }),

    runExportSubtitles: async (options) => {
      const { activeProject } = get();
      if (!activeProject) {
        return null;
      }
      try {
        await get().updateConfig({ export: options });
        const result = await exportSubtitles(activeProject, options);
        set({
          banner: {
            type: "success",
            message: `${
              get().config.uiLanguage === "en"
                ? "Exported:"
                : "Izvezeno:"
            } ${result.writtenFiles.join(", ")}`,
          },
        });
        return result;
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
        return null;
      }
    },

    quickExportSrt: async () => {
      const { activeProject, config } = get();
      if (!activeProject) {
        return;
      }
      const quickOptions: ExportOptions = {
        ...config.export,
        formats: ["srt"],
      };
      await get().runExportSubtitles(quickOptions);
    },
  };
});
