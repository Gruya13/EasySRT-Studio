import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApiKeyStatus,
  AppConfig,
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

export const DOWNLOAD_PROGRESS_EVENT = "download-progress" as const;
export const JOB_PROGRESS_EVENT = "job-progress" as const;

// ---------------------------------------------------------------------------
// Setup & Catalog Commands
// ---------------------------------------------------------------------------

export function getSetupStatus(): Promise<SetupStatus> {
  return invoke<SetupStatus>("get_setup_status");
}

export function getCatalog(): Promise<CatalogManifest> {
  return invoke<CatalogManifest>("get_catalog");
}

export function installComponent(
  itemId: string,
): Promise<InstalledComponentStatus> {
  return invoke<InstalledComponentStatus>("install_component", { itemId });
}

export function installRecommendedBundle(): Promise<SetupStatus> {
  return invoke<SetupStatus>("install_recommended_bundle");
}

export function importCustomModel(
  sourcePath: string,
  kind: CatalogItemKind,
  name?: string | null,
): Promise<CustomModelEntry> {
  return invoke<CustomModelEntry>("import_custom_model", {
    sourcePath,
    kind,
    name: name ?? null,
  });
}

export function deleteModel(modelId: string): Promise<void> {
  return invoke<void>("delete_model", { modelId });
}

// ---------------------------------------------------------------------------
// Config & API Key Commands
// ---------------------------------------------------------------------------

export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

export function saveConfig(config: AppConfig): Promise<AppConfig> {
  return invoke<AppConfig>("save_config", { config });
}

export function getApiKeyStatus(): Promise<ApiKeyStatus> {
  return invoke<ApiKeyStatus>("get_api_key_status");
}

export function setApiKey(
  provider: TranslationProviderKind,
  apiKey: string,
): Promise<ApiKeyStatus> {
  return invoke<ApiKeyStatus>("set_api_key", { provider, apiKey });
}

export function clearApiKey(
  provider: TranslationProviderKind,
): Promise<ApiKeyStatus> {
  return invoke<ApiKeyStatus>("clear_api_key", { provider });
}

// ---------------------------------------------------------------------------
// Checkpoint Commands
// ---------------------------------------------------------------------------

export function listCheckpoints(): Promise<SubtitleProject[]> {
  return invoke<SubtitleProject[]>("list_checkpoints");
}

export function loadCheckpoint(jobId: string): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("load_checkpoint", { jobId });
}

export function deleteCheckpoint(jobId: string): Promise<void> {
  return invoke<void>("delete_checkpoint", { jobId });
}

// ---------------------------------------------------------------------------
// Pipeline Commands
// ---------------------------------------------------------------------------

export function transcribeAudio(
  audioPath: string,
  options: JobOptions,
  jobId?: string | null,
): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("transcribe_audio", {
    audioPath,
    options,
    jobId: jobId ?? null,
  });
}

export function resegmentProject(
  project: SubtitleProject,
  rules: SegmentRules,
): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("resegment_project", { project, rules });
}

export function translateProject(
  project: SubtitleProject,
): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("translate_project", { project });
}

export function polishProject(
  project: SubtitleProject,
): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("polish_project", { project });
}

export function runFullPipeline(
  audioPath: string,
  options: JobOptions,
  jobId?: string | null,
): Promise<SubtitleProject> {
  return invoke<SubtitleProject>("run_full_pipeline", {
    audioPath,
    options,
    jobId: jobId ?? null,
  });
}

export function cancelJob(jobId: string): Promise<void> {
  return invoke<void>("cancel_job", { jobId });
}

// ---------------------------------------------------------------------------
// Script Conversion & Export Commands
// ---------------------------------------------------------------------------

export function convertCuesScript(
  cues: SubtitleCue[],
  targetScript: SerbianScript,
): Promise<SubtitleCue[]> {
  return invoke<SubtitleCue[]>("convert_cues_script", { cues, targetScript });
}

export function exportSubtitles(
  project: SubtitleProject,
  options: ExportOptions,
): Promise<ExportResult> {
  return invoke<ExportResult>("export_subtitles", { project, options });
}

// ---------------------------------------------------------------------------
// Event Subscriptions
// ---------------------------------------------------------------------------

export function onDownloadProgress(
  handler: (event: DownloadProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<DownloadProgressEvent>(DOWNLOAD_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}

export function onJobProgress(
  handler: (event: JobProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobProgressEvent>(JOB_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}
