/**
 * Canonical 1:1 shared contract with `src-tauri/src/models.rs`.
 * All Rust structs and enums use `#[serde(rename_all = "camelCase")]`.
 */

export type StorageMode = "installed";

export interface StoragePaths {
  mode: StorageMode;
  rootDir: string;
  binDir: string;
  modelsDir: string;
  dataDir: string;
  configPath: string;
}

export type VramTier = "cpuOnly" | "vram4Gb" | "vram8Gb" | "vram12GbPlus";

export interface GpuInfo {
  hasNvidia: boolean;
  gpuName: string | null;
  vramMb: number;
  recommendedTier: VramTier;
}

export type CatalogItemKind =
  | "ffmpeg"
  | "cudaRuntime"
  | "whisperBin"
  | "llamaBin"
  | "vadModel"
  | "whisperModel"
  | "llmModel";

export type ArchiveType = "zip" | "tarGz" | "raw";

export interface CatalogItem {
  id: string;
  name: string;
  kind: CatalogItemKind;
  platform: string;
  version: string;
  url: string;
  sha256: string | null;
  sizeBytes: number;
  vramTier: VramTier | null;
  targetSubdir: string;
  executableName: string | null;
  archiveType: ArchiveType;
  description: string;
  required: boolean;
}

export interface CatalogManifest {
  schemaVersion: number;
  updatedAt: string;
  items: CatalogItem[];
}

export interface InstalledComponentStatus {
  id: string;
  installed: boolean;
  path: string | null;
  sizeBytes: number;
}

export interface SetupStatus {
  platform: string;
  gpu: GpuInfo;
  storage: StoragePaths;
  components: InstalledComponentStatus[];
  coreRuntimeReady: boolean;
  hasLlmModel: boolean;
  readyForOffline: boolean;
  missingRequired: string[];
}

export interface DownloadProgressEvent {
  itemId: string;
  downloadedBytes: number;
  totalBytes: number;
  percent: number;
  stage: string;
  message: string;
}

export type SegmentPreset = "standard" | "reels" | "custom";

export interface SegmentRules {
  preset: SegmentPreset;
  maxCharsPerLine: number;
  maxLines: number;
  minDurationMs: number;
  maxDurationMs: number;
  maxCps: number;
}

export type SerbianScript = "latin" | "cyrillic";

export type TranslationProviderKind =
  | "localLlama"
  | "deepSeek"
  | "customOpenAi";

export interface CustomModelEntry {
  id: string;
  name: string;
  kind: CatalogItemKind;
  path: string;
  sizeBytes: number;
}

export interface TranslationConfig {
  provider: TranslationProviderKind;
  deepseekModel: string;
  customBaseUrl: string;
  customModel: string;
  batchSize: number;
  contextWindow: number;
}

export type ExportFormat = "srt" | "vtt" | "txt";

export interface ExportOptions {
  outputDir: string | null;
  saveBesideAudio: boolean;
  includeSourceSrt: boolean;
  formats: ExportFormat[];
  utf8Bom: boolean;
}

export interface AppConfig {
  uiLanguage: string;
  modelsDirOverride: string | null;
  activeWhisperModelId: string | null;
  activeLlmModelId: string | null;
  customModels: CustomModelEntry[];
  translation: TranslationConfig;
  defaultSegmentRules: SegmentRules;
  defaultScript: SerbianScript;
  export: ExportOptions;
  hasCompletedSetup: boolean;
}

export interface TimedWord {
  startMs: number;
  endMs: number;
  text: string;
  probability: number;
}

export interface SubtitleCue {
  id: number;
  startMs: number;
  endMs: number;
  sourceText: string;
  translatedText: string;
  cps: number;
  warnings: string[];
}

export interface JobOptions {
  sourceLanguage: string;
  targetLanguage: string;
  script: SerbianScript;
  segmentRules: SegmentRules;
  glossary: Record<string, string>;
  polishSerbian: boolean;
  correctSourceAsr?: boolean;
  providerOverride: TranslationProviderKind | null;
}

export type JobPhase =
  | "queued"
  | "convertingAudio"
  | "transcribing"
  | "segmenting"
  | "translating"
  | "polishing"
  | "completed"
  | "cancelled"
  | "failed";

export interface SubtitleProject {
  jobId: string;
  audioPath: string;
  audioFileName: string;
  durationMs: number;
  detectedLanguage: string | null;
  options: JobOptions;
  words: TimedWord[];
  cues: SubtitleCue[];
  phase: JobPhase;
  completedTranslationBatches: number;
  errorMessage: string | null;
  updatedAt: string;
}

export interface JobProgressEvent {
  jobId: string;
  phase: JobPhase;
  percent: number;
  detail: string;
}

export interface ExportResult {
  writtenFiles: string[];
}

export interface ApiKeyStatus {
  hasDeepSeekKey: boolean;
  hasCustomApiKey: boolean;
  storageBackend: string;
}

export interface AudioPreviewData {
  mimeType: string;
  base64Data: string;
}
