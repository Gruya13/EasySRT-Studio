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
import { browserFallback } from "./browserFallback";
import { isTauriRuntime } from "./subtitleUtils";

export const DOWNLOAD_PROGRESS_EVENT = "download-progress" as const;
export const JOB_PROGRESS_EVENT = "job-progress" as const;

// ---------------------------------------------------------------------------
// Setup & Catalog Commands
// ---------------------------------------------------------------------------

export function getSetupStatus(): Promise<SetupStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.getSetupStatus();
  }
  return invoke<SetupStatus>("get_setup_status");
}

export function getCatalog(): Promise<CatalogManifest> {
  if (!isTauriRuntime()) {
    return browserFallback.getCatalog();
  }
  return invoke<CatalogManifest>("get_catalog");
}

export function installComponent(
  itemId: string,
): Promise<InstalledComponentStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.installComponent(itemId);
  }
  return invoke<InstalledComponentStatus>("install_component", { itemId });
}

export function installRecommendedBundle(): Promise<SetupStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.installRecommendedBundle();
  }
  return invoke<SetupStatus>("install_recommended_bundle");
}

export function importCustomModel(
  sourcePath: string,
  kind: CatalogItemKind,
  name?: string | null,
): Promise<CustomModelEntry> {
  if (!isTauriRuntime()) {
    return browserFallback.importCustomModel(sourcePath, kind, name);
  }
  return invoke<CustomModelEntry>("import_custom_model", {
    sourcePath,
    kind,
    name: name ?? null,
  });
}

export function deleteModel(modelId: string): Promise<void> {
  if (!isTauriRuntime()) {
    return browserFallback.deleteModel(modelId);
  }
  return invoke<void>("delete_model", { modelId });
}

// ---------------------------------------------------------------------------
// Config & API Key Commands
// ---------------------------------------------------------------------------

export function getConfig(): Promise<AppConfig> {
  if (!isTauriRuntime()) {
    return browserFallback.getConfig();
  }
  return invoke<AppConfig>("get_config");
}

export function saveConfig(config: AppConfig): Promise<AppConfig> {
  if (!isTauriRuntime()) {
    return browserFallback.saveConfig(config);
  }
  return invoke<AppConfig>("save_config", { config });
}

export function getApiKeyStatus(): Promise<ApiKeyStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.getApiKeyStatus();
  }
  return invoke<ApiKeyStatus>("get_api_key_status");
}

export function setApiKey(
  provider: TranslationProviderKind,
  apiKey: string,
): Promise<ApiKeyStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.setApiKey(provider, apiKey);
  }
  return invoke<ApiKeyStatus>("set_api_key", { provider, apiKey });
}

export function clearApiKey(
  provider: TranslationProviderKind,
): Promise<ApiKeyStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.clearApiKey(provider);
  }
  return invoke<ApiKeyStatus>("clear_api_key", { provider });
}

// ---------------------------------------------------------------------------
// Checkpoint Commands
// ---------------------------------------------------------------------------

export function listCheckpoints(): Promise<SubtitleProject[]> {
  if (!isTauriRuntime()) {
    return browserFallback.listCheckpoints();
  }
  return invoke<SubtitleProject[]>("list_checkpoints");
}

export function loadCheckpoint(jobId: string): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.loadCheckpoint(jobId);
  }
  return invoke<SubtitleProject>("load_checkpoint", { jobId });
}

export function deleteCheckpoint(jobId: string): Promise<void> {
  if (!isTauriRuntime()) {
    return browserFallback.deleteCheckpoint(jobId);
  }
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
  if (!isTauriRuntime()) {
    return browserFallback.transcribeAudio(audioPath, options, jobId);
  }
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
  if (!isTauriRuntime()) {
    return browserFallback.resegmentProject(project, rules);
  }
  return invoke<SubtitleProject>("resegment_project", { project, rules });
}

export function translateProject(
  project: SubtitleProject,
): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.translateProject(project);
  }
  return invoke<SubtitleProject>("translate_project", { project });
}

export function polishProject(
  project: SubtitleProject,
): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.polishProject(project);
  }
  return invoke<SubtitleProject>("polish_project", { project });
}

export function runFullPipeline(
  audioPath: string,
  options: JobOptions,
  jobId?: string | null,
): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.runFullPipeline(audioPath, options, jobId);
  }
  return invoke<SubtitleProject>("run_full_pipeline", {
    audioPath,
    options,
    jobId: jobId ?? null,
  });
}

export function cancelJob(jobId: string): Promise<void> {
  if (!isTauriRuntime()) {
    return browserFallback.cancelJob(jobId);
  }
  return invoke<void>("cancel_job", { jobId });
}

// ---------------------------------------------------------------------------
// Script Conversion & Export Commands
// ---------------------------------------------------------------------------

export function convertCuesScript(
  cues: SubtitleCue[],
  targetScript: SerbianScript,
): Promise<SubtitleCue[]> {
  if (!isTauriRuntime()) {
    return browserFallback.convertCuesScript(cues, targetScript);
  }
  return invoke<SubtitleCue[]>("convert_cues_script", { cues, targetScript });
}

export function exportSubtitles(
  project: SubtitleProject,
  options: ExportOptions,
): Promise<ExportResult> {
  if (!isTauriRuntime()) {
    return browserFallback.exportSubtitles(project, options);
  }
  return invoke<ExportResult>("export_subtitles", { project, options });
}

// ---------------------------------------------------------------------------
// Event Subscriptions
// ---------------------------------------------------------------------------

export function onDownloadProgress(
  handler: (event: DownloadProgressEvent) => void,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return browserFallback.onDownloadProgress(handler);
  }
  return listen<DownloadProgressEvent>(DOWNLOAD_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}

export function onJobProgress(
  handler: (event: JobProgressEvent) => void,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return browserFallback.onJobProgress(handler);
  }
  return listen<JobProgressEvent>(JOB_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}
