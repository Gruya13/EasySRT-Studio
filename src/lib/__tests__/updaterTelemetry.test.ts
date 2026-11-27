// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in fs without @types/node
import { readFileSync, readdirSync, statSync } from "node:fs";
// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
// @ts-ignore Node built-in url without @types/node
import { fileURLToPath } from "node:url";
import {
  createAppStore,
  isEffectiveCrashReportingEnabled,
  type AppStoreDeps,
} from "../../stores/appStore.ts";
import { getPresetSegmentRules } from "../subtitleUtils.ts";
import {
  CURRENT_PRIVACY_CONSENT_VERSION,
  type AppConfig,
  type SetupStatus,
  type UpdateCheckResult,
} from "../../types/models.ts";

function makeLegacyConfig(overrides?: Partial<AppConfig>): AppConfig {
  return {
    uiLanguage: "en",
    modelsDirOverride: null,
    activeWhisperModelId: null,
    activeLlmModelId: null,
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
    ...overrides,
  };
}

function makeSetupStatus(): SetupStatus {
  return {
    platform: "linux-x64",
    gpu: {
      hasNvidia: false,
      gpuName: null,
      vramMb: 0,
      recommendedTier: "cpuOnly",
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

function makeDeps(
  config: AppConfig,
  overrides?: Partial<AppStoreDeps>,
): Partial<AppStoreDeps> {
  return {
    getSetupStatus: async () => makeSetupStatus(),
    getCatalog: async () => ({
      schemaVersion: 1,
      updatedAt: "2026-11-26T00:00:00Z",
      items: [],
    }),
    getConfig: async () => config,
    saveConfig: async (cfg) => cfg,
    getApiKeyStatus: async () => ({
      hasDeepSeekKey: false,
      hasCustomApiKey: false,
      storageBackend: "keyring",
    }),
    listCheckpoints: async () => [],
    flushPendingCrashReports: async () => 0,
    onDownloadProgress: async () => () => {},
    onJobProgress: async () => () => {},
    onProbeProgress: async () => () => {},
    getAppVersion: async () => "0.5.0",
    ...overrides,
  };
}

function makeUpdateResult(
  overrides?: Partial<UpdateCheckResult>,
): UpdateCheckResult {
  return {
    checked: true,
    skippedReason: null,
    updateAvailable: false,
    currentVersion: "0.5.0",
    latestVersion: null,
    releaseNotes: null,
    releaseDate: null,
    lastUpdateCheckEpochSec: 1_800_000_000,
    ...overrides,
  };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 20));
}

function walkSourceFiles(dir: URL): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const child = new URL(entry, dir);
    if (statSync(child).isDirectory()) {
      if (entry === "__tests__" || entry === "node_modules") continue;
      out.push(...walkSourceFiles(new URL(`${entry}/`, dir)));
    } else if (/\.(ts|tsx)$/.test(entry)) {
      out.push(fileURLToPath(child));
    }
  }
  return out;
}

describe("v0.5.0 privacy consent & CSP-safe updater (store)", () => {
  it("isEffectiveCrashReportingEnabled requires consent version >= 1 AND the opt-in toggle", () => {
    assert.equal(CURRENT_PRIVACY_CONSENT_VERSION, 1);
    assert.equal(
      isEffectiveCrashReportingEnabled(
        makeLegacyConfig({ privacyConsentVersion: 0, autoSendCrashReports: true }),
      ),
      false,
    );
    assert.equal(
      isEffectiveCrashReportingEnabled(
        makeLegacyConfig({ privacyConsentVersion: 1, autoSendCrashReports: false }),
      ),
      false,
    );
    assert.equal(
      isEffectiveCrashReportingEnabled(
        makeLegacyConfig({ privacyConsentVersion: 1, autoSendCrashReports: true }),
      ),
      true,
    );
    // v0.4.0 configs have no privacyConsentVersion key at all
    assert.equal(
      isEffectiveCrashReportingEnabled(
        makeLegacyConfig({ autoSendCrashReports: true }),
      ),
      false,
    );
  });

  it("does NOT flush pending crash reports for v0.4.0 upgraders (consent 0) even with autoSendCrashReports: true", async () => {
    let flushCalls = 0;
    const config = makeLegacyConfig({
      privacyConsentVersion: 0,
      autoSendCrashReports: true,
    });
    const store = createAppStore(
      makeDeps(config, {
        flushPendingCrashReports: async () => {
          flushCalls += 1;
          return 0;
        },
        checkAppUpdate: async () => makeUpdateResult({ checked: false }),
      }),
    );
    await store.getState().initApp();
    await settle();
    assert.equal(flushCalls, 0);
  });

  it("flushes pending crash reports only after explicit consent v1 + opt-in", async () => {
    let flushCalls = 0;
    const config = makeLegacyConfig({
      privacyConsentVersion: 1,
      autoSendCrashReports: true,
    });
    const store = createAppStore(
      makeDeps(config, {
        flushPendingCrashReports: async () => {
          flushCalls += 1;
          return 0;
        },
        checkAppUpdate: async () => makeUpdateResult({ checked: false }),
      }),
    );
    await store.getState().initApp();
    await settle();
    assert.equal(flushCalls, 1);
  });

  it("normalizes missing v0.4.0 config keys to privacy-safe defaults on load", async () => {
    // Simulate a v0.4.0 config.json that predates all consent fields
    const legacy = makeLegacyConfig();
    const store = createAppStore(
      makeDeps(legacy, {
        checkAppUpdate: async () =>
          makeUpdateResult({ checked: false, lastUpdateCheckEpochSec: 0 }),
      }),
    );
    await store.getState().initApp();
    const cfg = store.getState().config;
    assert.equal(cfg.privacyConsentVersion, 0);
    assert.equal(cfg.autoSendCrashReports, false);
    assert.equal(cfg.shareAnonymousUsageMetrics, false);
    assert.equal(cfg.shareAnonymizedTranslationEdits, false);
    assert.equal(cfg.autoCheckUpdates, true);
    assert.equal(cfg.lastUpdateCheckEpochSec, 0);
  });

  it("initApp calls deps.checkAppUpdate(false) via IPC and never calls global fetch", async () => {
    const calls: boolean[] = [];
    const fetchCalls: string[] = [];
    const g = globalThis as unknown as { fetch?: unknown; window?: unknown };
    const prevFetch = g.fetch;
    const prevWindow = g.window;
    g.fetch = (input: unknown) => {
      fetchCalls.push(String(input));
      return Promise.reject(new Error("fetch must not be called"));
    };
    g.window = { addEventListener: () => {} };
    try {
      const store = createAppStore(
        makeDeps(makeLegacyConfig(), {
          checkAppUpdate: async (force) => {
            calls.push(force);
            return makeUpdateResult({
              updateAvailable: true,
              latestVersion: "v0.6.0",
              releaseNotes: "Notes",
              lastUpdateCheckEpochSec: 1_800_000_123,
            });
          },
        }),
      );
      await store.getState().initApp();
      await settle();

      assert.deepEqual(calls, [false]);
      assert.deepEqual(fetchCalls, []);
      assert.equal(store.getState().availableUpdateVersion, "0.6.0");
      assert.equal(store.getState().config.lastUpdateCheckEpochSec, 1_800_000_123);
    } finally {
      g.fetch = prevFetch;
      g.window = prevWindow;
    }
  });

  it("keeps availableUpdateVersion null and survives IPC failure when skipped or offline", async () => {
    const skippedStore = createAppStore(
      makeDeps(makeLegacyConfig({ autoCheckUpdates: false }), {
        checkAppUpdate: async () =>
          makeUpdateResult({ checked: false, skippedReason: "auto_check_disabled" }),
      }),
    );
    await skippedStore.getState().initApp();
    await settle();
    assert.equal(skippedStore.getState().availableUpdateVersion, null);

    const failingStore = createAppStore(
      makeDeps(makeLegacyConfig(), {
        checkAppUpdate: async () => {
          throw new Error("network down");
        },
      }),
    );
    await failingStore.getState().initApp();
    await settle();
    assert.equal(failingStore.getState().initialized, true);
    assert.equal(failingStore.getState().banner, null);
    assert.equal(failingStore.getState().availableUpdateVersion, null);
  });

  it("passes skipTrainingDonation through runExportSubtitles to the exportSubtitles IPC", async () => {
    const seen: Array<boolean | undefined> = [];
    const store = createAppStore(
      makeDeps(makeLegacyConfig(), {
        saveProjectCheckpoint: async () => {},
        exportSubtitles: async (_project, _options, skip) => {
          seen.push(skip);
          return { writtenFiles: ["/x/a.srt"] };
        },
      }),
    );
    store.setState({
      activeProject: {
        jobId: "job-1",
        audioPath: "/x/a.mp3",
        audioFileName: "a.mp3",
        durationMs: 1000,
        detectedLanguage: "en",
        options: {
          sourceLanguage: "en",
          targetLanguage: "sr",
          script: "latin",
          segmentRules: getPresetSegmentRules("standard"),
          glossary: {},
          polishSerbian: true,
          providerOverride: null,
        },
        words: [],
        cues: [],
        phase: "completed",
        completedTranslationBatches: 0,
        errorMessage: null,
        updatedAt: "2026-11-26T00:00:00Z",
      },
    });
    await store.getState().runExportSubtitles(store.getState().config.export, true);
    await store.getState().runExportSubtitles(store.getState().config.export);
    assert.deepEqual(seen, [true, undefined]);
  });
});

describe("v0.5.0 privacy i18n & source hygiene", () => {
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

  it("keeps EN/SR privacy, export-notice, and updater keys in 100% parity", () => {
    const en = JSON.parse(
      readFileSync(new URL("../../i18n/en.json", import.meta.url), "utf8"),
    ) as Record<string, Record<string, string>>;
    const sr = JSON.parse(
      readFileSync(new URL("../../i18n/sr.json", import.meta.url), "utf8"),
    ) as Record<string, Record<string, string>>;

    assert.deepEqual(collectKeys(en), collectKeys(sr));

    for (const key of [
      "sectionTitle",
      "offlineGuarantee",
      "autoSendCrashReports",
      "shareAnonymousUsageMetrics",
      "shareAnonymizedTranslationEdits",
      "autoCheckUpdates",
      "inspectPayloadBtn",
      "envDisabledNotice",
      "upgradeBannerTitle",
      "upgradeConfirmBtn",
    ]) {
      assert.ok(en.privacy[key], `en.privacy.${key} missing`);
      assert.ok(sr.privacy[key], `sr.privacy.${key} missing`);
    }
    assert.equal(sr.privacy.inspectPayloadBtn, "Pogledaj tačan primer JSON-a");
    assert.equal(en.privacy.inspectPayloadBtn, "View Exact JSON Sample");
    assert.ok(en.export.trainingDonationNotice.includes("{count}"));
    assert.ok(sr.export.trainingDonationNotice.includes("{count}"));
    assert.ok(en.export.skipForThisExport);
    assert.ok(sr.export.skipForThisExport);
  });

  it("has no direct api.github.com reference left anywhere in src/ (CSP V05-H2)", () => {
    const srcRoot = new URL("../../", import.meta.url);
    const offenders = walkSourceFiles(srcRoot).filter((file) =>
      readFileSync(file, "utf8").includes("api.github.com"),
    );
    assert.deepEqual(offenders, []);
  });
});
