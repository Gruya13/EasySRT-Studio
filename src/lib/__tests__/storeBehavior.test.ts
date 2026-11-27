// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in fs without @types/node
import { readFileSync } from "node:fs";
// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
import {
  CHECKPOINT_AUTOSAVE_DEBOUNCE_MS,
  createAppStore,
  getTranslationStyleCacheStatus,
  type AppStoreDeps,
} from "../../stores/appStore.ts";
import {
  MIN_INTER_CUE_GAP_MS,
  convertSerbianScriptText,
  getPresetSegmentRules,
} from "../subtitleUtils.ts";
import type {
  AppConfig,
  MediaProbeResult,
  SetupStatus,
  SubtitleProject,
} from "../../types/models.ts";

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function makeBaseConfig(overrides?: Partial<AppConfig>): AppConfig {
  return {
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
    defaultSegmentRules: getPresetSegmentRules("standard"),
    defaultScript: "latin",
    export: {
      outputDir: null,
      saveBesideAudio: true,
      includeSourceSrt: true,
      formats: ["srt"],
      utf8Bom: true,
    },
    hasCompletedSetup: true,
    privacyConsentVersion: 1,
    autoSendCrashReports: true,
    shareAnonymousUsageMetrics: false,
    shareAnonymizedTranslationEdits: false,
    autoCheckUpdates: true,
    lastUpdateCheckEpochSec: 0,
    editorSourceFontSize: 19,
    editorTranslatedFontSize: 22,
    autoProbeOnImport: true,
    autoCheckpoints: true,
    allowLocalEncryptedVault: false,
    ...overrides,
  };
}

function makeSetupStatus(): SetupStatus {
  return {
    platform: "linux-x64",
    gpu: {
      hasNvidia: true,
      gpuName: "NVIDIA GeForce RTX 4070",
      vramMb: 12288,
      recommendedTier: "vram12GbPlus",
    },
    storage: {
      mode: "installed",
      rootDir: "/tmp/easysrt",
      binDir: "/tmp/easysrt/bin",
      modelsDir: "/tmp/easysrt/models",
      dataDir: "/tmp/easysrt/data",
      configPath: "/tmp/easysrt/config.json",
    },
    components: [],
    coreRuntimeReady: true,
    hasLlmModel: true,
    readyForOffline: true,
    missingRequired: [],
  };
}

function makeSampleProject(
  overrides?: Partial<SubtitleProject>,
): SubtitleProject {
  const rules = getPresetSegmentRules("standard");
  return {
    jobId: "job-test-01",
    audioPath: "/media/sample.mp3",
    audioFileName: "sample.mp3",
    durationMs: 12000,
    detectedLanguage: "en",
    options: {
      sourceLanguage: "en",
      targetLanguage: "sr",
      script: "latin",
      segmentRules: rules,
      glossary: {},
      polishSerbian: true,
      correctSourceAsr: false,
      translationStyle: "natural",
      removeFillers: true,
      providerOverride: "localLlama",
    },
    words: [],
    cues: [
      {
        id: 1,
        startMs: 500,
        endMs: 2500,
        sourceText: "Hey friend, how are you doing today?",
        translatedText: "Hej brate, kako si danas?",
        translatedVariants: {
          natural: "Hej brate, kako si danas?",
          formal: "Poštovani, kako se osećate danas u studiju?",
        },
        cps: 12.5,
        warnings: [],
      },
      {
        id: 2,
        startMs: 3000,
        endMs: 6000,
        sourceText: "Let us review the studio workflow.",
        translatedText: "Hajde da prođemo kroz rad u studiju.",
        translatedVariants: {
          natural: "Hajde da prođemo kroz rad u studiju.",
          formal: "Pogledajmo zajedno postupak rada u studiju.",
        },
        cps: 12.0,
        warnings: [],
      },
    ],
    phase: "completed",
    completedTranslationBatches: 1,
    completedTranslationStyle: "natural",
    topicContext: null,
    errorMessage: null,
    updatedAt: "2026-11-25T09:00:00Z",
    ...overrides,
  };
}

function makeNoopDeps(overrides?: Partial<AppStoreDeps>): Partial<AppStoreDeps> {
  const baseConfig = makeBaseConfig();
  return {
    getSetupStatus: async () => makeSetupStatus(),
    getCatalog: async () => ({
      schemaVersion: 1,
      updatedAt: "2026-11-25T00:00:00Z",
      items: [],
    }),
    getConfig: async () => baseConfig,
    saveConfig: async (cfg) => cfg,
    getApiKeyStatus: async () => ({
      hasDeepSeekKey: false,
      hasCustomApiKey: false,
      storageBackend: "keyring",
    }),
    listCheckpoints: async () => [],
    loadCheckpoint: async () => makeSampleProject(),
    saveProjectCheckpoint: async () => {},
    tryLoadCompanionSubtitles: async () => null,
    flushPendingCrashReports: async () => 0,
    checkAppUpdate: async () => ({
      checked: false,
      skippedReason: "cooldown_24h",
      updateAvailable: false,
      currentVersion: "0.4.0",
      latestVersion: null,
      releaseNotes: null,
      releaseDate: null,
      lastUpdateCheckEpochSec: 0,
    }),
    probeMediaFile: async (jobId, audioPath) => ({
      jobId,
      audioPath,
      stage: "completed",
      durationMs: 12000,
      speechRatio: 0.8,
      estimatedSpeakers: 1,
      detectedLanguage: "en",
      topic: "Tech interview",
      suggestedStyle: "natural",
      error: null,
    }),
    cancelMediaProbe: async () => false,
    onDownloadProgress: async () => () => {},
    onJobProgress: async () => () => {},
    onProbeProgress: async () => () => {},
    getAppVersion: async () => "0.4.0",
    ...overrides,
  };
}

describe("appStore v0.4.0 behaviors", () => {
  it("debounces saveProjectCheckpoint by 600 ms after updateCue and flushes on job switch / export (BUG-H1)", async () => {
    assert.equal(CHECKPOINT_AUTOSAVE_DEBOUNCE_MS, 600);
    const saved: SubtitleProject[] = [];
    const store = createAppStore(
      makeNoopDeps({
        saveProjectCheckpoint: async (project) => {
          saved.push(project);
        },
        exportSubtitles: async () => ({ writtenFiles: ["/media/sample.sr.srt"] }),
      }),
    );

    const project = makeSampleProject();
    store.setState({
      activeJobId: project.jobId,
      activeProject: project,
      batchQueue: [
        {
          jobId: project.jobId,
          audioPath: project.audioPath,
          audioFileName: project.audioFileName,
          phase: "completed",
          percent: 100,
          detail: "completed",
          project,
          errorMessage: null,
        },
      ],
    });

    // Rapid edits coalesce into a single debounced checkpoint save after 600 ms
    store.getState().updateCue(1, { translatedText: "Prva izmena" });
    await delay(150);
    store.getState().updateCue(1, { translatedText: "Druga izmena" });
    await delay(200);
    assert.equal(
      saved.length,
      0,
      "Checkpoint must not save before 600 ms debounce elapses",
    );

    await delay(500);
    assert.equal(saved.length, 1);
    assert.equal(saved[0].cues[0].translatedText, "Druga izmena");
    assert.equal(saved[0].cues[0].translatedVariants?.natural, "Druga izmena");

    // Immediate flush on export (without waiting 600 ms)
    store.getState().updateCue(1, { translatedText: "Pre izvoza" });
    assert.equal(saved.length, 1);
    await store.getState().runExportSubtitles(store.getState().config.export);
    assert.equal(saved.length, 2);
    assert.equal(saved[1].cues[0].translatedText, "Pre izvoza");

    // Disabled autoCheckpoints suppresses disk saves
    await store.getState().updateConfig({ autoCheckpoints: false });
    store.getState().updateCue(1, { translatedText: "Bez autosave" });
    await delay(700);
    assert.equal(saved.length, 2);
  });

  it("preserves backend-written activeWhisperModelId and activeLlmModelId after runInstallRecommendedBundle (BUG-H2)", async () => {
    let diskConfig = makeBaseConfig({
      activeWhisperModelId: null,
      activeLlmModelId: null,
      hasCompletedSetup: false,
    });
    let saveConfigCalls = 0;

    const store = createAppStore(
      makeNoopDeps({
        getConfig: async () => diskConfig,
        saveConfig: async (cfg) => {
          saveConfigCalls += 1;
          diskConfig = cfg;
          return cfg;
        },
        installRecommendedBundle: async () => {
          // Simulate Rust backend atomically setting active models + has_completed_setup = true on disk
          diskConfig = {
            ...diskConfig,
            activeWhisperModelId: "whisper-large-v3-turbo-q5_0",
            activeLlmModelId: "llm-gemma4-12b-qat",
            hasCompletedSetup: true,
          };
          return makeSetupStatus();
        },
      }),
    );

    // Pre-populate stale in-memory config with null active models
    store.setState({
      config: makeBaseConfig({
        activeWhisperModelId: null,
        activeLlmModelId: null,
        hasCompletedSetup: false,
      }),
    });

    await store.getState().runInstallRecommendedBundle();

    assert.equal(
      saveConfigCalls,
      0,
      "Must NOT overwrite freshly written disk config with stale in-memory config",
    );
    assert.equal(
      store.getState().config.activeWhisperModelId,
      "whisper-large-v3-turbo-q5_0",
    );
    assert.equal(
      store.getState().config.activeLlmModelId,
      "llm-gemma4-12b-qat",
    );
    assert.equal(store.getState().config.hasCompletedSetup, true);
  });

  it("halts a 3-item batch queue cleanly on stopProcessingQueue() preserving cancelled item and queued remainder (BUG-M1)", async () => {
    const startedJobs: string[] = [];
    const cancelledJobs: string[] = [];
    let localLlmBatchCalled = false;
    const rejectFirstJobRef: { current: ((err: Error) => void) | null } = {
      current: null,
    };
    let firstJobStartedResolve: (() => void) | null = null;
    const firstJobStarted = new Promise<void>((resolve) => {
      firstJobStartedResolve = resolve;
    });

    const store = createAppStore(
      makeNoopDeps({
        getConfig: async () => makeBaseConfig({ autoProbeOnImport: false }),
        transcribeAudio: async (_audioPath, _options, jobId) => {
          startedJobs.push(jobId ?? "unknown");
          if (startedJobs.length === 1) {
            firstJobStartedResolve?.();
            return new Promise<SubtitleProject>((_resolve, reject) => {
              rejectFirstJobRef.current = reject;
            });
          }
          return makeSampleProject({ jobId: jobId ?? "unexpected" });
        },
        runLocalLlmBatch: async (projects) => {
          localLlmBatchCalled = true;
          return projects;
        },
        cancelJob: async (jobId) => {
          cancelledJobs.push(jobId);
        },
        loadCheckpoint: async () => {
          throw new Error("no checkpoint");
        },
      }),
    );

    store.setState({
      config: makeBaseConfig({ autoProbeOnImport: false }),
    });

    store
      .getState()
      .enqueueAudioFiles(["/media/1.mp3", "/media/2.mp3", "/media/3.mp3"]);
    const [item1, item2, item3] = store.getState().batchQueue;
    assert.equal(store.getState().batchQueue.length, 3);

    const batchPromise = store.getState().startBatchPipeline("full");
    await firstJobStarted;

    // Click Stop during item 1
    await store.getState().stopProcessingQueue();
    rejectFirstJobRef.current?.(new Error("Job cancelled by user"));
    await batchPromise;

    assert.deepEqual(startedJobs, [item1.jobId]);
    assert.deepEqual(cancelledJobs, [item1.jobId]);
    assert.equal(localLlmBatchCalled, false);

    const queue = store.getState().batchQueue;
    assert.equal(queue.find((q) => q.jobId === item1.jobId)?.phase, "cancelled");
    assert.equal(
      queue.find((q) => q.jobId === item1.jobId)?.errorMessage,
      null,
    );
    assert.equal(queue.find((q) => q.jobId === item2.jobId)?.phase, "queued");
    assert.equal(queue.find((q) => q.jobId === item3.jobId)?.phase, "queued");
    assert.equal(store.getState().isRunningBatch, false);
    assert.equal(store.getState().queueCancelRequested, false);
  });

  it("swaps dual-style variants in 0 ms with CPS recalculation, independent edits per style, and full script conversion (WS3)", async () => {
    const store = createAppStore(
      makeNoopDeps({
        convertCuesScript: async (cues, targetScript) =>
          cues.map((cue) => ({
            ...cue,
            sourceText: convertSerbianScriptText(cue.sourceText, targetScript),
            translatedText: convertSerbianScriptText(
              cue.translatedText,
              targetScript,
            ),
            translatedVariants: cue.translatedVariants
              ? Object.fromEntries(
                  Object.entries(cue.translatedVariants).map(([k, v]) => [
                    k,
                    v ? convertSerbianScriptText(v, targetScript) : v,
                  ]),
                )
              : undefined,
          })),
      }),
    );

    const project = makeSampleProject();
    store.setState({
      activeJobId: project.jobId,
      activeProject: project,
    });

    // Switch to formal in 0 ms -> translatedText swaps & CPS recalculates (44 chars / 2s = 22.0 CPS -> critical_cps)
    store.getState().switchTranslationStyle("formal");
    let active = store.getState().activeProject!;
    assert.equal(active.options.translationStyle, "formal");
    assert.equal(
      active.cues[0].translatedText,
      "Poštovani, kako se osećate danas u studiju?",
    );
    assert.equal(active.cues[0].cps, 21.5);
    assert.ok(active.cues[0].warnings.includes("critical_cps"));

    // Edit formal cue manually -> must NOT overwrite natural slot
    store.getState().updateCue(1, { translatedText: "Poštovani, dobar dan." });
    active = store.getState().activeProject!;
    assert.equal(
      active.cues[0].translatedVariants?.formal,
      "Poštovani, dobar dan.",
    );
    assert.equal(
      active.cues[0].translatedVariants?.natural,
      "Hej brate, kako si danas?",
    );

    // Switch back to natural -> original natural text preserved and CPS drops cleanly
    store.getState().switchTranslationStyle("natural");
    active = store.getState().activeProject!;
    assert.equal(active.cues[0].translatedText, "Hej brate, kako si danas?");
    assert.equal(active.cues[0].cps, 12.5);
    assert.deepEqual(active.cues[0].warnings, []);

    // Script conversion converts BOTH natural and formal variants
    await store.getState().convertActiveProjectScript("cyrillic");
    active = store.getState().activeProject!;
    assert.equal(active.cues[0].translatedText, "Хеј брате, како си данас?");
    assert.equal(
      active.cues[0].translatedVariants?.natural,
      "Хеј брате, како си данас?",
    );
    assert.equal(
      active.cues[0].translatedVariants?.formal,
      "Поштовани, добар дан.",
    );

    // Non-destructive switch to uncached style on a legacy single-style project
    const legacyProject = makeSampleProject({
      jobId: "legacy-01",
      cues: [
        {
          id: 1,
          startMs: 500,
          endMs: 3000,
          sourceText: "Hello",
          translatedText: "Zdravo",
          cps: 2.4,
          warnings: [],
        },
      ],
    });
    store.setState({
      activeJobId: legacyProject.jobId,
      activeProject: legacyProject,
    });
    store.getState().switchTranslationStyle("formal");
    const afterUncached = store.getState().activeProject!;
    assert.equal(afterUncached.options.translationStyle, "formal");
    assert.equal(
      afterUncached.cues[0].translatedText,
      "Zdravo",
      "Uncached style switch must preserve visible text non-destructively",
    );
    assert.equal(afterUncached.cues[0].translatedVariants?.natural, "Zdravo");
    assert.deepEqual(getTranslationStyleCacheStatus(afterUncached), {
      natural: true,
      formal: false,
    });
  });

  it("runs sequential background intake probes and preempts them via cancelMediaProbe(null) before Run (WS5)", async () => {
    const callOrder: string[] = [];
    const emitProbeProgressRef: {
      current: ((evt: MediaProbeResult) => void) | null;
    } = { current: null };
    const resolveProbe1Ref: {
      current: ((res: MediaProbeResult) => void) | null;
    } = { current: null };
    let probe1StartedResolve: (() => void) | null = null;
    const probe1Started = new Promise<void>((resolve) => {
      probe1StartedResolve = resolve;
    });

    const store = createAppStore(
      makeNoopDeps({
        onProbeProgress: async (handler) => {
          emitProbeProgressRef.current = handler;
          return () => {
            emitProbeProgressRef.current = null;
          };
        },
        probeMediaFile: async (jobId, audioPath) => {
          callOrder.push(`probe:${jobId}`);
          if (callOrder.length === 1) {
            probe1StartedResolve?.();
            return new Promise<MediaProbeResult>((resolve) => {
              resolveProbe1Ref.current = resolve;
            });
          }
          return {
            jobId,
            audioPath,
            stage: "completed",
            durationMs: 10000,
            speechRatio: 0.7,
            estimatedSpeakers: 1,
            detectedLanguage: "en",
            topic: "Second file topic",
            suggestedStyle: "natural",
            error: null,
          };
        },
        cancelMediaProbe: async (jobId) => {
          callOrder.push(`cancelProbe:${jobId ?? "all"}`);
          resolveProbe1Ref.current?.({
            jobId: store.getState().batchQueue[0]?.jobId ?? "",
            audioPath: "/media/a.mp3",
            stage: "cancelled",
            durationMs: 14220,
            speechRatio: 0.8,
            estimatedSpeakers: 2,
            detectedLanguage: "en",
            topic: "AI podcast",
            suggestedStyle: "natural",
            error: null,
          });
          return true;
        },
        transcribeAudio: async (audioPath, options, jobId) => {
          callOrder.push(`transcribe:${jobId}`);
          return makeSampleProject({
            jobId: jobId ?? "job-x",
            audioPath,
            options,
          });
        },
        runLocalLlmBatch: async (projects) => {
          callOrder.push(`llmBatch:${projects.map((p) => p.topicContext).join(",")}`);
          return projects;
        },
      }),
    );

    await store.getState().initApp();
    store.getState().enqueueAudioFiles(["/media/a.mp3", "/media/b.mp3"]);
    await probe1Started;

    const firstJobId = store.getState().batchQueue[0].jobId;
    // Progressive probe event updates probeResults + attaches topicContext to activeProject
    emitProbeProgressRef.current?.({
      jobId: firstJobId,
      audioPath: "/media/a.mp3",
      stage: "topic",
      durationMs: 14220,
      speechRatio: 0.81,
      estimatedSpeakers: 2,
      detectedLanguage: "en",
      topic: "AI podcast",
      suggestedStyle: "natural",
      error: null,
    });
    assert.equal(store.getState().probeResults[firstJobId]?.estimatedSpeakers, 2);
    assert.equal(store.getState().activeProject?.topicContext, "AI podcast");

    // Clicking Run MUST call cancelMediaProbe(null) BEFORE starting transcription
    await store.getState().startBatchPipeline("full");

    const cancelIdx = callOrder.indexOf("cancelProbe:all");
    const firstTranscribeIdx = callOrder.findIndex((c) =>
      c.startsWith("transcribe:"),
    );
    assert.ok(cancelIdx !== -1, "Expected cancelMediaProbe(null) to be called");
    assert.ok(
      firstTranscribeIdx !== -1 && cancelIdx < firstTranscribeIdx,
      `Expected cancelMediaProbe(null) before transcribeAudio, got order: ${callOrder.join(" -> ")}`,
    );
    assert.ok(
      callOrder.some((c) => c.includes("llmBatch:AI podcast")),
      `Expected probe topicContext to flow into LLM translation, got: ${callOrder.join(" -> ")}`,
    );
  });

  it("injects probed topicContext before LLM translation on single-file 1-Click Pipeline runs (WS5)", async () => {
    const observedTopics: Array<string | null | undefined> = [];
    let runFullPipelineCalled = false;

    const store = createAppStore(
      makeNoopDeps({
        getConfig: async () => makeBaseConfig({ autoProbeOnImport: false }),
        runFullPipeline: async () => {
          runFullPipelineCalled = true;
          return makeSampleProject();
        },
        transcribeAudio: async (audioPath, options, jobId) =>
          makeSampleProject({
            jobId: jobId ?? "job-single",
            audioPath,
            options,
            topicContext: null,
          }),
        translateProject: async (project) => {
          observedTopics.push(project.topicContext);
          return { ...project, phase: "completed" };
        },
      }),
    );

    store.setState({
      config: makeBaseConfig({ autoProbeOnImport: false }),
    });
    store.getState().enqueueAudioFiles(["/media/single_doc.mp3"]);
    const singleJobId = store.getState().batchQueue[0].jobId;

    // Seed completed probe metadata for the single queued file
    store.setState({
      probeResults: {
        [singleJobId]: {
          jobId: singleJobId,
          audioPath: "/media/single_doc.mp3",
          stage: "completed",
          durationMs: 42000,
          speechRatio: 0.85,
          estimatedSpeakers: 1,
          detectedLanguage: "en",
          topic: "Medical cardiology lecture",
          suggestedStyle: "formal",
          error: null,
        },
      },
    });

    await store.getState().startBatchPipeline("full");

    assert.equal(
      runFullPipelineCalled,
      false,
      "Single-file 1-Click run with topicContext must use transcribeAudio -> translateProject so topicContext reaches LLM",
    );
    assert.deepEqual(observedTopics, ["Medical cardiology lecture"]);
    assert.equal(
      store.getState().activeProject?.topicContext,
      "Medical cardiology lecture",
    );
    assert.equal(store.getState().batchQueue[0].phase, "completed");
  });

  it("resets queueCancelRequested after stopping standalone translateActiveProject so subsequent errors surface as failed", async () => {
    let attempt = 0;
    const rejectFirstRef: { current: ((err: Error) => void) | null } = {
      current: null,
    };
    let firstTranslateStartedResolve: (() => void) | null = null;
    const firstTranslateStarted = new Promise<void>((resolve) => {
      firstTranslateStartedResolve = resolve;
    });
    const emitJobProgressRef: {
      current: ((evt: {
        jobId: string;
        phase: "translating";
        percent: number;
        detail: string;
      }) => void) | null;
    } = { current: null };

    const store = createAppStore(
      makeNoopDeps({
        onJobProgress: async (handler) => {
          emitJobProgressRef.current = handler;
          return () => {
            emitJobProgressRef.current = null;
          };
        },
        cancelJob: async () => {},
        translateProject: async () => {
          attempt += 1;
          if (attempt === 1) {
            firstTranslateStartedResolve?.();
            return new Promise<SubtitleProject>((_resolve, reject) => {
              rejectFirstRef.current = reject;
            });
          }
          emitJobProgressRef.current?.({
            jobId: "job-test-01",
            phase: "translating",
            percent: 40,
            detail: "Retry batch 1/2",
          });
          throw new Error("Upstream HTTP 500 error");
        },
      }),
    );

    await store.getState().initApp();
    const project = makeSampleProject();
    store.setState({
      activeJobId: project.jobId,
      activeProject: project,
      batchQueue: [
        {
          jobId: project.jobId,
          audioPath: project.audioPath,
          audioFileName: project.audioFileName,
          phase: "completed",
          percent: 100,
          detail: "completed",
          project,
          errorMessage: null,
        },
      ],
    });

    // 1. Start standalone translate and click Stop
    const firstRun = store.getState().translateActiveProject();
    await firstTranslateStarted;
    await store.getState().stopProcessingQueue();
    rejectFirstRef.current?.(new Error("Job cancelled by user"));
    await firstRun;

    assert.equal(store.getState().activeProject?.phase, "cancelled");
    assert.equal(
      store.getState().queueCancelRequested,
      false,
      "queueCancelRequested must be cleared after standalone translate cancellation",
    );

    // 2. Retry standalone translate and hit a real network/API error
    await store.getState().translateActiveProject();
    assert.equal(store.getState().batchQueue[0].percent, 40);
    assert.equal(store.getState().activeProject?.phase, "failed");
    assert.equal(
      store.getState().activeProject?.errorMessage,
      "Upstream HTTP 500 error",
    );
    assert.equal(store.getState().banner?.type, "error");
    assert.equal(store.getState().banner?.message, "Upstream HTTP 500 error");
    assert.equal(store.getState().queueCancelRequested, false);
  });

  it("clamps editor font sizes (WS4) and leaves an 80 ms breathing gap when splitting cues (WS7)", async () => {
    const store = createAppStore(makeNoopDeps());
    await store.getState().setEditorFontSize("source", 99);
    assert.equal(store.getState().config.editorSourceFontSize, 34);
    await store.getState().setEditorFontSize("source", 5);
    assert.equal(store.getState().config.editorSourceFontSize, 13);

    await store.getState().setEditorFontSize("translated", 100);
    assert.equal(store.getState().config.editorTranslatedFontSize, 36);
    await store.getState().setEditorFontSize("translated", 8);
    assert.equal(store.getState().config.editorTranslatedFontSize, 14);

    const project = makeSampleProject();
    store.setState({
      activeJobId: project.jobId,
      activeProject: project,
    });
    store.getState().splitCueAtMidpoint(1, 1500);
    const cues = store.getState().activeProject!.cues;
    assert.equal(cues.length, 3);
    assert.equal(cues[0].endMs, 1460);
    assert.equal(cues[1].startMs, 1540);
    assert.equal(cues[1].startMs - cues[0].endMs, MIN_INTER_CUE_GAP_MS);
    assert.ok(cues[0].translatedVariants?.formal);
    assert.ok(cues[1].translatedVariants?.formal);
  });

  it("keeps EN and SR i18n dictionaries in 100% key parity with canonical cue warning IDs", () => {
    const en = JSON.parse(
      readFileSync(new URL("../../i18n/en.json", import.meta.url), "utf8"),
    ) as Record<string, unknown>;
    const sr = JSON.parse(
      readFileSync(new URL("../../i18n/sr.json", import.meta.url), "utf8"),
    ) as Record<string, unknown>;

    const collectKeys = (obj: Record<string, unknown>, prefix = ""): string[] =>
      Object.keys(obj)
        .sort()
        .flatMap((key) => {
          const val = obj[key];
          const full = prefix ? `${prefix}.${key}` : key;
          if (val && typeof val === "object" && !Array.isArray(val)) {
            return collectKeys(val as Record<string, unknown>, full);
          }
          return [full];
        });

    assert.deepEqual(collectKeys(en), collectKeys(sr));

    const enWarnings = (en.table as { warnings: Record<string, string> })
      .warnings;
    assert.deepEqual(Object.keys(enWarnings).sort(), [
      "critical_cps",
      "duration_too_long",
      "duration_too_short",
      "high_cps",
      "line_too_long",
      "too_many_lines",
    ]);
  });

  it("startBatchPipeline('full') preserves cached style on completed files when translating an uncached style", async () => {
    let transcribeCalls = 0;
    let runFullCalls = 0;
    const translatedInputs: SubtitleProject[] = [];

    const formalOnlyProject = makeSampleProject({
      jobId: "job-formal-01",
      options: {
        ...makeSampleProject().options,
        translationStyle: "formal",
      },
      completedTranslationStyle: "formal",
      completedTranslationBatches: 1,
      cues: [
        {
          id: 1,
          startMs: 500,
          endMs: 2500,
          sourceText: "How did you play?",
          translatedText: "Kako ste odigrali?",
          translatedVariants: { formal: "Kako ste odigrali?" },
          cps: 9.0,
          warnings: [],
        },
      ],
    });

    const store = createAppStore(
      makeNoopDeps({
        transcribeAudio: async () => {
          transcribeCalls += 1;
          return formalOnlyProject;
        },
        runFullPipeline: async () => {
          runFullCalls += 1;
          return formalOnlyProject;
        },
        translateProject: async (project) => {
          translatedInputs.push(project);
          return {
            ...project,
            phase: "completed",
            completedTranslationBatches: 1,
            completedTranslationStyle: "natural",
            cues: project.cues.map((c) => ({
              ...c,
              translatedText: "Kako si odigrao?",
              translatedVariants: {
                ...(c.translatedVariants ?? {}),
                natural: "Kako si odigrao?",
              },
            })),
          };
        },
      }),
    );

    store.setState({
      activeJobId: formalOnlyProject.jobId,
      activeProject: formalOnlyProject,
      batchQueue: [
        {
          jobId: formalOnlyProject.jobId,
          audioPath: formalOnlyProject.audioPath,
          audioFileName: formalOnlyProject.audioFileName,
          phase: "completed",
          percent: 100,
          detail: "completed",
          project: formalOnlyProject,
          errorMessage: null,
        },
      ],
    });

    // Switch style to uncached "natural" and click Run ("full")
    store.getState().switchTranslationStyle("natural");
    await store.getState().startBatchPipeline("full");

    assert.equal(transcribeCalls, 0);
    assert.equal(runFullCalls, 0);
    assert.equal(translatedInputs.length, 1);
    assert.equal(translatedInputs[0].options.translationStyle, "natural");
    assert.equal(translatedInputs[0].completedTranslationBatches, 0);
    assert.equal(
      translatedInputs[0].cues[0].translatedVariants?.formal,
      "Kako ste odigrali?",
    );
    assert.deepEqual(
      getTranslationStyleCacheStatus(store.getState().activeProject),
      { natural: true, formal: true },
    );

    // Switch back to formal -> instant 0 ms swap without losing formal
    store.getState().switchTranslationStyle("formal");
    assert.equal(
      store.getState().activeProject?.cues[0].translatedText,
      "Kako ste odigrali?",
    );

    // Multi-file localLlama batch: both completed files have formal cached, switched to natural
    const batchLlmInputs: SubtitleProject[][] = [];
    const projA = makeSampleProject({
      jobId: "job-batch-a",
      audioPath: "/media/a.mp3",
      audioFileName: "a.mp3",
      options: { ...makeSampleProject().options, translationStyle: "formal" },
      completedTranslationStyle: "formal",
      cues: [
        {
          id: 1,
          startMs: 500,
          endMs: 2500,
          sourceText: "First",
          translatedText: "Prvi formalno.",
          translatedVariants: { formal: "Prvi formalno." },
          cps: 7.0,
          warnings: [],
        },
      ],
    });
    const projB = makeSampleProject({
      jobId: "job-batch-b",
      audioPath: "/media/b.mp3",
      audioFileName: "b.mp3",
      options: { ...makeSampleProject().options, translationStyle: "formal" },
      completedTranslationStyle: "formal",
      cues: [
        {
          id: 1,
          startMs: 500,
          endMs: 2500,
          sourceText: "Second",
          translatedText: "Drugi formalno.",
          translatedVariants: { formal: "Drugi formalno." },
          cps: 7.5,
          warnings: [],
        },
      ],
    });
    let batchTranscribeCalls = 0;
    const localBatchStore = createAppStore(
      makeNoopDeps({
        transcribeAudio: async () => {
          batchTranscribeCalls += 1;
          return projA;
        },
        runLocalLlmBatch: async (projects) => {
          batchLlmInputs.push(projects);
          return projects.map((p) => ({ ...p, phase: "completed" as const }));
        },
      }),
    );
    localBatchStore.setState({
      activeJobId: projA.jobId,
      activeProject: projA,
      jobOptions: { ...localBatchStore.getState().jobOptions, translationStyle: "natural" },
      batchQueue: [
        {
          jobId: projA.jobId,
          audioPath: projA.audioPath,
          audioFileName: projA.audioFileName,
          phase: "completed",
          percent: 100,
          detail: "completed",
          project: projA,
          errorMessage: null,
        },
        {
          jobId: projB.jobId,
          audioPath: projB.audioPath,
          audioFileName: projB.audioFileName,
          phase: "completed",
          percent: 100,
          detail: "completed",
          project: projB,
          errorMessage: null,
        },
      ],
    });

    await localBatchStore.getState().startBatchPipeline("full");
    assert.equal(batchTranscribeCalls, 0);
    assert.equal(batchLlmInputs.length, 1);
    assert.equal(batchLlmInputs[0].length, 2);
    assert.equal(batchLlmInputs[0][0].options.translationStyle, "natural");
    assert.equal(
      batchLlmInputs[0][0].cues[0].translatedVariants?.formal,
      "Prvi formalno.",
    );
    assert.equal(
      batchLlmInputs[0][1].cues[0].translatedVariants?.formal,
      "Drugi formalno.",
    );
  });

  it("translateActiveProject locks isRunningBatch, resets batches on completed projects, and keeps resume on failed projects", async () => {
    let callCount = 0;
    const seenInputs: SubtitleProject[] = [];
    const resolveFirstRef: { current: ((p: SubtitleProject) => void) | null } = {
      current: null,
    };
    let firstStartedResolve: (() => void) | null = null;
    const firstStarted = new Promise<void>((r) => {
      firstStartedResolve = r;
    });

    const refreshedProject = makeSampleProject({
      jobId: "job-lock-01",
      completedTranslationBatches: 3,
    });

    const store = createAppStore(
      makeNoopDeps({
        listCheckpoints: async () => [refreshedProject],
        translateProject: async (project) => {
          callCount += 1;
          seenInputs.push(project);
          if (callCount === 1) {
            firstStartedResolve?.();
            return new Promise<SubtitleProject>((resolve) => {
              resolveFirstRef.current = resolve;
            });
          }
          return { ...project, phase: "completed" };
        },
      }),
    );

    const completedProj = makeSampleProject({
      jobId: "job-lock-01",
      phase: "completed",
      completedTranslationStyle: "natural",
      completedTranslationBatches: 3,
    });
    store.setState({
      activeJobId: completedProj.jobId,
      activeProject: completedProj,
      jobOptions: { ...store.getState().jobOptions, translationStyle: "natural" },
    });

    const p1 = store.getState().translateActiveProject();
    await firstStarted;
    assert.equal(store.getState().isRunningBatch, true);

    // Concurrent invocation must be ignored while locked
    await store.getState().translateActiveProject();
    assert.equal(callCount, 1);
    assert.equal(
      seenInputs[0].completedTranslationBatches,
      0,
      "Completed project re-translation must reset completedTranslationBatches to 0",
    );

    resolveFirstRef.current?.({
      ...seenInputs[0],
      phase: "completed",
      completedTranslationBatches: 3,
    });
    await p1;
    assert.equal(store.getState().isRunningBatch, false);
    assert.equal(store.getState().checkpoints.length, 1);

    // Failed project on the same style preserves completedTranslationBatches for resume
    const failedProj = makeSampleProject({
      jobId: "job-lock-01",
      phase: "failed",
      completedTranslationStyle: "natural",
      completedTranslationBatches: 2,
    });
    store.setState({
      activeJobId: failedProj.jobId,
      activeProject: failedProj,
    });

    await store.getState().translateActiveProject();
    assert.equal(callCount, 2);
    assert.equal(
      seenInputs[1].completedTranslationBatches,
      2,
      "Failed project resume must preserve completedTranslationBatches",
    );
    assert.equal(store.getState().isRunningBatch, false);
  });

  it("defaults followPlayhead to true and auto-enables correctSourceAsr when switching to Cloud providers", async () => {
    const store = createAppStore(
      makeNoopDeps({
        getConfig: async () =>
          makeBaseConfig({
            translation: {
              provider: "deepSeek",
              deepseekModel: "deepseek-chat",
              customBaseUrl: "https://api.openai.com/v1",
              customModel: "gpt-4o-mini",
              batchSize: 20,
              contextWindow: 4096,
            },
          }),
      }),
    );

    await store.getState().initApp();
    assert.equal(store.getState().config.followPlayhead, true);
    assert.equal(store.getState().jobOptions.addressForm, "auto");
    assert.equal(
      store.getState().jobOptions.correctSourceAsr,
      true,
      "Cloud provider active on startup should auto-enable correctSourceAsr",
    );

    await store.getState().setEffectiveProvider("localLlama");
    assert.equal(
      store.getState().jobOptions.correctSourceAsr,
      false,
      "Switching back to localLlama should default correctSourceAsr off",
    );

    await store.getState().setEffectiveProvider("customOpenAi");
    assert.equal(
      store.getState().jobOptions.correctSourceAsr,
      true,
      "Switching to customOpenAi should auto-enable correctSourceAsr",
    );
  });
});
