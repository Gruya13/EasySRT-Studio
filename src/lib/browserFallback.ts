import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApiKeyStatus,
  AppConfig,
  CatalogItem,
  CatalogItemKind,
  CatalogManifest,
  CustomModelEntry,
  DownloadProgressEvent,
  ExportOptions,
  ExportResult,
  InstalledComponentStatus,
  JobOptions,
  JobProgressEvent,
  SegmentRules,
  SerbianScript,
  SetupStatus,
  SubtitleCue,
  SubtitleProject,
  TranslationProviderKind,
} from "../types/models";
import {
  convertSerbianScriptText,
  getPresetSegmentRules,
  parseSubtitleFileText,
  recalculateCue,
} from "./subtitleUtils";

const FALLBACK_CATALOG_ITEMS: CatalogItem[] = [
  {
    id: "ffmpeg-win-x64",
    name: "FFmpeg 9.1 Essentials (Windows x64)",
    kind: "ffmpeg",
    platform: "windows-x64",
    version: "9.1",
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip",
    sha256: "b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd",
    sizeBytes: 119467219,
    vramTier: null,
    targetSubdir: "ffmpeg",
    executableName: "ffmpeg-9.1-essentials_build/bin/ffmpeg.exe",
    archiveType: "zip",
    description:
      "Static FFmpeg 9.1 essentials build for extracting 16 kHz mono WAV audio.",
    required: true,
  },
  {
    id: "whisper-win-cuda",
    name: "whisper.cpp b5907 CUDA 12.4 (Windows x64)",
    kind: "whisperBin",
    platform: "windows-x64",
    version: "b5907-cuda12.4",
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5907/whisper-bin-win-cuda-12.4.0-x64.zip",
    sha256: "34f872ce724ca1ff0a4afca481853e5ee74250f8cb22153e499f953562846039",
    sizeBytes: 735571214,
    vramTier: "vram4Gb",
    targetSubdir: "whisper",
    executableName: "Release/whisper-cli.exe",
    archiveType: "zip",
    description:
      "NVIDIA CUDA 12.4 accelerated whisper-cli.exe (self-contained with CUDA DLLs in Release/).",
    required: true,
  },
  {
    id: "cudart-win-cuda12.4",
    name: "CUDA 12.4 Runtime DLLs for llama.cpp (Windows x64)",
    kind: "cudaRuntime",
    platform: "windows-x64",
    version: "b12633-cuda12.4",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b12633/cudart-llama-bin-win-cuda-12.4-x64.zip",
    sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
    sizeBytes: 391443627,
    vramTier: "vram4Gb",
    targetSubdir: "llama",
    executableName: "cudart64_12.dll",
    archiveType: "zip",
    description:
      "Companion cublas64_12.dll, cublasLt64_12.dll, and cudart64_12.dll extracted alongside llama-server.exe.",
    required: true,
  },
  {
    id: "llama-win-cuda",
    name: "llama.cpp b12633 CUDA 12.4 (Windows x64)",
    kind: "llamaBin",
    platform: "windows-x64",
    version: "b12633-cuda12.4",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b12633/llama-b12633-bin-win-cuda-12.4-x64.zip",
    sha256: "8c872e7fe215dcee32465051031454699b24967bebe0c2b746c50809472be0f0",
    sizeBytes: 357973767,
    vramTier: "vram4Gb",
    targetSubdir: "llama",
    executableName: "llama-server.exe",
    archiveType: "zip",
    description:
      "NVIDIA CUDA 12.4 accelerated llama-server.exe (requires cudart-win-cuda12.4 in the same folder).",
    required: true,
  },
  {
    id: "vad-silero-v6.2.0",
    name: "Silero VAD v6.2.0 (GGML)",
    kind: "vadModel",
    platform: "all",
    version: "6.2.0",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    sizeBytes: 885098,
    vramTier: null,
    targetSubdir: "vad",
    executableName: null,
    archiveType: "raw",
    description:
      "Official Silero VAD v6.2.0 model for whisper.cpp --vad to prevent silence hallucinations.",
    required: true,
  },
  {
    id: "whisper-large-v3-turbo-q5_0",
    name: "Whisper Large V3 Turbo (Q5_0)",
    kind: "whisperModel",
    platform: "all",
    version: "large-v3-turbo-q5_0",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    sizeBytes: 574041195,
    vramTier: "vram4Gb",
    targetSubdir: "whisper",
    executableName: null,
    archiveType: "raw",
    description:
      "Recommended default Whisper model for 4-8 GB VRAM: fast, high accuracy across 99 languages.",
    required: true,
  },
  {
    id: "whisper-large-v3-turbo",
    name: "Whisper Large V3 Turbo (FP16)",
    kind: "whisperModel",
    platform: "all",
    version: "large-v3-turbo-fp16",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    sizeBytes: 1624555275,
    vramTier: "vram12GbPlus",
    targetSubdir: "whisper",
    executableName: null,
    archiveType: "raw",
    description:
      "Full-precision FP16 Large V3 Turbo for maximum transcription fidelity on 12+ GB VRAM.",
    required: false,
  },
  {
    id: "whisper-medium-q5_0",
    name: "Whisper Medium (Q5_0)",
    kind: "whisperModel",
    platform: "all",
    version: "medium-q5_0",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin",
    sha256: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
    sizeBytes: 539212467,
    vramTier: "cpuOnly",
    targetSubdir: "whisper",
    executableName: null,
    archiveType: "raw",
    description: "Lightweight fallback Whisper model for CPU-only systems.",
    required: false,
  },
  {
    id: "llm-gemma3-4b-q4km",
    name: "Gemma 3 4B IT (Q4_K_M)",
    kind: "llmModel",
    platform: "all",
    version: "gemma-3-4b-it-Q4_K_M",
    url: "https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf",
    sha256: "882e8d2db44dc554fb0ea5077cb7e4bc49e7342a1f0da57901c0802ea21a0863",
    sizeBytes: 2489757856,
    vramTier: "vram4Gb",
    targetSubdir: "llm",
    executableName: null,
    archiveType: "raw",
    description:
      "Recommended 4 GB VRAM translator (2.49 GB): strong Serbian & multilingual fluency, fits RTX 3050 4 GB cleanly.",
    required: false,
  },
  {
    id: "llm-qwen3-4b-instruct-q4km",
    name: "Qwen 3 4B Instruct 2507 (Q4_K_M)",
    kind: "llmModel",
    platform: "all",
    version: "Qwen3-4B-Instruct-2507-Q4_K_M",
    url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    sha256: "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
    sizeBytes: 2497281120,
    vramTier: "vram4Gb",
    targetSubdir: "llm",
    executableName: null,
    archiveType: "raw",
    description:
      "Alternative 4 GB VRAM non-thinking instruct model (2.50 GB) with strong structured JSON compliance.",
    required: false,
  },
  {
    id: "llm-gemma4-e4b-qat",
    name: "Gemma 4 E4B IT (Official QAT Q4_0)",
    kind: "llmModel",
    platform: "all",
    version: "gemma-4-E4B-it-qat-q4_0",
    url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/main/gemma-4-E4B_q4_0-it.gguf",
    sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee",
    sizeBytes: 5154941280,
    vramTier: "vram8Gb",
    targetSubdir: "llm",
    executableName: null,
    archiveType: "raw",
    description:
      "Recommended 8 GB VRAM translator (5.15 GB): Apache 2.0 ungated Google QAT, native system prompt, 140+ languages.",
    required: false,
  },
  {
    id: "llm-gemma4-12b-qat",
    name: "Gemma 4 12B IT (Official QAT Q4_0)",
    kind: "llmModel",
    platform: "all",
    version: "gemma-4-12b-it-qat-q4_0",
    url: "https://huggingface.co/google/gemma-4-12B-it-qat-q4_0-gguf/resolve/main/gemma-4-12b-it-qat-q4_0.gguf",
    sha256: "93567e57a8fe10b23569b9d9ec38cd005deedf71e29477c421a4b83f418a538b",
    sizeBytes: 6975879296,
    vramTier: "vram12GbPlus",
    targetSubdir: "llm",
    executableName: null,
    archiveType: "raw",
    description:
      "Recommended 12+ GB VRAM translator (6.98 GB): Apache 2.0 ungated Google QAT preserving BF16 Serbian & translation quality.",
    required: false,
  },
];

let mockInstalledIds = new Set<string>([
  "ffmpeg-win-x64",
  "whisper-win-cuda",
  "cudart-win-cuda12.4",
  "llama-win-cuda",
  "vad-silero-v6.2.0",
  "whisper-large-v3-turbo-q5_0",
  "llm-gemma4-e4b-qat",
]);

let mockConfig: AppConfig = {
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
};

let mockApiKeyStatus: ApiKeyStatus = {
  hasDeepSeekKey: false,
  hasCustomApiKey: false,
  storageBackend: "portable-file",
};

const downloadListeners = new Set<(event: DownloadProgressEvent) => void>();
const jobListeners = new Set<(event: JobProgressEvent) => void>();

function createDemoProject(
  audioPath = "C:/Media/Podcast_Ep14_AI_Production.mp3",
  options?: JobOptions,
  jobId = "demo-session-01",
): SubtitleProject {
  const effectiveOptions: JobOptions = options ?? {
    sourceLanguage: "en",
    targetLanguage: "sr",
    script: mockConfig.defaultScript,
    segmentRules: mockConfig.defaultSegmentRules,
    glossary: {
      "EasySRT Studio": "EasySRT Studio",
      "whisper.cpp": "whisper.cpp",
      "NVIDIA RTX": "NVIDIA RTX",
    },
    polishSerbian: true,
    providerOverride: null,
  };

  const rawCues: Array<Omit<SubtitleCue, "cps" | "warnings">> = [
    {
      id: 1,
      startMs: 400,
      endMs: 3900,
      sourceText:
        "Welcome back to the studio! Today we are testing offline subtitle generation.",
      translatedText:
        "Dobro došli nazad u studio!\nDanas testiramo offline generisanje titlova.",
    },
    {
      id: 2,
      startMs: 4200,
      endMs: 8100,
      sourceText:
        "With whisper.cpp and local CUDA acceleration, transcription runs directly on your NVIDIA RTX card.",
      translatedText:
        "Uz whisper.cpp i lokalno CUDA ubrzanje,\ntranskripcija radi direktno na NVIDIA RTX kartici.",
    },
    {
      id: 3,
      startMs: 8400,
      endMs: 10300,
      sourceText:
        "This segment has a deliberately fast reading speed so you can inspect the CPS warning indicator in the editor.",
      translatedText:
        "Ovaj segment namerno ima veoma brz govor kako biste videli CPS indikator upozorenja u tabeli titlova.",
    },
    {
      id: 4,
      startMs: 10700,
      endMs: 14900,
      sourceText:
        "You can switch between Serbian Latin and Cyrillic scripts with a single click at any moment.",
      translatedText:
        "Možete prebacivati između srpske latinice\ni ćirilice jednim klikom u svakom trenutku.",
    },
  ];

  const cues = rawCues.map((c) => {
    const text =
      effectiveOptions.script === "cyrillic"
        ? convertSerbianScriptText(c.translatedText, "cyrillic")
        : c.translatedText;
    return recalculateCue(
      {
        ...c,
        translatedText:
          effectiveOptions.targetLanguage === "none" ? "" : text,
        cps: 0,
        warnings: [],
      },
      effectiveOptions.segmentRules,
    );
  });

  const fileName =
    audioPath.split(/[\\/]/).pop() || "Podcast_Ep14_AI_Production.mp3";

  return {
    jobId,
    audioPath,
    audioFileName: fileName,
    durationMs: 15500,
    detectedLanguage:
      effectiveOptions.sourceLanguage === "auto"
        ? "en"
        : effectiveOptions.sourceLanguage,
    options: effectiveOptions,
    words: [],
    cues,
    phase: "completed",
    completedTranslationBatches: 1,
    errorMessage: null,
    updatedAt: new Date().toISOString(),
  };
}

let mockCheckpoints: SubtitleProject[] = [createDemoProject()];

function buildSetupStatus(): SetupStatus {
  const components: InstalledComponentStatus[] = FALLBACK_CATALOG_ITEMS.map(
    (item) => ({
      id: item.id,
      installed: mockInstalledIds.has(item.id),
      path: mockInstalledIds.has(item.id)
        ? `C:/EasySRT Studio/${item.targetSubdir}/${item.executableName ?? item.id}`
        : null,
      sizeBytes: item.sizeBytes,
    }),
  );
  const requiredIds = [
    "ffmpeg-win-x64",
    "whisper-win-cuda",
    "vad-silero-v6.2.0",
    "whisper-large-v3-turbo-q5_0",
  ];
  const missingRequired = requiredIds.filter(
    (id) => !mockInstalledIds.has(id),
  );
  return {
    gpu: {
      hasNvidia: true,
      gpuName: "NVIDIA GeForce RTX 3060",
      vramMb: 8192,
      recommendedTier: "vram8Gb",
    },
    storage: {
      mode: "portable",
      rootDir: "C:/EasySRT Studio",
      binDir: "C:/EasySRT Studio/bin",
      modelsDir: mockConfig.modelsDirOverride ?? "C:/EasySRT Studio/models",
      dataDir: "C:/EasySRT Studio/data",
      configPath: "C:/EasySRT Studio/data/config.json",
    },
    components,
    readyForOffline: missingRequired.length === 0,
    missingRequired,
  };
}

export const browserFallback = {
  async getSetupStatus(): Promise<SetupStatus> {
    return buildSetupStatus();
  },

  async getCatalog(): Promise<CatalogManifest> {
    return {
      schemaVersion: 1,
      updatedAt: "2026-11-22T18:30:00Z",
      items: FALLBACK_CATALOG_ITEMS,
    };
  },

  async installComponent(itemId: string): Promise<InstalledComponentStatus> {
    const item = FALLBACK_CATALOG_ITEMS.find((i) => i.id === itemId);
    const total = item?.sizeBytes ?? 50_000_000;
    for (const pct of [25, 65, 100]) {
      await new Promise((r) => setTimeout(r, 120));
      const evt: DownloadProgressEvent = {
        itemId,
        downloadedBytes: Math.round((total * pct) / 100),
        totalBytes: total,
        percent: pct,
        stage: pct < 100 ? "downloading" : "completed",
        message: `${pct}%`,
      };
      downloadListeners.forEach((fn) => fn(evt));
    }
    mockInstalledIds.add(itemId);
    return {
      id: itemId,
      installed: true,
      path: `C:/EasySRT Studio/${item?.targetSubdir ?? "models"}/${itemId}`,
      sizeBytes: total,
    };
  },

  async installRecommendedBundle(): Promise<SetupStatus> {
    const bundleIds = [
      "ffmpeg-win-x64",
      "whisper-win-cuda",
      "cudart-win-cuda12.4",
      "llama-win-cuda",
      "vad-silero-v6.2.0",
      "whisper-large-v3-turbo-q5_0",
      "llm-gemma4-e4b-qat",
    ];
    for (const id of bundleIds) {
      if (!mockInstalledIds.has(id)) {
        await this.installComponent(id);
      }
    }
    mockConfig = { ...mockConfig, hasCompletedSetup: true };
    return buildSetupStatus();
  },

  async importCustomModel(
    sourcePath: string,
    kind: CatalogItemKind,
    name?: string | null,
  ): Promise<CustomModelEntry> {
    const fileName = sourcePath.split(/[\\/]/).pop() || "custom-model.gguf";
    const entry: CustomModelEntry = {
      id: `custom-${Date.now()}`,
      name: name?.trim() || fileName,
      kind,
      path: sourcePath,
      sizeBytes: 2_500_000_000,
    };
    mockConfig = {
      ...mockConfig,
      customModels: [...mockConfig.customModels, entry],
    };
    return entry;
  },

  async deleteModel(modelId: string): Promise<void> {
    mockInstalledIds.delete(modelId);
    mockConfig = {
      ...mockConfig,
      customModels: mockConfig.customModels.filter((m) => m.id !== modelId),
    };
  },

  async getConfig(): Promise<AppConfig> {
    return mockConfig;
  },

  async saveConfig(config: AppConfig): Promise<AppConfig> {
    mockConfig = { ...config };
    return mockConfig;
  },

  async getApiKeyStatus(): Promise<ApiKeyStatus> {
    return mockApiKeyStatus;
  },

  async setApiKey(
    provider: TranslationProviderKind,
    apiKey: string,
  ): Promise<ApiKeyStatus> {
    const hasKey = apiKey.trim().length > 0;
    if (provider === "deepSeek") {
      mockApiKeyStatus = { ...mockApiKeyStatus, hasDeepSeekKey: hasKey };
    } else if (provider === "customOpenAi") {
      mockApiKeyStatus = { ...mockApiKeyStatus, hasCustomApiKey: hasKey };
    }
    return mockApiKeyStatus;
  },

  async clearApiKey(
    provider: TranslationProviderKind,
  ): Promise<ApiKeyStatus> {
    if (provider === "deepSeek") {
      mockApiKeyStatus = { ...mockApiKeyStatus, hasDeepSeekKey: false };
    } else if (provider === "customOpenAi") {
      mockApiKeyStatus = { ...mockApiKeyStatus, hasCustomApiKey: false };
    }
    return mockApiKeyStatus;
  },

  async listCheckpoints(): Promise<SubtitleProject[]> {
    return mockCheckpoints;
  },

  async loadCheckpoint(jobId: string): Promise<SubtitleProject> {
    const found = mockCheckpoints.find((c) => c.jobId === jobId);
    return found ?? createDemoProject(undefined, undefined, jobId);
  },

  async deleteCheckpoint(jobId: string): Promise<void> {
    mockCheckpoints = mockCheckpoints.filter((c) => c.jobId !== jobId);
  },

  async transcribeAudio(
    audioPath: string,
    options: JobOptions,
    jobId?: string | null,
  ): Promise<SubtitleProject> {
    const id = jobId ?? `job-${Date.now()}`;
    const phases: Array<{ phase: JobProgressEvent["phase"]; pct: number; detail: string }> = [
      { phase: "convertingAudio", pct: 20, detail: "FFmpeg 16 kHz mono WAV" },
      { phase: "transcribing", pct: 65, detail: "Whisper ASR CUDA" },
      { phase: "segmenting", pct: 95, detail: "CPS & line segmentation" },
    ];
    for (const step of phases) {
      await new Promise((r) => setTimeout(r, 150));
      jobListeners.forEach((fn) =>
        fn({
          jobId: id,
          phase: step.phase,
          percent: step.pct,
          detail: step.detail,
        }),
      );
    }
    const project = createDemoProject(
      audioPath,
      { ...options, targetLanguage: "none" },
      id,
    );
    mockCheckpoints = [
      project,
      ...mockCheckpoints.filter((c) => c.jobId !== id),
    ];
    return project;
  },

  async resegmentProject(
    project: SubtitleProject,
    rules: SegmentRules,
  ): Promise<SubtitleProject> {
    const updatedCues = project.cues.map((cue) => recalculateCue(cue, rules));
    const updated: SubtitleProject = {
      ...project,
      options: {
        ...project.options,
        segmentRules: rules,
      },
      cues: updatedCues,
      updatedAt: new Date().toISOString(),
    };
    return updated;
  },

  async translateProject(project: SubtitleProject): Promise<SubtitleProject> {
    jobListeners.forEach((fn) =>
      fn({
        jobId: project.jobId,
        phase: "translating",
        percent: 50,
        detail: "Translating batch 1/1",
      }),
    );
    await new Promise((r) => setTimeout(r, 180));
    const demo = createDemoProject(
      project.audioPath,
      {
        ...project.options,
        targetLanguage:
          project.options.targetLanguage === "none"
            ? "sr"
            : project.options.targetLanguage,
      },
      project.jobId,
    );
    return {
      ...project,
      cues: project.cues.map((c, idx) => {
        const fallbackTranslated =
          demo.cues[idx]?.translatedText ||
          convertSerbianScriptText(c.sourceText, project.options.script);
        return recalculateCue(
          {
            ...c,
            translatedText: c.translatedText.trim() || fallbackTranslated,
          },
          project.options.segmentRules,
        );
      }),
      phase: "completed",
      updatedAt: new Date().toISOString(),
    };
  },

  async polishProject(project: SubtitleProject): Promise<SubtitleProject> {
    jobListeners.forEach((fn) =>
      fn({
        jobId: project.jobId,
        phase: "polishing",
        percent: 60,
        detail: "LLM Serbian transcript polishing",
      }),
    );
    await new Promise((r) => setTimeout(r, 160));
    return {
      ...project,
      cues: project.cues.map((c) =>
        recalculateCue(
          {
            ...c,
            translatedText: convertSerbianScriptText(
              c.translatedText.trim() || c.sourceText,
              project.options.script,
            ),
          },
          project.options.segmentRules,
        ),
      ),
      phase: "completed",
      updatedAt: new Date().toISOString(),
    };
  },

  async condenseProjectCps(project: SubtitleProject): Promise<SubtitleProject> {
    jobListeners.forEach((fn) =>
      fn({
        jobId: project.jobId,
        phase: "polishing",
        percent: 65,
        detail: "Condensing high-CPS subtitle cues",
      }),
    );
    await new Promise((r) => setTimeout(r, 140));
    const rules = project.options.segmentRules;
    const condenseTextToBudget = (text: string, maxChars: number): string => {
      const single = text.replace(/\s+/g, " ").trim();
      if (single.length <= maxChars) return single;
      const words = single.split(" ");
      let out = "";
      for (const w of words) {
        const candidate = out ? `${out} ${w}` : w;
        if (candidate.length > maxChars - 1 && out) break;
        out = candidate;
      }
      if (!out || out.length > maxChars) {
        return single.slice(0, Math.max(4, maxChars - 1)).trimEnd() + "…";
      }
      return `${out.replace(/[,;:\-]+$/, "")}.`;
    };

    const nextCues = project.cues.map((c) => {
      const needsFix =
        c.cps > rules.maxCps ||
        c.warnings.includes("high_cps") ||
        c.warnings.includes("line_too_long");
      if (!needsFix) return c;
      const durSec = Math.max(0.2, (c.endMs - c.startMs) / 1000);
      const maxChars = Math.min(
        rules.maxCharsPerLine * rules.maxLines,
        Math.max(12, Math.floor(durSec * rules.maxCps)),
      );
      const hasTranslated = c.translatedText.trim().length > 0;
      const raw = hasTranslated ? c.translatedText : c.sourceText;
      const condensed = condenseTextToBudget(raw, maxChars);
      return recalculateCue(
        hasTranslated
          ? { ...c, translatedText: condensed }
          : { ...c, sourceText: condensed },
        rules,
      );
    });

    const nextProject: SubtitleProject = {
      ...project,
      cues: nextCues,
      phase: "completed",
      updatedAt: new Date().toISOString(),
    };
    mockCheckpoints = [
      nextProject,
      ...mockCheckpoints.filter((c) => c.jobId !== project.jobId),
    ];
    return nextProject;
  },

  async runFullPipeline(
    audioPath: string,
    options: JobOptions,
    jobId?: string | null,
  ): Promise<SubtitleProject> {
    const id = jobId ?? `job-${Date.now()}`;
    const phases: Array<{ phase: JobProgressEvent["phase"]; pct: number; detail: string }> = [
      { phase: "convertingAudio", pct: 15, detail: "FFmpeg 16 kHz WAV" },
      { phase: "transcribing", pct: 50, detail: "Whisper CUDA ASR" },
      { phase: "segmenting", pct: 70, detail: "Segmenting cues" },
      { phase: "translating", pct: 90, detail: "LLM contextual translation" },
    ];
    for (const step of phases) {
      await new Promise((r) => setTimeout(r, 130));
      jobListeners.forEach((fn) =>
        fn({
          jobId: id,
          phase: step.phase,
          percent: step.pct,
          detail: step.detail,
        }),
      );
    }
    const project = createDemoProject(audioPath, options, id);
    mockCheckpoints = [
      project,
      ...mockCheckpoints.filter((c) => c.jobId !== id),
    ];
    return project;
  },

  async cancelJob(_jobId: string): Promise<void> {
    return;
  },

  async importSubtitleFile(
    subtitlePath: string,
    options: JobOptions,
  ): Promise<SubtitleProject> {
    const fileName =
      subtitlePath.split(/[\\/]/).pop() || "Imported_Subtitles.srt";
    const demo = createDemoProject(subtitlePath, options, `import-${Date.now()}`);
    const parsedCues = parseSubtitleFileText(
      `1\n00:00:00,500 --> 00:00:03,800\n${demo.cues[0]?.sourceText ?? "Imported subtitle segment."}\n\n2\n00:00:04,200 --> 00:00:08,100\n${demo.cues[1]?.sourceText ?? "Ready for translation or Serbian script conversion."}\n`,
      options.segmentRules,
    ).map((c) =>
      options.sourceLanguage === "sr"
        ? recalculateCue(
            {
              ...c,
              sourceText: convertSerbianScriptText(
                c.sourceText,
                options.script,
              ),
            },
            options.segmentRules,
          )
        : c,
    );
    const durationMs = parsedCues.reduce((max, c) => Math.max(max, c.endMs), 0);
    const project: SubtitleProject = {
      ...demo,
      audioPath: subtitlePath,
      audioFileName: fileName,
      durationMs,
      cues: parsedCues,
      phase: "completed",
      updatedAt: new Date().toISOString(),
    };
    mockCheckpoints = [
      project,
      ...mockCheckpoints.filter((c) => c.jobId !== project.jobId),
    ];
    return project;
  },

  async convertCuesScript(
    cues: SubtitleCue[],
    targetScript: SerbianScript,
  ): Promise<SubtitleCue[]> {
    return cues.map((cue) => ({
      ...cue,
      sourceText: convertSerbianScriptText(cue.sourceText, targetScript),
      translatedText: convertSerbianScriptText(
        cue.translatedText,
        targetScript,
      ),
    }));
  },

  async exportSubtitles(
    project: SubtitleProject,
    options: ExportOptions,
  ): Promise<ExportResult> {
    const baseDir = options.saveBesideAudio
      ? project.audioPath.replace(/[\\/][^\\/]+$/, "") || "C:/Exports"
      : options.outputDir || "C:/Exports";
    const stem = project.audioFileName.replace(/\.[^.]+$/, "");
    const writtenFiles = options.formats.map(
      (fmt) => `${baseDir}/${stem}.${project.options.targetLanguage || "sr"}.${fmt}`,
    );
    if (options.includeSourceSrt && options.formats.includes("srt")) {
      writtenFiles.push(`${baseDir}/${stem}.source.srt`);
    }
    return { writtenFiles };
  },

  async onDownloadProgress(
    handler: (event: DownloadProgressEvent) => void,
  ): Promise<UnlistenFn> {
    downloadListeners.add(handler);
    return () => {
      downloadListeners.delete(handler);
    };
  },

  async onJobProgress(
    handler: (event: JobProgressEvent) => void,
  ): Promise<UnlistenFn> {
    jobListeners.add(handler);
    return () => {
      jobListeners.delete(handler);
    };
  },
};
