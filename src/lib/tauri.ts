import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  ApiKeyStatus,
  AppConfig,
  BugReportRequest,
  CatalogItemKind,
  CatalogManifest,
  CrashReportRequest,
  CustomModelEntry,
  DiagnosticsPreview,
  DownloadProgressEvent,
  ExportOptions,
  ExportResult,
  InstalledComponentStatus,
  JobOptions,
  JobProgressEvent,
  MediaProbeResult,
  ReportSubmitResult,
  SegmentRules,
  SerbianScript,
  SetupStatus,
  SubtitleCue,
  SubtitleProject,
  TranslationProviderKind,
  WaveformPreviewData,
} from "../types/models.ts";
import { browserFallback } from "./browserFallback.ts";
import { isTauriRuntime } from "./subtitleUtils.ts";

export const DOWNLOAD_PROGRESS_EVENT = "download-progress" as const;
export const JOB_PROGRESS_EVENT = "job-progress" as const;
export const PROBE_PROGRESS_EVENT = "probe-progress" as const;

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

export function installCoreRuntime(): Promise<SetupStatus> {
  if (!isTauriRuntime()) {
    return browserFallback.installCoreRuntime();
  }
  return invoke<SetupStatus>("install_core_runtime");
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

export function saveProjectCheckpoint(project: SubtitleProject): Promise<void> {
  if (!isTauriRuntime()) {
    return browserFallback.saveProjectCheckpoint(project);
  }
  return invoke<void>("save_project_checkpoint", { project });
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

export function condenseProjectCps(
  project: SubtitleProject,
): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.condenseProjectCps(project);
  }
  return invoke<SubtitleProject>("condense_project_cps", { project });
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

export function runLocalLlmBatch(
  projects: SubtitleProject[],
): Promise<SubtitleProject[]> {
  if (!isTauriRuntime()) {
    return browserFallback.runLocalLlmBatch(projects);
  }
  return invoke<SubtitleProject[]>("run_local_llm_batch", { projects });
}

export function cancelJob(jobId: string): Promise<void> {
  if (!isTauriRuntime()) {
    return browserFallback.cancelJob(jobId);
  }
  return invoke<void>("cancel_job", { jobId });
}

export function importSubtitleFile(
  subtitlePath: string,
  options: JobOptions,
): Promise<SubtitleProject> {
  if (!isTauriRuntime()) {
    return browserFallback.importSubtitleFile(subtitlePath, options);
  }
  return invoke<SubtitleProject>("import_subtitle_file", {
    subtitlePath,
    options,
  });
}

export async function tryLoadCompanionSubtitles(
  audioPath: string,
  options: JobOptions,
): Promise<SubtitleProject | null> {
  if (!isTauriRuntime()) {
    return null;
  }
  return invoke<SubtitleProject | null>("try_load_companion_subtitles", {
    audioPath,
    options,
  });
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

export async function readAudioPlaybackBytes(
  audioPath: string,
): Promise<ArrayBuffer> {
  if (!isTauriRuntime()) {
    return browserFallback.readAudioPlaybackBytes(audioPath);
  }
  const raw = await invoke<ArrayBuffer | number[]>(
    "read_audio_playback_bytes",
    { audioPath },
  );
  if (raw instanceof ArrayBuffer) {
    return raw;
  }
  return new Uint8Array(raw).buffer;
}

export function prepareAudioWaveform(
  audioPath: string,
  targetBuckets?: number,
): Promise<WaveformPreviewData> {
  if (!isTauriRuntime()) {
    return browserFallback.prepareAudioWaveform(audioPath, targetBuckets);
  }
  return invoke<WaveformPreviewData>("prepare_audio_waveform", {
    audioPath,
    targetBuckets: targetBuckets ?? null,
  });
}

export function probeMediaFile(
  jobId: string,
  audioPath: string,
): Promise<MediaProbeResult> {
  if (!isTauriRuntime()) {
    return browserFallback.probeMediaFile(jobId, audioPath);
  }
  return invoke<MediaProbeResult>("probe_media_file", { jobId, audioPath });
}

export function cancelMediaProbe(jobId?: string | null): Promise<boolean> {
  if (!isTauriRuntime()) {
    return browserFallback.cancelMediaProbe(jobId);
  }
  return invoke<boolean>("cancel_media_probe", { jobId: jobId ?? null });
}

export async function setWindowMaximized(
  maximized?: boolean | null,
): Promise<boolean> {
  if (!isTauriRuntime()) {
    return Boolean(maximized);
  }
  return invoke<boolean>("set_window_maximized", {
    maximized: maximized ?? null,
  });
}

export async function openDialogPreservingMaximize(
  options: Parameters<typeof open>[0],
): Promise<Awaited<ReturnType<typeof open>>> {
  try {
    return await open(options);
  } finally {
    if (isTauriRuntime()) {
      void setWindowMaximized(true).catch(() => {});
      setTimeout(() => {
        void setWindowMaximized(true).catch(() => {});
      }, 120);
    }
  }
}

// ---------------------------------------------------------------------------
// Diagnostics, Bug Reports & Crash Telemetry Commands
// ---------------------------------------------------------------------------

export function collectDiagnosticsPreview(): Promise<DiagnosticsPreview> {
  if (!isTauriRuntime()) {
    return browserFallback.collectDiagnosticsPreview();
  }
  return invoke<DiagnosticsPreview>("collect_diagnostics_preview");
}

export function submitBugReport(
  request: BugReportRequest,
): Promise<ReportSubmitResult> {
  if (!isTauriRuntime()) {
    return browserFallback.submitBugReport(request);
  }
  return invoke<ReportSubmitResult>("submit_bug_report", { request });
}

export function submitCrashReport(
  request: CrashReportRequest,
): Promise<ReportSubmitResult> {
  if (!isTauriRuntime()) {
    return browserFallback.submitCrashReport(request);
  }
  return invoke<ReportSubmitResult>("submit_crash_report", { request });
}

export function flushPendingCrashReports(): Promise<number> {
  if (!isTauriRuntime()) {
    return browserFallback.flushPendingCrashReports();
  }
  return invoke<number>("flush_pending_crash_reports");
}

export function saveDiagnosticsLocal(): Promise<string> {
  if (!isTauriRuntime()) {
    return browserFallback.saveDiagnosticsLocal();
  }
  return invoke<string>("save_diagnostics_local");
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

export function onProbeProgress(
  handler: (event: MediaProbeResult) => void,
): Promise<UnlistenFn> {
  if (!isTauriRuntime()) {
    return browserFallback.onProbeProgress(handler);
  }
  return listen<MediaProbeResult>(PROBE_PROGRESS_EVENT, (event) => {
    handler(event.payload);
  });
}
