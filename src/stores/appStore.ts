import { getVersion } from "@tauri-apps/api/app";
import { create } from "zustand";
import { normalizeUiLanguage, type UiLanguage } from "../i18n";
import {
  calculateCueMetrics,
  compareSemver,
  convertSerbianScriptText,
  getPresetSegmentRules,
  isTauriRuntime,
  parseSubtitleFileText,
  recalculateCue,
  resolveInitialBatchActiveJobId,
  shouldActivateSyncedProject,
} from "../lib/subtitleUtils";
import {
  cancelJob,
  clearApiKey,
  condenseProjectCps,
  convertCuesScript,
  deleteCheckpoint,
  deleteModel,
  exportSubtitles,
  flushPendingCrashReports,
  getApiKeyStatus,
  getCatalog,
  getConfig,
  getSetupStatus,
  importCustomModel,
  importSubtitleFile,
  installComponent,
  installCoreRuntime,
  installRecommendedBundle,
  listCheckpoints,
  loadCheckpoint,
  onDownloadProgress,
  onJobProgress,
  polishProject,
  resegmentProject,
  runFullPipeline,
  runLocalLlmBatch,
  saveConfig,
  setApiKey,
  transcribeAudio,
  translateProject,
  tryLoadCompanionSubtitles,
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
  uiLanguage: "en",
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
  autoSendCrashReports: true,
};

const DEFAULT_JOB_OPTIONS: JobOptions = {
  sourceLanguage: "auto",
  targetLanguage: "sr",
  script: "latin",
  segmentRules: DEFAULT_SEGMENT_RULES,
  glossary: {},
  polishSerbian: true,
  correctSourceAsr: false,
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
  isBugReportOpen: boolean;
  appVersion: string;
  availableUpdateVersion: string | null;
  onlineConfirm: OnlineConfirmState;
  onlineApprovedForSession: boolean;

  banner: BannerNotification | null;

  initApp: () => Promise<void>;
  setBanner: (banner: BannerNotification | null) => void;
  setAvailableUpdateVersion: (version: string | null) => void;
  setUiLanguage: (lang: UiLanguage) => Promise<void>;
  updateConfig: (patch: Partial<AppConfig>) => Promise<void>;
  refreshSetupAndCatalog: () => Promise<void>;

  setSetupOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setExportOpen: (open: boolean) => void;
  setUpdaterOpen: (open: boolean) => void;
  setCheckpointsOpen: (open: boolean) => void;
  setBugReportOpen: (open: boolean) => void;

  runInstallComponent: (itemId: string) => Promise<void>;
  runInstallCoreRuntime: () => Promise<void>;
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
  reprocessQueueItem: (jobId: string) => void;
  clearFinishedQueueItems: () => void;
  selectQueueItem: (jobId: string) => void;

  loadCheckpointIntoEditor: (jobId: string) => Promise<void>;
  removeCheckpoint: (jobId: string) => Promise<void>;

  closeOnlineConfirm: () => void;
  confirmOnlineAction: (rememberForSession: boolean) => void;

  startBatchPipeline: (mode: "full" | "transcribeOnly") => void;
  translateActiveProject: () => void;
  polishActiveProject: () => void;
  condenseActiveProjectCps: () => void;
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
  splitCueAtMidpoint: (cueId: number, splitMs?: number) => void;
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

  const findCueIdAtPlayheadZero = (cues: SubtitleCue[]): number | null => {
    const match = cues.find((c) => c.startMs <= 0 && c.endMs >= 0);
    return match ? match.id : null;
  };

  const syncProjectIntoQueueAndState = (
    project: SubtitleProject,
    options?: { forceActivate?: boolean },
  ) => {
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

      const shouldUpdateActive = shouldActivateSyncedProject(
        state.activeJobId,
        project.jobId,
        options?.forceActivate,
      );
      const isSwitchingActiveJob =
        shouldUpdateActive && state.activeJobId !== project.jobId;

      return {
        activeProject: shouldUpdateActive ? project : state.activeProject,
        activeJobId: shouldUpdateActive ? project.jobId : state.activeJobId,
        activeCueId: isSwitchingActiveJob
          ? findCueIdAtPlayheadZero(project.cues)
          : state.activeCueId,
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
    isBugReportOpen: false,
    appVersion: "0.3.2",
    availableUpdateVersion: null,
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
        const [
          setupStatus,
          catalog,
          loadedConfig,
          apiKeyStatus,
          checkpoints,
          appVersion,
        ] = await Promise.all([
          getSetupStatus(),
          getCatalog(),
          getConfig(),
          getApiKeyStatus(),
          listCheckpoints(),
          isTauriRuntime()
            ? getVersion().catch(() => "0.3.2")
            : Promise.resolve("0.3.2"),
        ]);

        const config: AppConfig = {
          ...DEFAULT_CONFIG,
          ...loadedConfig,
          autoSendCrashReports: loadedConfig.autoSendCrashReports ?? true,
          uiLanguage: normalizeUiLanguage(loadedConfig.uiLanguage),
        };

        const jobOptions: JobOptions = {
          ...DEFAULT_JOB_OPTIONS,
          script: config.defaultScript,
          segmentRules: config.defaultSegmentRules,
          providerOverride: config.translation.provider,
        };

        const hasCloudKey =
          apiKeyStatus.hasDeepSeekKey || apiKeyStatus.hasCustomApiKey;
        const hasTranslationReady =
          setupStatus.hasLlmModel ||
          (config.translation.provider !== "localLlama" && hasCloudKey);
        const shouldShowSetup =
          !config.hasCompletedSetup &&
          (!setupStatus.coreRuntimeReady || !hasTranslationReady);

        const stateBeforeInit = get();

        set({
          initialized: true,
          setupStatus,
          catalog,
          config,
          apiKeyStatus,
          checkpoints,
          jobOptions,
          appVersion,
          isSetupOpen: shouldShowSetup,
          activeProject: stateBeforeInit.activeProject,
          activeJobId: stateBeforeInit.activeJobId,
          activeCueId: stateBeforeInit.activeCueId,
          batchQueue: stateBeforeInit.batchQueue,
        });

        // Flush pending Rust panic / crash reports queued on disk (silent, non-blocking)
        if (config.autoSendCrashReports !== false) {
          void flushPendingCrashReports().catch(() => {});
        }

        // Background GitHub latest-release version check (silent, non-blocking)
        void fetch(
          "https://api.github.com/repos/Gruya13/EasySRT-Studio/releases/latest",
          { headers: { Accept: "application/vnd.github+json" } },
        )
          .then(async (res) => {
            if (!res.ok) return;
            const data = (await res.json()) as { tag_name?: string };
            const remoteTag = data.tag_name?.trim();
            if (remoteTag && compareSemver(remoteTag, appVersion) > 0) {
              set({ availableUpdateVersion: remoteTag.replace(/^v/i, "") });
            }
          })
          .catch(() => {
            // Offline or GitHub unreachable — ignore silently at startup
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
    setAvailableUpdateVersion: (version) =>
      set({ availableUpdateVersion: version }),

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
    setBugReportOpen: (open) => set({ isBugReportOpen: open }),

    runInstallComponent: async (itemId) => {
      set((state) => ({
        installingItemIds: { ...state.installingItemIds, [itemId]: true },
      }));
      try {
        await installComponent(itemId);
        const [setupStatus, config] = await Promise.all([
          getSetupStatus(),
          getConfig(),
        ]);
        set({ setupStatus, config });
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

    runInstallCoreRuntime: async () => {
      set({ installingBundle: true });
      try {
        const setupStatus = await installCoreRuntime();
        const config = await getConfig();
        set({
          setupStatus,
          config,
          banner: {
            type: "success",
            message:
              config.uiLanguage === "en"
                ? "Core AI runtime (FFmpeg, Whisper, llama.cpp, VAD) installed!"
                : "Osnovni AI sistem (FFmpeg, Whisper, llama.cpp, VAD) je uspešno instaliran!",
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
        syncProjectIntoQueueAndState(project, { forceActivate: true });
        set({
          activeCueId: findCueIdAtPlayheadZero(project.cues),
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
      syncProjectIntoQueueAndState(project, { forceActivate: true });
      set({
        activeCueId: findCueIdAtPlayheadZero(cues),
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
      const cleanedPaths = Array.from(
        new Set(paths.map((p) => p.trim()).filter((p) => p.length > 0)),
      );
      if (cleanedPaths.length === 0) {
        return;
      }

      const newlyAddedItems: BatchQueueItem[] = [];
      let existingToInspect: BatchQueueItem | null = null;

      set((state) => {
        const existingByPath = new Map(
          state.batchQueue.map((item) => [item.audioPath, item]),
        );
        const newItems: BatchQueueItem[] = [];
        for (const audioPath of cleanedPaths) {
          if (existingByPath.has(audioPath)) {
            continue;
          }
          const audioFileName =
            audioPath.split(/[\\/]/).pop() || "audio-input.wav";
          const jobId = `job-${Date.now()}-${Math.random()
            .toString(36)
            .slice(2, 7)}`;
          const newItem: BatchQueueItem = {
            jobId,
            audioPath,
            audioFileName,
            phase: "queued",
            percent: 0,
            detail: "",
            project: null,
            errorMessage: null,
          };
          newItems.push(newItem);
          existingByPath.set(audioPath, newItem);
        }

        newlyAddedItems.push(...newItems);

        if (newItems.length === 0) {
          const firstExisting = existingByPath.get(cleanedPaths[0]);
          if (!firstExisting) {
            return state;
          }
          if (firstExisting.project === null) {
            existingToInspect = firstExisting;
          }
          const targetProject: SubtitleProject =
            firstExisting.project ?? {
              jobId: firstExisting.jobId,
              audioPath: firstExisting.audioPath,
              audioFileName: firstExisting.audioFileName,
              durationMs: 0,
              detectedLanguage: null,
              options: { ...state.jobOptions },
              words: [],
              cues: [],
              phase: firstExisting.phase,
              completedTranslationBatches: 0,
              errorMessage: firstExisting.errorMessage,
              updatedAt: new Date().toISOString(),
            };
          return {
            activeJobId: firstExisting.jobId,
            activeProject: targetProject,
            activeCueId: findCueIdAtPlayheadZero(targetProject.cues),
          };
        }

        const combined = [...state.batchQueue, ...newItems];
        const shouldActivateFirst =
          state.activeJobId === null || state.activeProject === null;
        const firstAdded = newItems[0];
        const nextActiveId = shouldActivateFirst
          ? firstAdded.jobId
          : state.activeJobId;
        const nextActiveProject: SubtitleProject | null = shouldActivateFirst
          ? {
              jobId: firstAdded.jobId,
              audioPath: firstAdded.audioPath,
              audioFileName: firstAdded.audioFileName,
              durationMs: 0,
              detectedLanguage: null,
              options: { ...state.jobOptions },
              words: [],
              cues: [],
              phase: firstAdded.phase,
              completedTranslationBatches: 0,
              errorMessage: null,
              updatedAt: new Date().toISOString(),
            }
          : state.activeProject;

        return {
          batchQueue: combined,
          activeJobId: nextActiveId,
          activeProject: nextActiveProject,
          activeCueId: shouldActivateFirst ? null : state.activeCueId,
        };
      });

      const itemsToCheck =
        newlyAddedItems.length > 0
          ? newlyAddedItems
          : existingToInspect
            ? [existingToInspect]
            : [];
      if (itemsToCheck.length === 0) {
        return;
      }

      const optionsSnapshot = { ...get().jobOptions };
      const uiLang = get().config.uiLanguage;
      for (const item of itemsToCheck) {
        void tryLoadCompanionSubtitles(item.audioPath, optionsSnapshot)
          .then((companionProject) => {
            if (!companionProject) {
              return;
            }
            const mergedProject: SubtitleProject = {
              ...companionProject,
              jobId: item.jobId,
              audioPath: item.audioPath,
              audioFileName: item.audioFileName,
              phase: "completed",
            };
            set((state) => {
              const currentItem = state.batchQueue.find(
                (q) => q.jobId === item.jobId,
              );
              if (!currentItem) {
                return state;
              }
              if (
                currentItem.phase !== "queued" &&
                currentItem.phase !== "completed"
              ) {
                return state;
              }
              const updatedQueue = state.batchQueue.map((q) =>
                q.jobId === item.jobId
                  ? {
                      ...q,
                      phase: "completed" as const,
                      percent: 100,
                      detail:
                        uiLang === "en"
                          ? `Loaded ${mergedProject.cues.length} cues from companion SRT`
                          : `Učitano ${mergedProject.cues.length} titlova iz pratećeg SRT fajla`,
                      project: mergedProject,
                      errorMessage: null,
                    }
                  : q,
              );
              const isCurrentlyActive = state.activeJobId === item.jobId;
              return {
                batchQueue: updatedQueue,
                activeProject: isCurrentlyActive
                  ? mergedProject
                  : state.activeProject,
                activeCueId: isCurrentlyActive
                  ? findCueIdAtPlayheadZero(mergedProject.cues)
                  : state.activeCueId,
              };
            });
          })
          .catch(() => {
            // Keep file queued cleanly if companion subtitle fails to parse
          });
      }
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
          activeCueId: isRemovingActive
            ? findCueIdAtPlayheadZero(fallbackItem?.project?.cues ?? [])
            : state.activeCueId,
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
      const isSameJob = get().activeJobId === jobId;
      if (!isSameJob) {
        resetUndoHistory();
      }
      const placeholderProject: SubtitleProject | null = item.project
        ? item.project
        : {
            jobId: item.jobId,
            audioPath: item.audioPath,
            audioFileName: item.audioFileName,
            durationMs: 0,
            detectedLanguage: null,
            options: { ...get().jobOptions },
            words: [],
            cues: [],
            phase: item.phase,
            completedTranslationBatches: 0,
            errorMessage: item.errorMessage,
            updatedAt: new Date().toISOString(),
          };
      set({
        activeJobId: jobId,
        activeProject: placeholderProject ?? get().activeProject,
        activeCueId: isSameJob
          ? get().activeCueId
          : findCueIdAtPlayheadZero(placeholderProject.cues),
      });
    },

    loadCheckpointIntoEditor: async (jobId) => {
      try {
        const project = await loadCheckpoint(jobId);
        resetUndoHistory();
        syncProjectIntoQueueAndState(project, { forceActivate: true });
        set({
          jobOptions: {
            ...DEFAULT_JOB_OPTIONS,
            ...project.options,
            correctSourceAsr: Boolean(project.options.correctSourceAsr),
          },
          isCheckpointsOpen: false,
          activeCueId: findCueIdAtPlayheadZero(project.cues),
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

    reprocessQueueItem: (jobId) => {
      if (get().isRunningBatch) {
        return;
      }
      const target = get().batchQueue.find((q) => q.jobId === jobId);
      if (!target) {
        return;
      }
      set((state) => ({
        batchQueue: state.batchQueue.map((item) =>
          item.jobId === jobId
            ? {
                ...item,
                phase: "queued",
                percent: 0,
                detail: "queued",
                errorMessage: null,
              }
            : item,
        ),
        activeJobId: resolveInitialBatchActiveJobId(state.activeJobId, jobId),
      }));
      get().startBatchPipeline("full");
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
            if (!(isTauriRuntime() && active.jobId === "demo-session-01")) {
              get().enqueueAudioFiles([active.audioPath]);
              queue = get().batchQueue;
            }
          }

          const pendingItems = queue.filter(
            (item) =>
              item.phase === "queued" ||
              item.phase === "failed" ||
              item.phase === "cancelled",
          );
          const runnable =
            pendingItems.length > 0
              ? pendingItems
              : queue.filter(
                  (item) =>
                    item.phase === "completed" &&
                    !(isTauriRuntime() && item.jobId === "demo-session-01"),
                );

          if (runnable.length === 0) {
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

          set((s) => ({
            isRunningBatch: true,
            activeJobId: resolveInitialBatchActiveJobId(
              s.activeJobId,
              runnable[0]?.jobId ?? null,
            ),
          }));

          const effectiveProvider =
            get().jobOptions.providerOverride ??
            get().config.translation.provider;
          const canPipelineCloud =
            mode === "full" &&
            runnable.length > 1 &&
            (effectiveProvider === "deepSeek" ||
              effectiveProvider === "customOpenAi");
          const canPipelineLocal =
            mode === "full" &&
            runnable.length > 1 &&
            effectiveProvider === "localLlama";

          const handleJobError = async (
            jobId: string,
            audioFileName: string,
            err: unknown,
            fallbackProject?: SubtitleProject,
          ) => {
            const msg = err instanceof Error ? err.message : String(err);
            const recovered =
              fallbackProject ??
              (await loadCheckpoint(jobId).catch(() => null));
            if (recovered && recovered.cues.length > 0) {
              syncProjectIntoQueueAndState({
                ...recovered,
                phase: "failed",
                errorMessage: msg,
              });
            }
            set((s) => ({
              batchQueue: s.batchQueue.map((q) =>
                q.jobId === jobId
                  ? {
                      ...q,
                      phase: "failed",
                      errorMessage: msg,
                    }
                  : q,
              ),
              banner: {
                type: "error",
                message: `${audioFileName}: ${msg}`,
              },
            }));
          };

          const classifyLlmStage = (
            project: SubtitleProject,
            options: JobOptions,
          ): "translating" | "polishing" | null => {
            const rawSrc =
              project.detectedLanguage && project.detectedLanguage !== "auto"
                ? project.detectedLanguage
                : options.sourceLanguage;
            const srcLang = rawSrc.trim().toLowerCase();
            const dstLang = options.targetLanguage.trim().toLowerCase();
            const isSameSerbian =
              (srcLang === "sr" || srcLang === "hr" || srcLang === "bs") &&
              dstLang === "sr";

            if (
              Boolean(dstLang) &&
              dstLang !== "none" &&
              srcLang !== dstLang &&
              !isSameSerbian
            ) {
              return "translating";
            }
            if (
              (srcLang === "sr" || srcLang === "hr" || srcLang === "bs") &&
              options.polishSerbian
            ) {
              return "polishing";
            }
            return null;
          };

          if (canPipelineCloud) {
            // Pipeline GPU Whisper transcription (sequential on GPU) with
            // async cloud LLM translation/polishing across queued files.
            const pendingCloudTasks: Promise<void>[] = [];

            for (const item of runnable) {
              const currentOptions = { ...get().jobOptions };
              set((s) => ({
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
                const transcribed = await transcribeAudio(
                  item.audioPath,
                  currentOptions,
                  item.jobId,
                );
                syncProjectIntoQueueAndState(transcribed);

                const stagePhase = classifyLlmStage(
                  transcribed,
                  currentOptions,
                );
                if (stagePhase) {
                  set((s) => ({
                    batchQueue: s.batchQueue.map((q) =>
                      q.jobId === item.jobId
                        ? {
                            ...q,
                            phase: stagePhase,
                            percent: 55,
                          }
                        : q,
                    ),
                  }));

                  const projectForLlm: SubtitleProject = {
                    ...transcribed,
                    phase: stagePhase,
                  };

                  const cloudTask = (async () => {
                    try {
                      const finished =
                        stagePhase === "translating"
                          ? await translateProject(projectForLlm)
                          : await polishProject(projectForLlm);
                      syncProjectIntoQueueAndState(finished);
                    } catch (err) {
                      await handleJobError(
                        item.jobId,
                        item.audioFileName,
                        err,
                      );
                    }
                  })();
                  pendingCloudTasks.push(cloudTask);
                }
              } catch (err) {
                await handleJobError(item.jobId, item.audioFileName, err);
              }
            }

            await Promise.all(pendingCloudTasks);
          } else if (canPipelineLocal) {
            // Two-Phase Local Offline Batch:
            // Phase 1: Transcribe all files sequentially while Whisper owns GPU VRAM.
            // Phase 2: Translate/polish all qualifying projects in a single warm llama-server session.
            const transcribedEntries: Array<{
              project: SubtitleProject;
              options: JobOptions;
            }> = [];

            for (const item of runnable) {
              const currentOptions = { ...get().jobOptions };
              set((s) => ({
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
                const transcribed = await transcribeAudio(
                  item.audioPath,
                  currentOptions,
                  item.jobId,
                );
                const normalizedTranscribed: SubtitleProject = {
                  ...transcribed,
                  options: {
                    ...currentOptions,
                    segmentRules: transcribed.options.segmentRules,
                  },
                };
                syncProjectIntoQueueAndState(normalizedTranscribed);
                transcribedEntries.push({
                  project: normalizedTranscribed,
                  options: currentOptions,
                });
              } catch (err) {
                await handleJobError(item.jobId, item.audioFileName, err);
              }
            }

            const projectsForLlm: SubtitleProject[] = [];
            const llmStageByJobId = new Map<
              string,
              "translating" | "polishing"
            >();

            for (const { project, options: itemOptions } of transcribedEntries) {
              const stagePhase = classifyLlmStage(project, itemOptions);
              if (stagePhase) {
                llmStageByJobId.set(project.jobId, stagePhase);
                projectsForLlm.push({
                  ...project,
                  phase: stagePhase,
                });
              }
            }

            if (projectsForLlm.length > 0) {
              set((s) => ({
                batchQueue: s.batchQueue.map((q) => {
                  const stagePhase = llmStageByJobId.get(q.jobId);
                  return stagePhase
                    ? {
                        ...q,
                        phase: stagePhase,
                        percent: 55,
                      }
                    : q;
                }),
              }));

              try {
                const finishedProjects = await runLocalLlmBatch(projectsForLlm);
                for (const proj of finishedProjects) {
                  syncProjectIntoQueueAndState(proj);
                }
              } catch (err) {
                for (const proj of projectsForLlm) {
                  const recovered = await loadCheckpoint(proj.jobId).catch(
                    () => null,
                  );
                  if (recovered && recovered.phase === "completed") {
                    syncProjectIntoQueueAndState(recovered);
                  } else {
                    await handleJobError(
                      proj.jobId,
                      proj.audioFileName,
                      err,
                      recovered && recovered.cues.length > 0 ? recovered : proj,
                    );
                  }
                }
              }
            }
          } else {
            for (const item of runnable) {
              const currentOptions = { ...get().jobOptions };
              set((s) => ({
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
                await handleJobError(item.jobId, item.audioFileName, err);
              }
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
          syncProjectIntoQueueAndState({
            ...projectWithUpdatedOptions,
            phase: "failed",
            errorMessage: msg,
          });
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
          syncProjectIntoQueueAndState({
            ...projectWithUpdatedOptions,
            phase: "failed",
            errorMessage: msg,
          });
          set({
            banner: {
              type: "error",
              message: msg,
            },
          });
        }
      });
    },

    condenseActiveProjectCps: () => {
      const { activeProject, jobOptions, isRunningBatch } = get();
      if (!activeProject || activeProject.cues.length === 0 || isRunningBatch) {
        return;
      }
      withOnlineGuard(true, "Auto-Fit CPS", async () => {
        pushUndoSnapshot(activeProject.cues);
        const projectWithUpdatedOptions: SubtitleProject = {
          ...activeProject,
          options: {
            ...activeProject.options,
            ...jobOptions,
          },
          phase: "polishing",
        };
        set({ isRunningBatch: true });
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const condensed = await condenseProjectCps(projectWithUpdatedOptions);
          syncProjectIntoQueueAndState(condensed);
          const checkpoints = await listCheckpoints().catch(
            () => get().checkpoints,
          );
          set({
            checkpoints,
            banner: {
              type: "success",
              message:
                get().config.uiLanguage === "en"
                  ? "High-CPS cues automatically condensed."
                  : "Predugi titlovi su automatski sažeti (Auto-Fit CPS).",
            },
          });
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          const recovered = await loadCheckpoint(activeProject.jobId).catch(
            () => null,
          );
          if (recovered && recovered.cues.length > 0) {
            syncProjectIntoQueueAndState({
              ...recovered,
              phase: "failed",
              errorMessage: msg,
            });
          } else {
            syncProjectIntoQueueAndState({
              ...projectWithUpdatedOptions,
              phase: "failed",
              errorMessage: msg,
            });
          }
          set({
            banner: {
              type: "error",
              message: msg,
            },
          });
        } finally {
          set({ isRunningBatch: false });
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

    splitCueAtMidpoint: (cueId, splitMs) => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      const idx = activeProject.cues.findIndex((c) => c.id === cueId);
      if (idx === -1) {
        return;
      }
      const cue = activeProject.cues[idx];
      const MIN_SPLIT_MARGIN_MS = 80;
      const hasValidCustomSplit =
        typeof splitMs === "number" &&
        Number.isFinite(splitMs) &&
        splitMs >= cue.startMs + MIN_SPLIT_MARGIN_MS &&
        splitMs <= cue.endMs - MIN_SPLIT_MARGIN_MS;
      const cutMs = hasValidCustomSplit
        ? Math.round(splitMs)
        : Math.round((cue.startMs + cue.endMs) / 2);
      if (cutMs <= cue.startMs || cutMs >= cue.endMs) {
        return;
      }

      pushUndoSnapshot(activeProject.cues);

      const durationMs = Math.max(1, cue.endMs - cue.startMs);
      const ratio = Math.max(
        0.05,
        Math.min(0.95, (cutMs - cue.startMs) / durationMs),
      );
      const cueWords = activeProject.words.filter(
        (w) => w.endMs > cue.startMs + 20 && w.startMs < cue.endMs - 20,
      );

      const splitWords = (
        text: string,
        useTimedWords: boolean,
      ): [string, string] => {
        const trimmed = text.trim();
        if (!trimmed) {
          return ["", ""];
        }
        const rawTokens = trimmed.split(/\s+/).filter(Boolean);
        if (
          useTimedWords &&
          cueWords.length === rawTokens.length &&
          rawTokens.length >= 2
        ) {
          const wordsBeforeCut = cueWords.filter(
            (w) => (w.startMs + w.endMs) / 2 <= cutMs,
          ).length;
          const cutIdx = Math.max(
            1,
            Math.min(rawTokens.length - 1, wordsBeforeCut),
          );
          return [
            rawTokens.slice(0, cutIdx).join(" "),
            rawTokens.slice(cutIdx).join(" "),
          ];
        }

        const lines = trimmed.split("\n");
        if (!hasValidCustomSplit && lines.length >= 2) {
          const half = Math.ceil(lines.length / 2);
          return [
            lines.slice(0, half).join("\n"),
            lines.slice(half).join("\n"),
          ];
        }
        if (rawTokens.length <= 1) {
          return ratio < 0.5 ? ["", trimmed] : [trimmed, ""];
        }
        const cutIdx = Math.max(
          1,
          Math.min(rawTokens.length - 1, Math.round(rawTokens.length * ratio)),
        );
        return [
          rawTokens.slice(0, cutIdx).join(" "),
          rawTokens.slice(cutIdx).join(" "),
        ];
      };

      const [src1, src2] = splitWords(cue.sourceText, true);
      const [tr1, tr2] = splitWords(cue.translatedText, false);

      const firstHalf = recalculateCue(
        {
          ...cue,
          endMs: cutMs,
          sourceText: src1,
          translatedText: tr1,
        },
        jobOptions.segmentRules,
      );

      const secondHalf = recalculateCue(
        {
          ...cue,
          startMs: cutMs,
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
      set({ activeCueId: idx + 2 });
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
