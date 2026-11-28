import { getCurrentWindow } from "@tauri-apps/api/window";
import { Download, RotateCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatDurationShort, isTauriRuntime } from "../lib/subtitleUtils";
import { openDialogPreservingMaximize } from "../lib/tauri";
import { useAppStore, type BatchQueueItem } from "../stores/appStore";
import type { JobPhase } from "../types/models";

const SUPPORTED_EXTENSIONS = [
  "mp3",
  "wav",
  "m4a",
  "flac",
  "ogg",
  "aac",
  "opus",
  "mp4",
  "mkv",
  "webm",
  "srt",
  "vtt",
];

function isSubtitleFileName(nameOrPath: string): boolean {
  return /\.(srt|vtt)$/i.test(nameOrPath.trim());
}

function getFileFormatLabel(fileName: string): string {
  const match = /\.([a-z0-9]+)$/i.exec(fileName.trim());
  return match ? match[1].toUpperCase() : "MEDIA";
}

function isProcessingPhase(phase: JobPhase): boolean {
  return (
    phase === "convertingAudio" ||
    phase === "transcribing" ||
    phase === "segmenting" ||
    phase === "translating" ||
    phase === "polishing"
  );
}

function readBrowserFileText(file: File): Promise<string> {
  if (typeof file.text === "function") {
    return file.text();
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () =>
      reject(reader.error ?? new Error("Failed to read file"));
    reader.readAsText(file, "utf-8");
  });
}

type RowVisualStatus = "done" | "processing" | "queued" | "failed" | "cancelled";

function toRowStatus(phase: JobPhase): RowVisualStatus {
  if (phase === "completed") return "done";
  if (isProcessingPhase(phase)) return "processing";
  if (phase === "failed") return "failed";
  if (phase === "cancelled") return "cancelled";
  return "queued";
}

export function FilesPanel() {
  const config = useAppStore((s) => s.config);
  const batchQueue = useAppStore((s) => s.batchQueue);
  const activeJobId = useAppStore((s) => s.activeJobId);
  const activeProject = useAppStore((s) => s.activeProject);
  const isRunningBatch = useAppStore((s) => s.isRunningBatch);
  const probeResults = useAppStore((s) => s.probeResults);

  const enqueueAudioFiles = useAppStore((s) => s.enqueueAudioFiles);
  const importSubtitleFromPath = useAppStore((s) => s.importSubtitleFromPath);
  const importSubtitleFromText = useAppStore((s) => s.importSubtitleFromText);
  const selectQueueItem = useAppStore((s) => s.selectQueueItem);
  const removeQueueItem = useAppStore((s) => s.removeQueueItem);
  const reprocessQueueItem = useAppStore((s) => s.reprocessQueueItem);
  const setExportOpen = useAppStore((s) => s.setExportOpen);
  const startBatchPipeline = useAppStore((s) => s.startBatchPipeline);
  const stopProcessingQueue = useAppStore((s) => s.stopProcessingQueue);

  const [isDragging, setIsDragging] = useState(false);
  const filePickerRef = useRef<HTMLInputElement | null>(null);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const displayRows: BatchQueueItem[] = useMemo(() => {
    if (batchQueue.length > 0) {
      return batchQueue;
    }
    if (activeProject) {
      return [
        {
          jobId: activeProject.jobId,
          audioPath: activeProject.audioPath,
          audioFileName: activeProject.audioFileName,
          phase: activeProject.phase,
          percent: activeProject.phase === "completed" ? 100 : 0,
          detail: activeProject.phase,
          project: activeProject,
          errorMessage: activeProject.errorMessage,
        },
      ];
    }
    return [];
  }, [batchQueue, activeProject]);

  const doneCount = useMemo(
    () => displayRows.filter((r) => r.phase === "completed").length,
    [displayRows],
  );

  const handleSelectedPaths = (paths: string[]) => {
    const audioPaths: string[] = [];
    for (const path of paths) {
      if (!path) continue;
      if (isSubtitleFileName(path)) {
        void importSubtitleFromPath(path);
      } else {
        audioPaths.push(path);
      }
    }
    if (audioPaths.length > 0) {
      enqueueAudioFiles(audioPaths);
    }
  };

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let unlisten: (() => void) | null = null;
    let active = true;

    void getCurrentWindow()
      .onDragDropEvent((event) => {
        if (!active) return;
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setIsDragging(true);
        } else if (payload.type === "drop") {
          setIsDragging(false);
          if (Array.isArray(payload.paths) && payload.paths.length > 0) {
            handleSelectedPaths(payload.paths);
          }
        } else {
          setIsDragging(false);
        }
      })
      .then((fn) => {
        if (active) {
          unlisten = fn;
        } else {
          fn();
        }
      })
      .catch(() => {
        // Ignore if drag-drop listener is unavailable
      });

    return () => {
      active = false;
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  const handleAddFilesClick = async () => {
    if (isTauriRuntime()) {
      try {
        const selected = await openDialogPreservingMaximize({
          multiple: true,
          filters: [
            {
              name: "Media & Subtitles",
              extensions: SUPPORTED_EXTENSIONS,
            },
          ],
        });
        if (Array.isArray(selected) && selected.length > 0) {
          handleSelectedPaths(selected);
        } else if (typeof selected === "string" && selected) {
          handleSelectedPaths([selected]);
        }
        return;
      } catch {
        // Fallback to hidden file input
      }
    }
    filePickerRef.current?.click();
  };

  const handleBrowserFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) {
      return;
    }
    const audioPaths: string[] = [];
    for (const file of Array.from(files)) {
      if (isSubtitleFileName(file.name)) {
        void readBrowserFileText(file).then((text) => {
          importSubtitleFromText(file.name, text);
        });
      } else {
        audioPaths.push(file.name);
      }
    }
    if (audioPaths.length > 0) {
      enqueueAudioFiles(audioPaths);
    }
    e.target.value = "";
  };

  const handleDrop = (e: React.DragEvent<HTMLElement>) => {
    e.preventDefault();
    setIsDragging(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length === 0) {
      return;
    }

    const audioPaths: string[] = [];
    for (const file of files) {
      const nativePath = (file as File & { path?: string }).path;
      const targetName = nativePath || file.name;
      if (!targetName) continue;

      if (isSubtitleFileName(targetName)) {
        if (isTauriRuntime() && nativePath) {
          void importSubtitleFromPath(nativePath);
        } else {
          void readBrowserFileText(file).then((text) => {
            importSubtitleFromText(file.name, text);
          });
        }
      } else {
        audioPaths.push(targetName);
      }
    }

    if (audioPaths.length > 0) {
      enqueueAudioFiles(audioPaths);
    }
  };

  const handleStopBatch = () => {
    void stopProcessingQueue();
  };

  const renderStatusIcon = (status: RowVisualStatus) => {
    if (status === "done") {
      return (
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
          <polyline points="22 4 12 14.01 9 11.01" />
        </svg>
      );
    }
    if (status === "processing") {
      return (
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.1"
          strokeLinecap="round"
        >
          <path d="M4 10v4" />
          <path d="M8 6v12" />
          <path d="M12 3v18" />
          <path d="M16 8v8" />
          <path d="M20 11v2" />
        </svg>
      );
    }
    if (status === "failed") {
      return (
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.1"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <circle cx="12" cy="12" r="9" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
      );
    }
    return (
      <svg
        width="13"
        height="13"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <circle cx="12" cy="12" r="9" />
        <polyline points="12 7 12 12 15 14" />
      </svg>
    );
  };

  const renderBadgeText = (item: BatchQueueItem, status: RowVisualStatus) => {
    if (status === "done") {
      return lang === "en" ? "Completed" : "Završeno";
    }
    if (status === "processing") {
      const pct = Math.round(item.percent);
      const phaseLabel =
        item.phase === "convertingAudio"
          ? lang === "en"
            ? "Audio"
            : "Audio"
          : item.phase === "transcribing"
            ? "Whisper"
            : item.phase === "translating"
              ? lang === "en"
                ? "Translating"
                : "Prevod"
              : item.phase === "polishing"
                ? lang === "en"
                  ? "Polishing"
                  : "Poliranje"
                : lang === "en"
                  ? "Processing"
                  : "Obrada";
      return `${phaseLabel} ${pct}%`;
    }
    if (status === "failed") {
      return lang === "en" ? "Failed" : "Greška";
    }
    if (status === "cancelled") {
      return lang === "en" ? "Cancelled" : "Prekinuto";
    }
    return lang === "en" ? "Queued" : "Na čekanju";
  };

  return (
    <section
      id="files-panel"
      aria-label={
        lang === "sr" ? "Lista ubačenih fajlova" : "Inserted files list"
      }
      className={isDragging ? "dragover" : undefined}
      onDragOver={(e) => {
        e.preventDefault();
        setIsDragging(true);
      }}
      onDragLeave={() => setIsDragging(false)}
      onDrop={handleDrop}
    >
      <header className="files-head">
        <div className="ec-head-left">
          <span className="files-head-icon" aria-hidden="true">
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
              <line x1="8" y1="13" x2="16" y2="13" />
              <line x1="8" y1="17" x2="13" y2="17" />
            </svg>
          </span>
          <span className="ec-title">
            {lang === "en" ? "Inserted Files" : "Ubačeni fajlovi"}
          </span>
          <span id="files-summary" className="files-summary-pill">
            {doneCount}/{displayRows.length}{" "}
            {lang === "en" ? "done" : "završeno"}
          </span>
        </div>

        <div className="files-head-actions">
          {isRunningBatch ? (
            <button
              id="btn-stop-batch"
              className="files-stop-btn"
              type="button"
              onClick={handleStopBatch}
              title={t.queue.cancel}
            >
              ■ {lang === "en" ? "Stop" : "Prekini"}
            </button>
          ) : (
            <button
              id="btn-start-batch"
              className="files-run-btn"
              type="button"
              disabled={displayRows.length === 0}
              onClick={() => startBatchPipeline("full")}
              title={t.workspace.oneClickPipeline}
            >
              ▶ {lang === "en" ? "Run" : "Pokreni"}
            </button>
          )}

          <button
            id="btn-add-file"
            className="files-add-btn"
            type="button"
            onClick={(e) => {
              const btn = e.currentTarget;
              btn.blur();
              void handleAddFilesClick().finally(() => {
                btn.blur();
              });
            }}
          >
            {lang === "en" ? "+ Add File" : "+ Dodaj fajl"}
          </button>

          <input
            ref={filePickerRef}
            id="file-picker"
            type="file"
            multiple
            accept=".mp3,.wav,.m4a,.flac,.ogg,.aac,.opus,.mp4,.mkv,.webm,.srt,.vtt"
            onChange={handleBrowserFileChange}
            hidden
          />
        </div>
      </header>

      <div id="files-list" className="files-list">
        {displayRows.length === 0 ? (
          <div
            className="files-empty"
            onClick={() => void handleAddFilesClick()}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                void handleAddFilesClick();
              }
            }}
          >
            <span>{t.workspace.dropTitle}</span>
            <span style={{ fontSize: "10.5px", color: "#5f6a7e" }}>
              MP3, WAV, M4A, FLAC, OGG, MP4, MKV, WEBM, .SRT, .VTT
            </span>
          </div>
        ) : (
          displayRows.map((item) => {
            const status = toRowStatus(item.phase);
            const isSelected =
              item.jobId === activeJobId ||
              (!activeJobId && activeProject?.jobId === item.jobId);
            const probe = probeResults[item.jobId];
            const format = getFileFormatLabel(item.audioFileName);
            const effectiveDurationMs =
              item.project && item.project.durationMs > 0
                ? item.project.durationMs
                : (probe?.durationMs ?? 0);
            const duration =
              effectiveDurationMs > 0
                ? formatDurationShort(effectiveDurationMs)
                : "--:--";
            const cuesCount = item.project?.cues.length ?? null;
            const metaParts = [format, duration];
            if (status === "done" && cuesCount !== null) {
              metaParts.push(`${cuesCount} ${t.queue.cuesCount}`);
            }
            const probeActive =
              probe &&
              (probe.stage === "acoustic" ||
                probe.stage === "asr" ||
                probe.stage === "topic");
            const probeTopic =
              probe?.topic?.trim() || item.project?.topicContext?.trim() || "";
            const probeLang =
              probe?.detectedLanguage?.trim() ||
              item.project?.detectedLanguage?.trim() ||
              "";
            const showProbePills = Boolean(
              probe &&
                probe.stage !== "cancelled" &&
                probe.stage !== "failed" &&
                (probeActive ||
                  probe.estimatedSpeakers !== null ||
                  probeLang ||
                  probeTopic),
            );

            return (
              <div
                key={item.jobId}
                data-file-id={item.jobId}
                className={`file-row ${status}${isSelected ? " selected" : ""}`}
                onClick={() => selectQueueItem(item.jobId)}
              >
                <div className="file-left">
                  <span className="file-status-icon">
                    {renderStatusIcon(status)}
                  </span>

                  <div className="file-info">
                    <div
                      className="file-name"
                      title={item.audioPath || item.audioFileName}
                    >
                      {item.audioFileName}
                    </div>
                    <div className="file-meta-line">
                      {metaParts.join(" • ")}
                    </div>

                    {showProbePills && (
                      <div className="file-probe-pills">
                        {probe?.estimatedSpeakers !== null &&
                          probe?.estimatedSpeakers !== undefined &&
                          probe.estimatedSpeakers > 0 && (
                            <span className="file-probe-pill speakers">
                              {probe.estimatedSpeakers === 1
                                ? t.probe.oneSpeaker
                                : `${probe.estimatedSpeakers} ${t.probe.speakers}`}
                            </span>
                          )}
                        {probeLang && (
                          <span className="file-probe-pill lang">
                            {probeLang.toUpperCase()}
                          </span>
                        )}
                        {probeTopic && (
                          <span
                            className="file-probe-pill topic"
                            title={`${t.probe.topicLabel}: ${probeTopic}`}
                          >
                            {probeTopic}
                          </span>
                        )}
                        {probeActive && (
                          <span className="file-probe-pill analyzing">
                            {t.probe.analyzing}
                          </span>
                        )}
                      </div>
                    )}

                    {status === "processing" && (
                      <div className="file-inline-progress">
                        <div
                          className="file-inline-progress-fill"
                          style={{
                            width: `${Math.min(100, Math.max(6, item.percent))}%`,
                          }}
                        />
                      </div>
                    )}

                    {item.errorMessage && (
                      <div
                        className="file-error-line"
                        title={item.errorMessage}
                      >
                        {item.errorMessage}
                      </div>
                    )}
                  </div>
                </div>

                <div className="file-right">
                  {status !== "done" && (
                    <span className={`file-badge ${status}`}>
                      {renderBadgeText(item, status)}
                    </span>
                  )}
                  <div className="file-hover-actions">
                    {item.project && item.project.cues.length > 0 && (
                      <button
                        type="button"
                        className="file-action-btn export"
                        title={
                          lang === "en" ? "Export subtitles" : "Izvoz titla"
                        }
                        aria-label={
                          lang === "en" ? "Export subtitles" : "Izvoz titla"
                        }
                        onClick={(e) => {
                          e.stopPropagation();
                          selectQueueItem(item.jobId);
                          setExportOpen(true);
                        }}
                      >
                        <Download size={11} />
                        <span>{lang === "en" ? "Export" : "Izvoz"}</span>
                      </button>
                    )}
                    <button
                      type="button"
                      className="file-action-btn reprocess"
                      disabled={isRunningBatch}
                      title={
                        lang === "en" ? "Re-process file" : "Ponovna obrada"
                      }
                      aria-label={
                        lang === "en" ? "Re-process file" : "Ponovna obrada"
                      }
                      onClick={(e) => {
                        e.stopPropagation();
                        reprocessQueueItem(item.jobId);
                      }}
                    >
                      <RotateCw size={11} />
                      <span>{lang === "en" ? "Reprocess" : "Ponovi"}</span>
                    </button>
                    <button
                      type="button"
                      className="file-remove-btn"
                      title={t.queue.remove}
                      onClick={(e) => {
                        e.stopPropagation();
                        removeQueueItem(item.jobId);
                      }}
                    >
                      ×
                    </button>
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}
