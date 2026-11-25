import { getVersion } from "@tauri-apps/api/app";
import { create } from "zustand";
import { normalizeUiLanguage, type UiLanguage } from "../lib/i18nUtils.ts";
import { APP_VERSION } from "../lib/version.ts";
import {
  calculateCueMetrics,
  compareSemver,
  computeSplitBoundaries,
  convertSerbianScriptText,
  getPresetSegmentRules,
  isTauriRuntime,
  normalizeSegmentRules,
  normalizeTranslationStyle,
  parseSubtitleFileText,
  recalculateCue,
  resolveInitialBatchActiveJobId,
  shouldActivateSyncedProject,
} from "../lib/subtitleUtils.ts";
import {
  cancelJob,
  cancelMediaProbe,
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
  onProbeProgress,
  polishProject,
  probeMediaFile,
  resegmentProject,
  runFullPipeline,
  runLocalLlmBatch,
  saveConfig,
  saveProjectCheckpoint,
  setApiKey,
  transcribeAudio,
  translateProject,
  tryLoadCompanionSubtitles,
} from "../lib/tauri.ts";
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
  MediaProbeResult,
  SegmentPreset,
  SegmentRules,
  SerbianScript,
  SetupStatus,
  SubtitleCue,
  SubtitleProject,
  TranslationProviderKind,
  TranslationStyle,
} from "../types/models.ts";

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

export const EDITOR_SOURCE_FONT_MIN = 13;
export const EDITOR_SOURCE_FONT_MAX = 34;
export const EDITOR_SOURCE_FONT_DEFAULT = 19;
export const EDITOR_TRANSLATED_FONT_MIN = 14;
export const EDITOR_TRANSLATED_FONT_MAX = 36;
export const EDITOR_TRANSLATED_FONT_DEFAULT = 22;
export const CHECKPOINT_AUTOSAVE_DEBOUNCE_MS = 600;

export function clampEditorFontSize(
  target: "source" | "translated",
  size: number,
): number {
  const min =
    target === "source" ? EDITOR_SOURCE_FONT_MIN : EDITOR_TRANSLATED_FONT_MIN;
  const max =
    target === "source" ? EDITOR_SOURCE_FONT_MAX : EDITOR_TRANSLATED_FONT_MAX;
  const fallback =
    target === "source"
      ? EDITOR_SOURCE_FONT_DEFAULT
      : EDITOR_TRANSLATED_FONT_DEFAULT;
  if (!Number.isFinite(size)) {
    return fallback;
  }
  return Math.max(min, Math.min(max, Math.round(size)));
}

export function getTranslationStyleCacheStatus(
  project: SubtitleProject | null,
): Record<TranslationStyle, boolean> {
  if (!project || project.cues.length === 0) {
    return { natural: false, formal: false };
  }
  const activeStyle = normalizeTranslationStyle(
    project.completedTranslationStyle ?? project.options.translationStyle,
  );
  let hasNatural = false;
  let hasFormal = false;
  for (const cue of project.cues) {
    const variants = cue.translatedVariants;
    if (variants && Object.keys(variants).length > 0) {
      if (variants.natural?.trim()) hasNatural = true;
      if (variants.formal?.trim()) hasFormal = true;
    } else if (cue.translatedText.trim().length > 0) {
      if (activeStyle === "formal") {
        hasFormal = true;
      } else {
        hasNatural = true;
      }
    }
    if (hasNatural && hasFormal) break;
  }
  return { natural: hasNatural, formal: hasFormal };
}

export function seedProjectTranslatedVariants(
  project: SubtitleProject,
): SubtitleProject {
  const seedStyle = normalizeTranslationStyle(
    project.completedTranslationStyle ?? project.options.translationStyle,
  );
  const normalizedOptions: JobOptions = {
    ...DEFAULT_JOB_OPTIONS,
    ...project.options,
    segmentRules: normalizeSegmentRules(project.options.segmentRules),
    correctSourceAsr: Boolean(project.options.correctSourceAsr),
    translationStyle: normalizeTranslationStyle(
      project.options.translationStyle,
    ),
    removeFillers: project.options.removeFillers ?? true,
  };

  let changed = false;
  const cues = project.cues.map((cue) => {
    const existing = cue.translatedVariants;
    const hasExistingVariants = Boolean(
      existing && Object.keys(existing).length > 0,
    );
    if (!hasExistingVariants && cue.translatedText.trim().length > 0) {
      changed = true;
      return {
        ...cue,
        translatedVariants: { [seedStyle]: cue.translatedText },
      };
    }
    return cue;
  });

  if (
    !changed &&
    project.options.removeFillers !== undefined &&
    project.options.translationStyle !== undefined &&
    project.topicContext !== undefined
  ) {
    return project;
  }

  return {
    ...project,
    options: normalizedOptions,
    cues,
    topicContext: project.topicContext ?? null,
    completedTranslationStyle: project.completedTranslationStyle
      ? normalizeTranslationStyle(project.completedTranslationStyle)
      : null,
  };
}

function isProcessingJobPhase(phase: JobPhase): boolean {
  return (
    phase === "convertingAudio" ||
    phase === "transcribing" ||
    phase === "segmenting" ||
    phase === "translating" ||
    phase === "polishing"
  );
}

function isCancellationError(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return /\bcancel(?:led|ed)\b/i.test(msg);
}

function isSubtitlePath(path: string): boolean {
  return /\.(srt|vtt)$/i.test(path.trim());
}

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
  editorSourceFontSize: EDITOR_SOURCE_FONT_DEFAULT,
  editorTranslatedFontSize: EDITOR_TRANSLATED_FONT_DEFAULT,
  autoProbeOnImport: true,
  autoCheckpoints: true,
  allowLocalEncryptedVault: false,
};

const DEFAULT_JOB_OPTIONS: JobOptions = {
  sourceLanguage: "auto",
  targetLanguage: "sr",
  script: "latin",
  segmentRules: DEFAULT_SEGMENT_RULES,
  glossary: {},
  polishSerbian: true,
  correctSourceAsr: false,
  translationStyle: "natural",
  removeFillers: true,
  providerOverride: null,
};

export interface AppStoreDeps {
  getSetupStatus: typeof getSetupStatus;
  getCatalog: typeof getCatalog;
  getConfig: typeof getConfig;
  saveConfig: typeof saveConfig;
  getApiKeyStatus: typeof getApiKeyStatus;
  setApiKey: typeof setApiKey;
  clearApiKey: typeof clearApiKey;
  listCheckpoints: typeof listCheckpoints;
  loadCheckpoint: typeof loadCheckpoint;
  saveProjectCheckpoint: typeof saveProjectCheckpoint;
  deleteCheckpoint: typeof deleteCheckpoint;
  transcribeAudio: typeof transcribeAudio;
  resegmentProject: typeof resegmentProject;
  translateProject: typeof translateProject;
  polishProject: typeof polishProject;
  condenseProjectCps: typeof condenseProjectCps;
  runFullPipeline: typeof runFullPipeline;
  runLocalLlmBatch: typeof runLocalLlmBatch;
  cancelJob: typeof cancelJob;
  importSubtitleFile: typeof importSubtitleFile;
  tryLoadCompanionSubtitles: typeof tryLoadCompanionSubtitles;
  convertCuesScript: typeof convertCuesScript;
  exportSubtitles: typeof exportSubtitles;
  installComponent: typeof installComponent;
  installCoreRuntime: typeof installCoreRuntime;
  installRecommendedBundle: typeof installRecommendedBundle;
  importCustomModel: typeof importCustomModel;
  deleteModel: typeof deleteModel;
  flushPendingCrashReports: typeof flushPendingCrashReports;
  probeMediaFile: typeof probeMediaFile;
  cancelMediaProbe: typeof cancelMediaProbe;
  onDownloadProgress: typeof onDownloadProgress;
  onJobProgress: typeof onJobProgress;
  onProbeProgress: typeof onProbeProgress;
  getAppVersion: () => Promise<string>;
  autosaveDebounceMs: number;
}

const DEFAULT_STORE_DEPS: AppStoreDeps = {
  getSetupStatus,
  getCatalog,
  getConfig,
  saveConfig,
  getApiKeyStatus,
  setApiKey,
  clearApiKey,
  listCheckpoints,
  loadCheckpoint,
  saveProjectCheckpoint,
  deleteCheckpoint,
  transcribeAudio,
  resegmentProject,
  translateProject,
  polishProject,
  condenseProjectCps,
  runFullPipeline,
  runLocalLlmBatch,
  cancelJob,
  importSubtitleFile,
  tryLoadCompanionSubtitles,
  convertCuesScript,
  exportSubtitles,
  installComponent,
  installCoreRuntime,
  installRecommendedBundle,
  importCustomModel,
  deleteModel,
  flushPendingCrashReports,
  probeMediaFile,
  cancelMediaProbe,
  onDownloadProgress,
  onJobProgress,
  onProbeProgress,
  getAppVersion: () =>
    isTauriRuntime()
      ? getVersion().catch(() => APP_VERSION)
      : Promise.resolve(APP_VERSION),
  autosaveDebounceMs: CHECKPOINT_AUTOSAVE_DEBOUNCE_MS,
};

export interface AppStoreState {
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
  queueCancelRequested: boolean;
  probeResults: Record<string, MediaProbeResult>;

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
  setEditorFontSize: (
    target: "source" | "translated",
    size: number,
  ) => Promise<void>;
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
  switchTranslationStyle: (style: TranslationStyle) => void;
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
  flushPendingCheckpoint: () => Promise<void>;

  closeOnlineConfirm: () => void;
  confirmOnlineAction: (rememberForSession: boolean) => void;

  startBatchPipeline: (mode: "full" | "transcribeOnly") => Promise<void>;
  stopProcessingQueue: () => Promise<void>;
  translateActiveProject: () => Promise<void>;
  polishActiveProject: () => Promise<void>;
  condenseActiveProjectCps: () => Promise<void>;
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

export function createAppStore(depsOverrides?: Partial<AppStoreDeps>) {
  const deps: AppStoreDeps = { ...DEFAULT_STORE_DEPS, ...depsOverrides };
  let listenersRegistered = false;
  let autosaveTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingAutosaveProject: SubtitleProject | null = null;
  const probePendingQueue: Array<{ jobId: string; audioPath: string }> = [];
  let probeRunnerActive = false;
  let activeProbeJobId: string | null = null;

  return create<AppStoreState>((set, get) => {
  const withOnlineGuard = (
    requiresLlm: boolean,
    actionLabel: string,
    action: () => Promise<void>,
  ): Promise<void> => {
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
      return Promise.resolve();
    }

    return action();
  };

  const normalizeLoadedConfig = (
    loaded: AppConfig,
    fallbackSplitOnSpeakerChange?: boolean,
  ): AppConfig => ({
    ...DEFAULT_CONFIG,
    ...loaded,
    defaultSegmentRules: normalizeSegmentRules({
      ...loaded.defaultSegmentRules,
      splitOnSpeakerChange:
        loaded.defaultSegmentRules?.splitOnSpeakerChange ??
        fallbackSplitOnSpeakerChange ??
        true,
    }),
    autoSendCrashReports: loaded.autoSendCrashReports ?? true,
    editorSourceFontSize: clampEditorFontSize(
      "source",
      loaded.editorSourceFontSize ?? EDITOR_SOURCE_FONT_DEFAULT,
    ),
    editorTranslatedFontSize: clampEditorFontSize(
      "translated",
      loaded.editorTranslatedFontSize ?? EDITOR_TRANSLATED_FONT_DEFAULT,
    ),
    autoProbeOnImport: loaded.autoProbeOnImport ?? true,
    autoCheckpoints: loaded.autoCheckpoints ?? true,
    allowLocalEncryptedVault: loaded.allowLocalEncryptedVault ?? false,
    uiLanguage: normalizeUiLanguage(loaded.uiLanguage),
  });

  const flushCheckpointAutosave = async (): Promise<void> => {
    if (autosaveTimer !== null) {
      clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
    const projectToSave = pendingAutosaveProject;
    pendingAutosaveProject = null;
    if (!projectToSave || get().config.autoCheckpoints === false) {
      return;
    }
    try {
      await deps.saveProjectCheckpoint(projectToSave);
    } catch {
      // Non-fatal background checkpoint error
    }
  };

  const scheduleCheckpointAutosave = (project: SubtitleProject) => {
    if (get().config.autoCheckpoints === false || !project.jobId.trim()) {
      if (autosaveTimer !== null) {
        clearTimeout(autosaveTimer);
        autosaveTimer = null;
      }
      pendingAutosaveProject = null;
      return;
    }
    if (
      pendingAutosaveProject &&
      pendingAutosaveProject.jobId !== project.jobId
    ) {
      void flushCheckpointAutosave();
    }
    pendingAutosaveProject = project;
    if (autosaveTimer !== null) {
      clearTimeout(autosaveTimer);
    }
    autosaveTimer = setTimeout(() => {
      void flushCheckpointAutosave();
    }, deps.autosaveDebounceMs);
  };

  const applyProbeResultToState = (evt: MediaProbeResult) => {
    set((state) => {
      const nextProbeResults = {
        ...state.probeResults,
        [evt.jobId]: evt,
      };
      const topic = evt.topic?.trim() || null;
      const updateProjectMeta = (
        proj: SubtitleProject | null,
      ): SubtitleProject | null => {
        if (!proj || proj.jobId !== evt.jobId) return proj;
        const nextDuration =
          proj.durationMs > 0 ? proj.durationMs : (evt.durationMs ?? 0);
        const nextLang = proj.detectedLanguage ?? evt.detectedLanguage ?? null;
        const nextTopic = topic ?? proj.topicContext ?? null;
        if (
          nextDuration === proj.durationMs &&
          nextLang === proj.detectedLanguage &&
          nextTopic === (proj.topicContext ?? null)
        ) {
          return proj;
        }
        return {
          ...proj,
          durationMs: nextDuration,
          detectedLanguage: nextLang,
          topicContext: nextTopic,
        };
      };

      const nextQueue = state.batchQueue.map((item) => {
        if (item.jobId !== evt.jobId) return item;
        const nextProj = updateProjectMeta(item.project);
        return nextProj === item.project ? item : { ...item, project: nextProj };
      });

      return {
        probeResults: nextProbeResults,
        batchQueue: nextQueue,
        activeProject: updateProjectMeta(state.activeProject),
      };
    });
  };

  const drainProbeQueue = async () => {
    if (probeRunnerActive) {
      return;
    }
    probeRunnerActive = true;
    try {
      while (probePendingQueue.length > 0) {
        if (get().isRunningBatch || get().config.autoProbeOnImport === false) {
          break;
        }
        const next = probePendingQueue.shift();
        if (!next) break;
        const queueItem = get().batchQueue.find(
          (q) => q.jobId === next.jobId,
        );
        if (!queueItem || queueItem.phase !== "queued") {
          continue;
        }
        if (isSubtitlePath(next.audioPath)) {
          continue;
        }
        activeProbeJobId = next.jobId;
        try {
          const result = await deps.probeMediaFile(next.jobId, next.audioPath);
          applyProbeResultToState(result);
          if (result.stage === "cancelled" || get().isRunningBatch) {
            break;
          }
        } catch (err) {
          if (get().isRunningBatch) {
            break;
          }
          applyProbeResultToState({
            jobId: next.jobId,
            audioPath: next.audioPath,
            stage: "failed",
            durationMs: null,
            speechRatio: null,
            estimatedSpeakers: null,
            detectedLanguage: null,
            topic: null,
            suggestedStyle: null,
            error: err instanceof Error ? err.message : String(err),
          });
        } finally {
          if (activeProbeJobId === next.jobId) {
            activeProbeJobId = null;
          }
        }
      }
    } finally {
      activeProbeJobId = null;
      probeRunnerActive = false;
    }
  };

  const preemptActiveProbe = async () => {
    probePendingQueue.length = 0;
    try {
      await deps.cancelMediaProbe(null);
    } catch {
      // Ignore probe cancel errors when no probe is running
    }
  };

  const findCueIdAtPlayheadZero = (cues: SubtitleCue[]): number | null => {
    const match = cues.find((c) => c.startMs <= 0 && c.endMs >= 0);
    return match ? match.id : null;
  };

  const syncProjectIntoQueueAndState = (
    rawProject: SubtitleProject,
    options?: { forceActivate?: boolean },
  ) => {
    const project = seedProjectTranslatedVariants(rawProject);
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
    cues.map((c) => ({
      ...c,
      warnings: [...c.warnings],
      translatedVariants: c.translatedVariants
        ? { ...c.translatedVariants }
        : undefined,
    }));

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
    queueCancelRequested: false,
    probeResults: {},

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
    appVersion: APP_VERSION,
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
        void deps.onDownloadProgress((evt) => {
          set((state) => ({
            downloadProgress: {
              ...state.downloadProgress,
              [evt.itemId]: evt,
            },
          }));
        });

        void deps.onJobProgress((evt) => {
          set((state) => {
            const queueItem = state.batchQueue.find(
              (item) => item.jobId === evt.jobId,
            );
            if (
              queueItem?.phase === "cancelled" &&
              state.queueCancelRequested &&
              evt.phase !== "cancelled"
            ) {
              return state;
            }
            return {
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
            };
          });
        });

        void deps.onProbeProgress((evt) => {
          applyProbeResultToState(evt);
        });

        if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
          window.addEventListener("beforeunload", () => {
            void flushCheckpointAutosave();
          });
        }
      }

      try {
        const [
          setupStatus,
          catalog,
          loadedConfig,
          apiKeyStatus,
          rawCheckpoints,
          appVersion,
        ] = await Promise.all([
          deps.getSetupStatus(),
          deps.getCatalog(),
          deps.getConfig(),
          deps.getApiKeyStatus(),
          deps.listCheckpoints(),
          deps.getAppVersion(),
        ]);

        const config = normalizeLoadedConfig(loadedConfig);
        const checkpoints = rawCheckpoints.map(seedProjectTranslatedVariants);

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
          void deps.flushPendingCrashReports().catch(() => {});
        }

        // Background GitHub latest-release version check (silent, non-blocking)
        if (typeof fetch === "function" && typeof window !== "undefined") {
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
        }
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
        const saved = await deps.saveConfig(nextConfig);
        set({
          config: normalizeLoadedConfig(
            saved,
            nextConfig.defaultSegmentRules.splitOnSpeakerChange,
          ),
        });
      } catch {
        // Keep optimistic UI language
      }
    },

    updateConfig: async (patch) => {
      const nextConfig: AppConfig = {
        ...get().config,
        ...patch,
        defaultSegmentRules: normalizeSegmentRules(
          patch.defaultSegmentRules ?? get().config.defaultSegmentRules,
        ),
        editorSourceFontSize: clampEditorFontSize(
          "source",
          patch.editorSourceFontSize ??
            get().config.editorSourceFontSize ??
            EDITOR_SOURCE_FONT_DEFAULT,
        ),
        editorTranslatedFontSize: clampEditorFontSize(
          "translated",
          patch.editorTranslatedFontSize ??
            get().config.editorTranslatedFontSize ??
            EDITOR_TRANSLATED_FONT_DEFAULT,
        ),
      };
      set({ config: nextConfig });
      try {
        const saved = await deps.saveConfig(nextConfig);
        const normalized = normalizeLoadedConfig(
          saved,
          nextConfig.defaultSegmentRules.splitOnSpeakerChange,
        );
        if (patch.allowLocalEncryptedVault !== undefined) {
          const apiKeyStatus = await deps
            .getApiKeyStatus()
            .catch(() => get().apiKeyStatus);
          set({ config: normalized, apiKeyStatus });
        } else {
          set({ config: normalized });
        }
      } catch (err) {
        set({
          banner: {
            type: "error",
            message: err instanceof Error ? err.message : String(err),
          },
        });
      }
    },

    setEditorFontSize: async (target, size) => {
      const clamped = clampEditorFontSize(target, size);
      if (target === "source") {
        await get().updateConfig({ editorSourceFontSize: clamped });
      } else {
        await get().updateConfig({ editorTranslatedFontSize: clamped });
      }
    },

    refreshSetupAndCatalog: async () => {
      try {
        const [setupStatus, catalog, config, apiKeyStatus] = await Promise.all([
          deps.getSetupStatus(),
          deps.getCatalog(),
          deps.getConfig(),
          deps.getApiKeyStatus(),
        ]);
        set({
          setupStatus,
          catalog,
          config: normalizeLoadedConfig(
            config,
            get().config.defaultSegmentRules.splitOnSpeakerChange,
          ),
          apiKeyStatus,
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
        await deps.installComponent(itemId);
        const [setupStatus, config] = await Promise.all([
          deps.getSetupStatus(),
          deps.getConfig(),
        ]);
        set({
          setupStatus,
          config: normalizeLoadedConfig(
            config,
            get().config.defaultSegmentRules.splitOnSpeakerChange,
          ),
        });
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
        const setupStatus = await deps.installCoreRuntime();
        const config = normalizeLoadedConfig(
          await deps.getConfig(),
          get().config.defaultSegmentRules.splitOnSpeakerChange,
        );
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
        const setupStatus = await deps.installRecommendedBundle();
        const prevSplit = get().config.defaultSegmentRules.splitOnSpeakerChange;
        const freshConfig = await deps.getConfig();
        let nextConfig = normalizeLoadedConfig(freshConfig, prevSplit);
        if (!nextConfig.hasCompletedSetup) {
          nextConfig = normalizeLoadedConfig(
            await deps.saveConfig({
              ...nextConfig,
              hasCompletedSetup: true,
            }),
            prevSplit,
          );
        }
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
        await deps.importCustomModel(sourcePath, kind, name);
        const [rawConfig, setupStatus] = await Promise.all([
          deps.getConfig(),
          deps.getSetupStatus(),
        ]);
        const config = normalizeLoadedConfig(
          rawConfig,
          get().config.defaultSegmentRules.splitOnSpeakerChange,
        );
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
        await deps.deleteModel(modelId);
        const [config, setupStatus] = await Promise.all([
          deps.getConfig(),
          deps.getSetupStatus(),
        ]);
        set({
          config: normalizeLoadedConfig(
            config,
            get().config.defaultSegmentRules.splitOnSpeakerChange,
          ),
          setupStatus,
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

    runSetApiKey: async (provider, apiKey) => {
      try {
        const apiKeyStatus = await deps.setApiKey(provider, apiKey);
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
        const apiKeyStatus = await deps.clearApiKey(provider);
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

    switchTranslationStyle: (style) => {
      const nextStyle = normalizeTranslationStyle(style);
      set((state) => ({
        jobOptions: {
          ...state.jobOptions,
          translationStyle: nextStyle,
        },
      }));

      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }

      const seededProject = seedProjectTranslatedVariants(activeProject);
      const prevStyle = normalizeTranslationStyle(
        seededProject.options.translationStyle,
      );
      const rules =
        seededProject.options.segmentRules ?? jobOptions.segmentRules;

      const preparedCues = seededProject.cues.map((cue) => {
        const existing = cue.translatedVariants;
        if (
          (!existing || Object.keys(existing).length === 0) &&
          cue.translatedText.trim().length > 0
        ) {
          return {
            ...cue,
            translatedVariants: { [prevStyle]: cue.translatedText },
          };
        }
        return cue;
      });

      const hasCachedTarget = preparedCues.some((cue) =>
        Boolean(cue.translatedVariants?.[nextStyle]?.trim()),
      );

      const nextCues = hasCachedTarget
        ? preparedCues.map((cue) => {
            const cachedText = cue.translatedVariants?.[nextStyle] ?? "";
            return recalculateCue(
              {
                ...cue,
                translatedText: cachedText,
              },
              rules,
            );
          })
        : preparedCues;

      const updatedProject: SubtitleProject = {
        ...seededProject,
        options: {
          ...seededProject.options,
          translationStyle: nextStyle,
        },
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      };

      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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
      set((state) => {
        const hasNumericRuleChange =
          patch.maxCharsPerLine !== undefined ||
          patch.maxLines !== undefined ||
          patch.minDurationMs !== undefined ||
          patch.maxDurationMs !== undefined ||
          patch.maxCps !== undefined;
        return {
          jobOptions: {
            ...state.jobOptions,
            segmentRules: {
              ...state.jobOptions.segmentRules,
              ...patch,
              preset:
                patch.preset ??
                (hasNumericRuleChange
                  ? "custom"
                  : state.jobOptions.segmentRules.preset),
            },
          },
        };
      });
    },

    applyResegmentToActiveProject: async () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return;
      }
      pushUndoSnapshot(activeProject.cues);
      try {
        const updated = await deps.resegmentProject(
          activeProject,
          jobOptions.segmentRules,
        );
        syncProjectIntoQueueAndState(updated);
        scheduleCheckpointAutosave(updated);
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
        const fallbackProject: SubtitleProject = {
          ...activeProject,
          options: {
            ...activeProject.options,
            segmentRules: jobOptions.segmentRules,
          },
          cues: updatedCues,
          updatedAt: new Date().toISOString(),
        };
        syncProjectIntoQueueAndState(fallbackProject);
        scheduleCheckpointAutosave(fallbackProject);
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
        const convertedCues = await deps.convertCuesScript(
          activeProject.cues,
          targetScript,
        );
        const recalculated = convertedCues.map((c, idx) => {
          const origCue = activeProject.cues[idx];
          let nextVariants = c.translatedVariants;
          if (
            nextVariants &&
            origCue &&
            nextVariants === origCue.translatedVariants
          ) {
            const convertedMap: Partial<Record<TranslationStyle, string>> = {};
            for (const style of ["natural", "formal"] as const) {
              const val = nextVariants[style];
              if (typeof val === "string") {
                convertedMap[style] = convertSerbianScriptText(
                  val,
                  targetScript,
                );
              }
            }
            nextVariants = convertedMap;
          }
          return recalculateCue(
            { ...c, translatedVariants: nextVariants },
            jobOptions.segmentRules,
          );
        });
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
        scheduleCheckpointAutosave(updated);
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
      await flushCheckpointAutosave();
      try {
        const project = seedProjectTranslatedVariants(
          await deps.importSubtitleFile(path, get().jobOptions),
        );
        resetUndoHistory();
        syncProjectIntoQueueAndState(project, { forceActivate: true });
        scheduleCheckpointAutosave(project);
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
      void flushCheckpointAutosave();
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

      const project: SubtitleProject = seedProjectTranslatedVariants({
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
        completedTranslationStyle: null,
        topicContext: null,
        errorMessage: null,
        updatedAt: new Date().toISOString(),
      });

      resetUndoHistory();
      syncProjectIntoQueueAndState(project, { forceActivate: true });
      scheduleCheckpointAutosave(project);
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
              completedTranslationStyle: null,
              topicContext: null,
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
              completedTranslationStyle: null,
              topicContext: null,
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
        void deps
          .tryLoadCompanionSubtitles(item.audioPath, optionsSnapshot)
          .then((companionProject) => {
            if (!companionProject) {
              return;
            }
            const mergedProject: SubtitleProject = seedProjectTranslatedVariants(
              {
                ...companionProject,
                jobId: item.jobId,
                audioPath: item.audioPath,
                audioFileName: item.audioFileName,
                phase: "completed",
              },
            );
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
            scheduleCheckpointAutosave(mergedProject);
          })
          .catch(() => {
            // Keep file queued cleanly if companion subtitle fails to parse
          });
      }

      // Workstream 5: Schedule sequential background media intake probes when idle
      if (
        newlyAddedItems.length > 0 &&
        get().config.autoProbeOnImport !== false &&
        !get().isRunningBatch
      ) {
        for (const item of newlyAddedItems) {
          if (!isSubtitlePath(item.audioPath)) {
            probePendingQueue.push({
              jobId: item.jobId,
              audioPath: item.audioPath,
            });
          }
        }
        void drainProbeQueue();
      }
    },

    removeQueueItem: (jobId) => {
      if (pendingAutosaveProject?.jobId === jobId) {
        if (autosaveTimer !== null) {
          clearTimeout(autosaveTimer);
          autosaveTimer = null;
        }
        pendingAutosaveProject = null;
      } else if (get().activeJobId === jobId) {
        void flushCheckpointAutosave();
      }
      const probeIdx = probePendingQueue.findIndex((p) => p.jobId === jobId);
      if (probeIdx !== -1) {
        probePendingQueue.splice(probeIdx, 1);
      }
      if (activeProbeJobId === jobId) {
        void deps.cancelMediaProbe(jobId).catch(() => false);
      }
      set((state) => {
        const nextQueue = state.batchQueue.filter(
          (item) => item.jobId !== jobId,
        );
        const isRemovingActive = state.activeJobId === jobId;
        const fallbackItem = nextQueue[0] ?? null;
        const nextProbes = { ...state.probeResults };
        delete nextProbes[jobId];
        return {
          batchQueue: nextQueue,
          probeResults: nextProbes,
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
        void flushCheckpointAutosave();
        resetUndoHistory();
      }
      const probe = get().probeResults[jobId];
      const placeholderProject: SubtitleProject | null = item.project
        ? item.project
        : {
            jobId: item.jobId,
            audioPath: item.audioPath,
            audioFileName: item.audioFileName,
            durationMs: probe?.durationMs ?? 0,
            detectedLanguage: probe?.detectedLanguage ?? null,
            options: { ...get().jobOptions },
            words: [],
            cues: [],
            phase: item.phase,
            completedTranslationBatches: 0,
            completedTranslationStyle: null,
            topicContext: probe?.topic ?? null,
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
      await flushCheckpointAutosave();
      try {
        const project = seedProjectTranslatedVariants(
          await deps.loadCheckpoint(jobId),
        );
        resetUndoHistory();
        syncProjectIntoQueueAndState(project, { forceActivate: true });
        set({
          jobOptions: {
            ...DEFAULT_JOB_OPTIONS,
            ...project.options,
            segmentRules: normalizeSegmentRules(project.options.segmentRules),
            correctSourceAsr: Boolean(project.options.correctSourceAsr),
            translationStyle: normalizeTranslationStyle(
              project.options.translationStyle,
            ),
            removeFillers: project.options.removeFillers ?? true,
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
        await deps.deleteCheckpoint(jobId);
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

    flushPendingCheckpoint: () => flushCheckpointAutosave(),

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

      return withOnlineGuard(
        requiresLlm,
        mode === "full" ? "1-Click Pipeline" : "Transcribe",
        async () => {
          await flushCheckpointAutosave();
          await preemptActiveProbe();

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
            queueCancelRequested: false,
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

          const attachTopicContext = (
            project: SubtitleProject,
            jobId: string,
          ): SubtitleProject => {
            const topic =
              project.topicContext ?? get().probeResults[jobId]?.topic ?? null;
            return topic ? { ...project, topicContext: topic } : project;
          };

          const handleJobError = async (
            jobId: string,
            audioFileName: string,
            err: unknown,
            fallbackProject?: SubtitleProject,
          ) => {
            const currentQueueItem = get().batchQueue.find(
              (q) => q.jobId === jobId,
            );
            const wasCancelled =
              get().queueCancelRequested ||
              currentQueueItem?.phase === "cancelled" ||
              isCancellationError(err);

            if (wasCancelled) {
              const recovered =
                fallbackProject ??
                (await deps.loadCheckpoint(jobId).catch(() => null));
              if (recovered && recovered.cues.length > 0) {
                syncProjectIntoQueueAndState({
                  ...recovered,
                  phase: "cancelled",
                  errorMessage: null,
                });
              }
              set((s) => ({
                batchQueue: s.batchQueue.map((q) =>
                  q.jobId === jobId
                    ? {
                        ...q,
                        phase: "cancelled",
                        detail: "Cancelled",
                        errorMessage: null,
                      }
                    : q,
                ),
                activeProject:
                  s.activeProject && s.activeProject.jobId === jobId
                    ? {
                        ...s.activeProject,
                        phase: "cancelled",
                        errorMessage: null,
                      }
                    : s.activeProject,
              }));
              return;
            }

            const msg = err instanceof Error ? err.message : String(err);
            const recovered =
              fallbackProject ??
              (await deps.loadCheckpoint(jobId).catch(() => null));
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
              if (get().queueCancelRequested) {
                break;
              }
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
                const transcribed = attachTopicContext(
                  await deps.transcribeAudio(
                    item.audioPath,
                    currentOptions,
                    item.jobId,
                  ),
                  item.jobId,
                );
                if (get().queueCancelRequested) {
                  await handleJobError(
                    item.jobId,
                    item.audioFileName,
                    new Error("Job cancelled"),
                    transcribed,
                  );
                  break;
                }
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
                          ? await deps.translateProject(projectForLlm)
                          : await deps.polishProject(projectForLlm);
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
                if (get().queueCancelRequested) {
                  break;
                }
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
              if (get().queueCancelRequested) {
                break;
              }
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
                const transcribed = attachTopicContext(
                  await deps.transcribeAudio(
                    item.audioPath,
                    currentOptions,
                    item.jobId,
                  ),
                  item.jobId,
                );
                if (get().queueCancelRequested) {
                  await handleJobError(
                    item.jobId,
                    item.audioFileName,
                    new Error("Job cancelled"),
                    transcribed,
                  );
                  break;
                }
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
                if (get().queueCancelRequested) {
                  break;
                }
              }
            }

            const projectsForLlm: SubtitleProject[] = [];
            const llmStageByJobId = new Map<
              string,
              "translating" | "polishing"
            >();

            if (!get().queueCancelRequested) {
              for (const {
                project,
                options: itemOptions,
              } of transcribedEntries) {
                const stagePhase = classifyLlmStage(project, itemOptions);
                if (stagePhase) {
                  llmStageByJobId.set(project.jobId, stagePhase);
                  projectsForLlm.push({
                    ...project,
                    phase: stagePhase,
                  });
                }
              }
            }

            if (!get().queueCancelRequested && projectsForLlm.length > 0) {
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
                const finishedProjects =
                  await deps.runLocalLlmBatch(projectsForLlm);
                for (const proj of finishedProjects) {
                  syncProjectIntoQueueAndState(proj);
                }
              } catch (err) {
                for (const proj of projectsForLlm) {
                  const recovered = await deps
                    .loadCheckpoint(proj.jobId)
                    .catch(() => null);
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
              if (get().queueCancelRequested) {
                break;
              }
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

              const itemTopic =
                get().probeResults[item.jobId]?.topic?.trim() ||
                item.project?.topicContext?.trim() ||
                null;

              try {
                if (mode === "full" && itemTopic) {
                  // Single-file 1-click run with probed/existing topic context:
                  // transcribe first so `topicContext` is attached on the SubtitleProject
                  // before invoking LLM translation/polishing (Workstream 5).
                  const transcribed = attachTopicContext(
                    await deps.transcribeAudio(
                      item.audioPath,
                      currentOptions,
                      item.jobId,
                    ),
                    item.jobId,
                  );
                  if (get().queueCancelRequested) {
                    await handleJobError(
                      item.jobId,
                      item.audioFileName,
                      new Error("Job cancelled"),
                      transcribed,
                    );
                    break;
                  }
                  const normalizedTranscribed: SubtitleProject = {
                    ...transcribed,
                    options: {
                      ...currentOptions,
                      segmentRules: transcribed.options.segmentRules,
                    },
                  };
                  syncProjectIntoQueueAndState(normalizedTranscribed);

                  const stagePhase = classifyLlmStage(
                    normalizedTranscribed,
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
                      ...normalizedTranscribed,
                      phase: stagePhase,
                    };
                    const finished =
                      stagePhase === "translating"
                        ? await deps.translateProject(projectForLlm)
                        : await deps.polishProject(projectForLlm);
                    if (get().queueCancelRequested) {
                      await handleJobError(
                        item.jobId,
                        item.audioFileName,
                        new Error("Job cancelled"),
                        finished,
                      );
                      break;
                    }
                    syncProjectIntoQueueAndState(finished);
                  }
                } else {
                  const project = attachTopicContext(
                    mode === "full"
                      ? await deps.runFullPipeline(
                          item.audioPath,
                          currentOptions,
                          item.jobId,
                        )
                      : await deps.transcribeAudio(
                          item.audioPath,
                          currentOptions,
                          item.jobId,
                        ),
                    item.jobId,
                  );
                  if (get().queueCancelRequested) {
                    await handleJobError(
                      item.jobId,
                      item.audioFileName,
                      new Error("Job cancelled"),
                      project,
                    );
                    break;
                  }
                  syncProjectIntoQueueAndState(project);
                }
              } catch (err) {
                await handleJobError(item.jobId, item.audioFileName, err);
                if (get().queueCancelRequested) {
                  break;
                }
              }
            }
          }

          const rawCheckpoints = await deps
            .listCheckpoints()
            .catch(() => get().checkpoints);
          set({
            isRunningBatch: false,
            queueCancelRequested: false,
            checkpoints: rawCheckpoints.map(seedProjectTranslatedVariants),
          });
        },
      );
    },

    stopProcessingQueue: async () => {
      set({ queueCancelRequested: true });
      const state = get();
      const processingIds = state.batchQueue
        .filter((item) => isProcessingJobPhase(item.phase))
        .map((item) => item.jobId);
      const targetIds =
        processingIds.length > 0
          ? processingIds
          : state.activeJobId
            ? [state.activeJobId]
            : [];
      for (const jobId of targetIds) {
        await get().cancelJobById(jobId);
      }
      await preemptActiveProbe();
    },

    translateActiveProject: () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return Promise.resolve();
      }
      return withOnlineGuard(true, "Translate Project", async () => {
        set({ queueCancelRequested: false });
        await flushCheckpointAutosave();
        await preemptActiveProbe();
        const latestProject = get().activeProject ?? activeProject;
        const targetStyle = normalizeTranslationStyle(
          jobOptions.translationStyle ?? latestProject.options.translationStyle,
        );
        const topicContext =
          latestProject.topicContext ??
          get().probeResults[latestProject.jobId]?.topic ??
          null;
        const isSwitchingStyle =
          latestProject.completedTranslationStyle !== targetStyle;
        const projectWithUpdatedOptions: SubtitleProject = {
          ...latestProject,
          options: {
            ...latestProject.options,
            ...jobOptions,
            translationStyle: targetStyle,
            targetLanguage:
              jobOptions.targetLanguage === "none"
                ? "sr"
                : jobOptions.targetLanguage,
          },
          completedTranslationBatches: isSwitchingStyle
            ? 0
            : latestProject.completedTranslationBatches,
          topicContext,
          phase: "translating",
        };
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const translated = await deps.translateProject(
            projectWithUpdatedOptions,
          );
          syncProjectIntoQueueAndState(translated);
        } catch (err) {
          const cancelled =
            get().queueCancelRequested || isCancellationError(err);
          const msg = err instanceof Error ? err.message : String(err);
          syncProjectIntoQueueAndState({
            ...projectWithUpdatedOptions,
            phase: cancelled ? "cancelled" : "failed",
            errorMessage: cancelled ? null : msg,
          });
          if (!cancelled) {
            set({
              banner: {
                type: "error",
                message: msg,
              },
            });
          }
        } finally {
          set({ queueCancelRequested: false });
        }
      });
    },

    polishActiveProject: () => {
      const { activeProject, jobOptions } = get();
      if (!activeProject) {
        return Promise.resolve();
      }
      return withOnlineGuard(true, "Polish Serbian Transcript", async () => {
        set({ queueCancelRequested: false });
        await flushCheckpointAutosave();
        await preemptActiveProbe();
        const latestProject = get().activeProject ?? activeProject;
        const topicContext =
          latestProject.topicContext ??
          get().probeResults[latestProject.jobId]?.topic ??
          null;
        const projectWithUpdatedOptions: SubtitleProject = {
          ...latestProject,
          options: {
            ...latestProject.options,
            ...jobOptions,
            polishSerbian: true,
          },
          topicContext,
          phase: "polishing",
        };
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const polished = await deps.polishProject(projectWithUpdatedOptions);
          syncProjectIntoQueueAndState(polished);
        } catch (err) {
          const cancelled =
            get().queueCancelRequested || isCancellationError(err);
          const msg = err instanceof Error ? err.message : String(err);
          syncProjectIntoQueueAndState({
            ...projectWithUpdatedOptions,
            phase: cancelled ? "cancelled" : "failed",
            errorMessage: cancelled ? null : msg,
          });
          if (!cancelled) {
            set({
              banner: {
                type: "error",
                message: msg,
              },
            });
          }
        } finally {
          set({ queueCancelRequested: false });
        }
      });
    },

    condenseActiveProjectCps: () => {
      const { activeProject, jobOptions, isRunningBatch } = get();
      if (!activeProject || activeProject.cues.length === 0 || isRunningBatch) {
        return Promise.resolve();
      }
      return withOnlineGuard(true, "Auto-Fit CPS", async () => {
        set({ queueCancelRequested: false });
        await flushCheckpointAutosave();
        await preemptActiveProbe();
        const latestProject = get().activeProject ?? activeProject;
        pushUndoSnapshot(latestProject.cues);
        const projectWithUpdatedOptions: SubtitleProject = {
          ...latestProject,
          options: {
            ...latestProject.options,
            ...jobOptions,
          },
          phase: "polishing",
        };
        set({ isRunningBatch: true });
        syncProjectIntoQueueAndState(projectWithUpdatedOptions);
        try {
          const condensed = await deps.condenseProjectCps(
            projectWithUpdatedOptions,
          );
          syncProjectIntoQueueAndState(condensed);
          const rawCheckpoints = await deps
            .listCheckpoints()
            .catch(() => get().checkpoints);
          set({
            checkpoints: rawCheckpoints.map(seedProjectTranslatedVariants),
            banner: {
              type: "success",
              message:
                get().config.uiLanguage === "en"
                  ? "High-CPS cues automatically condensed."
                  : "Predugi titlovi su automatski sažeti (Auto-Fit CPS).",
            },
          });
        } catch (err) {
          const cancelled =
            get().queueCancelRequested || isCancellationError(err);
          const msg = err instanceof Error ? err.message : String(err);
          const recovered = await deps
            .loadCheckpoint(latestProject.jobId)
            .catch(() => null);
          if (recovered && recovered.cues.length > 0) {
            syncProjectIntoQueueAndState({
              ...recovered,
              phase: cancelled ? "cancelled" : "failed",
              errorMessage: cancelled ? null : msg,
            });
          } else {
            syncProjectIntoQueueAndState({
              ...projectWithUpdatedOptions,
              phase: cancelled ? "cancelled" : "failed",
              errorMessage: cancelled ? null : msg,
            });
          }
          if (!cancelled) {
            set({
              banner: {
                type: "error",
                message: msg,
              },
            });
          }
        } finally {
          set({ isRunningBatch: false, queueCancelRequested: false });
        }
      });
    },

    cancelJobById: async (jobId) => {
      try {
        await deps.cancelJob(jobId);
      } finally {
        set((state) => ({
          batchQueue: state.batchQueue.map((item) =>
            item.jobId === jobId
              ? {
                  ...item,
                  phase: "cancelled",
                  detail: "Cancelled",
                  errorMessage: null,
                }
              : item,
          ),
          activeProject:
            state.activeProject && state.activeProject.jobId === jobId
              ? {
                  ...state.activeProject,
                  phase: "cancelled",
                  errorMessage: null,
                }
              : state.activeProject,
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
      const activeStyle = normalizeTranslationStyle(
        activeProject.options.translationStyle ?? jobOptions.translationStyle,
      );
      const updatedCues = activeProject.cues.map((cue) => {
        if (cue.id !== cueId) {
          return cue;
        }
        const merged: SubtitleCue = {
          ...cue,
          ...patch,
        };
        if (patch.translatedText !== undefined) {
          const nextVariants: Partial<Record<TranslationStyle, string>> = {
            ...(cue.translatedVariants ?? {}),
          };
          if (patch.translatedText.trim().length > 0) {
            nextVariants[activeStyle] = patch.translatedText;
          } else {
            delete nextVariants[activeStyle];
          }
          merged.translatedVariants =
            Object.keys(nextVariants).length > 0 ? nextVariants : undefined;
        }
        if (merged.endMs <= merged.startMs) {
          merged.endMs = merged.startMs + 500;
        }
        return recalculateCue(merged, rules);
      });

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: updatedCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const { leftEndMs, rightStartMs } = computeSplitBoundaries(
        cue.startMs,
        cue.endMs,
        cutMs,
      );
      if (leftEndMs <= cue.startMs || rightStartMs >= cue.endMs) {
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

      const variants1: Partial<Record<TranslationStyle, string>> = {};
      const variants2: Partial<Record<TranslationStyle, string>> = {};
      if (cue.translatedVariants) {
        for (const style of ["natural", "formal"] as const) {
          const variantText = cue.translatedVariants[style];
          if (variantText && variantText.trim().length > 0) {
            const [v1, v2] = splitWords(variantText, false);
            if (v1) variants1[style] = v1;
            if (v2) variants2[style] = v2;
          }
        }
      }

      const firstHalf = recalculateCue(
        {
          ...cue,
          endMs: leftEndMs,
          sourceText: src1,
          translatedText: tr1,
          translatedVariants:
            Object.keys(variants1).length > 0 ? variants1 : undefined,
        },
        jobOptions.segmentRules,
      );

      const secondHalf = recalculateCue(
        {
          ...cue,
          startMs: rightStartMs,
          sourceText: src2,
          translatedText: tr2,
          translatedVariants:
            Object.keys(variants2).length > 0 ? variants2 : undefined,
        },
        jobOptions.segmentRules,
      );

      const nextCues = [
        ...activeProject.cues.slice(0, idx),
        firstHalf,
        secondHalf,
        ...activeProject.cues.slice(idx + 1),
      ].map((item, i) => ({ ...item, id: i + 1 }));

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const mergedVariants: Partial<Record<TranslationStyle, string>> = {};
      for (const style of ["natural", "formal"] as const) {
        const a = current.translatedVariants?.[style]?.trim() ?? "";
        const b = next.translatedVariants?.[style]?.trim() ?? "";
        const joined = [a, b].filter(Boolean).join("\n");
        if (joined) {
          mergedVariants[style] = joined;
        }
      }

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
          translatedVariants:
            Object.keys(mergedVariants).length > 0 ? mergedVariants : undefined,
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

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: nextCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: [...activeProject.cues, newCue],
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const activeStyle = normalizeTranslationStyle(
        activeProject.options.translationStyle ?? jobOptions.translationStyle,
      );
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
          let translatedChanged = false;

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
              translatedChanged = true;
            }
          }

          if (!cueChanged) {
            return cue;
          }

          let nextVariants = cue.translatedVariants;
          if (translatedChanged) {
            const copy: Partial<Record<TranslationStyle, string>> = {
              ...(cue.translatedVariants ?? {}),
            };
            if (nextTranslated.trim().length > 0) {
              copy[activeStyle] = nextTranslated;
            } else {
              delete copy[activeStyle];
            }
            nextVariants = Object.keys(copy).length > 0 ? copy : undefined;
          }

          return recalculateCue(
            {
              ...cue,
              sourceText: nextSource,
              translatedText: nextTranslated,
              translatedVariants: nextVariants,
            },
            jobOptions.segmentRules,
          );
        });

        if (totalReplaced > 0) {
          pushUndoSnapshot(activeProject.cues);
          const updatedProject: SubtitleProject = {
            ...activeProject,
            cues: nextCues,
            updatedAt: new Date().toISOString(),
          };
          syncProjectIntoQueueAndState(updatedProject);
          scheduleCheckpointAutosave(updatedProject);
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
        const nextTranslated =
          !sourceMatches && translatedMatches
            ? cue.translatedText.replace(singleRegex, () => replaceText)
            : cue.translatedText;
        let nextVariants = cue.translatedVariants;
        if (!sourceMatches && translatedMatches) {
          const copy: Partial<Record<TranslationStyle, string>> = {
            ...(cue.translatedVariants ?? {}),
          };
          if (nextTranslated.trim().length > 0) {
            copy[activeStyle] = nextTranslated;
          } else {
            delete copy[activeStyle];
          }
          nextVariants = Object.keys(copy).length > 0 ? copy : undefined;
        }

        const updatedCue = recalculateCue(
          {
            ...cue,
            sourceText: sourceMatches
              ? cue.sourceText.replace(singleRegex, () => replaceText)
              : cue.sourceText,
            translatedText: nextTranslated,
            translatedVariants: nextVariants,
          },
          jobOptions.segmentRules,
        );

        const nextCues = cues.map((item, i) =>
          i === idx ? updatedCue : item,
        );

        set({ activeCueId: cue.id });
        const updatedProject: SubtitleProject = {
          ...activeProject,
          cues: nextCues,
          updatedAt: new Date().toISOString(),
        };
        syncProjectIntoQueueAndState(updatedProject);
        scheduleCheckpointAutosave(updatedProject);
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

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: previousCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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

      const updatedProject: SubtitleProject = {
        ...activeProject,
        cues: restoredCues,
        updatedAt: new Date().toISOString(),
      };
      syncProjectIntoQueueAndState(updatedProject);
      scheduleCheckpointAutosave(updatedProject);
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
        await flushCheckpointAutosave();
        await get().updateConfig({ export: options });
        const latestProject = get().activeProject ?? activeProject;
        const result = await deps.exportSubtitles(latestProject, options);
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
}

export const useAppStore = createAppStore();
