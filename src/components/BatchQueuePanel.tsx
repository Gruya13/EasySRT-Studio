import {
  AlertCircle,
  CheckCircle2,
  Clock,
  FileAudio,
  History,
  Loader2,
  Trash2,
  XCircle,
} from "lucide-react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatDurationShort } from "../lib/subtitleUtils";
import { useAppStore, type BatchQueueItem } from "../stores/appStore";
import type { JobPhase } from "../types/models";

export function BatchQueuePanel() {
  const config = useAppStore((s) => s.config);
  const batchQueue = useAppStore((s) => s.batchQueue);
  const activeJobId = useAppStore((s) => s.activeJobId);
  const checkpoints = useAppStore((s) => s.checkpoints);
  const isCheckpointsOpen = useAppStore((s) => s.isCheckpointsOpen);

  const selectQueueItem = useAppStore((s) => s.selectQueueItem);
  const removeQueueItem = useAppStore((s) => s.removeQueueItem);
  const clearFinishedQueueItems = useAppStore(
    (s) => s.clearFinishedQueueItems,
  );
  const cancelJobById = useAppStore((s) => s.cancelJobById);
  const setCheckpointsOpen = useAppStore((s) => s.setCheckpointsOpen);
  const loadCheckpointIntoEditor = useAppStore(
    (s) => s.loadCheckpointIntoEditor,
  );
  const removeCheckpoint = useAppStore((s) => s.removeCheckpoint);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const isRunningPhase = (phase: JobPhase) =>
    phase === "convertingAudio" ||
    phase === "transcribing" ||
    phase === "segmenting" ||
    phase === "translating" ||
    phase === "polishing";

  const getPhaseLabel = (item: BatchQueueItem) => {
    const base = t.queue.phases[item.phase] ?? item.phase;
    if (isRunningPhase(item.phase) && item.percent > 0) {
      return `${base} ${Math.round(item.percent)}%`;
    }
    return base;
  };

  const renderPhaseBadge = (item: BatchQueueItem) => {
    if (item.phase === "completed") {
      return (
        <span className="inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-300">
          <CheckCircle2 className="h-3 w-3" />
          {getPhaseLabel(item)}
        </span>
      );
    }
    if (item.phase === "failed") {
      return (
        <span className="inline-flex items-center gap-1 rounded-full border border-rose-500/30 bg-rose-500/15 px-2 py-0.5 text-[11px] font-medium text-rose-300">
          <AlertCircle className="h-3 w-3" />
          {getPhaseLabel(item)}
        </span>
      );
    }
    if (item.phase === "cancelled") {
      return (
        <span className="inline-flex items-center gap-1 rounded-full border border-slate-700 bg-slate-800 px-2 py-0.5 text-[11px] font-medium text-slate-400">
          <XCircle className="h-3 w-3" />
          {getPhaseLabel(item)}
        </span>
      );
    }
    if (isRunningPhase(item.phase)) {
      return (
        <span className="inline-flex items-center gap-1 rounded-full border border-cyan-500/30 bg-cyan-500/15 px-2 py-0.5 text-[11px] font-medium text-cyan-300">
          <Loader2 className="h-3 w-3 animate-spin" />
          {getPhaseLabel(item)}
        </span>
      );
    }
    return (
      <span className="inline-flex items-center gap-1 rounded-full border border-slate-700 bg-slate-800/80 px-2 py-0.5 text-[11px] font-medium text-slate-300">
        <Clock className="h-3 w-3" />
        {getPhaseLabel(item)}
      </span>
    );
  };

  return (
    <div className="space-y-3">
      {/* Checkpoints Recovery Drawer */}
      {isCheckpointsOpen && (
        <div className="rounded-2xl border border-cyan-500/30 bg-slate-900/90 p-4 shadow-lg">
          <div className="mb-3 flex items-center justify-between">
            <div className="flex items-center gap-2 text-xs font-bold uppercase tracking-wider text-cyan-300">
              <History className="h-4 w-4" />
              <span>{t.workspace.checkpointsTitle}</span>
            </div>
            <button
              type="button"
              onClick={() => setCheckpointsOpen(false)}
              className="text-xs text-slate-400 hover:text-slate-200"
            >
              ✕
            </button>
          </div>

          {checkpoints.length === 0 ? (
            <p className="text-xs text-slate-400">
              {t.workspace.noCheckpoints}
            </p>
          ) : (
            <div className="grid max-h-48 gap-2 overflow-y-auto pr-1 sm:grid-cols-2">
              {checkpoints.map((cp) => (
                <div
                  key={cp.jobId}
                  className="flex items-center justify-between gap-2 rounded-xl border border-slate-800 bg-slate-950/80 p-2.5"
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs font-semibold text-slate-200">
                      {cp.audioFileName}
                    </div>
                    <div className="mt-0.5 flex items-center gap-2 font-mono text-[10px] text-slate-400">
                      <span>
                        {cp.cues.length} {t.queue.cuesCount}
                      </span>
                      <span>•</span>
                      <span>{formatDurationShort(cp.durationMs)}</span>
                      <span>•</span>
                      <span>
                        {cp.options.sourceLanguage} →{" "}
                        {cp.options.targetLanguage}
                      </span>
                    </div>
                  </div>

                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => void loadCheckpointIntoEditor(cp.jobId)}
                      className="rounded-lg border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-1 text-[11px] font-medium text-emerald-300 transition hover:bg-emerald-500/25"
                    >
                      {t.workspace.loadCheckpoint}
                    </button>
                    <button
                      type="button"
                      onClick={() => void removeCheckpoint(cp.jobId)}
                      className="rounded-lg p-1 text-slate-500 transition hover:bg-rose-500/15 hover:text-rose-300"
                      title={t.workspace.deleteCheckpoint}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Batch Queue List */}
      <div className="rounded-2xl border border-slate-800/90 bg-slate-900/60 p-3.5">
        <div className="mb-2.5 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-xs font-bold uppercase tracking-wider text-slate-300">
              {t.queue.title}
            </span>
            <span className="rounded-full bg-slate-800 px-2 py-0.5 font-mono text-[11px] text-slate-300">
              {batchQueue.length}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setCheckpointsOpen(!isCheckpointsOpen)}
              className="flex items-center gap-1 text-[11px] font-medium text-cyan-400 hover:text-cyan-300"
            >
              <History className="h-3.5 w-3.5" />
              <span>
                {t.workspace.checkpointsTitle} ({checkpoints.length})
              </span>
            </button>

            {batchQueue.some(
              (q) =>
                q.phase === "completed" ||
                q.phase === "cancelled" ||
                q.phase === "failed",
            ) && (
              <button
                type="button"
                onClick={clearFinishedQueueItems}
                className="text-[11px] font-medium text-slate-400 hover:text-slate-200"
              >
                {t.queue.clearFinished}
              </button>
            )}
          </div>
        </div>

        {batchQueue.length === 0 ? (
          <div className="rounded-xl border border-dashed border-slate-800 px-4 py-3 text-center text-xs text-slate-500">
            {t.queue.empty}
          </div>
        ) : (
          <div className="grid max-h-44 gap-2 overflow-y-auto pr-1 sm:grid-cols-2 lg:grid-cols-3">
            {batchQueue.map((item) => {
              const isSelected = item.jobId === activeJobId;
              const running = isRunningPhase(item.phase);

              return (
                <div
                  key={item.jobId}
                  onClick={() => selectQueueItem(item.jobId)}
                  className={`group relative flex cursor-pointer flex-col justify-between gap-2 rounded-xl border p-2.5 transition ${
                    isSelected
                      ? "border-emerald-500/50 bg-emerald-950/20 shadow-sm"
                      : "border-slate-800 bg-slate-950/70 hover:border-slate-700"
                  }`}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex min-w-0 items-center gap-2">
                      <FileAudio
                        className={`h-4 w-4 shrink-0 ${
                          isSelected ? "text-emerald-400" : "text-slate-400"
                        }`}
                      />
                      <span
                        className="truncate text-xs font-semibold text-slate-200"
                        title={item.audioPath}
                      >
                        {item.audioFileName}
                      </span>
                    </div>

                    {running ? (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          void cancelJobById(item.jobId);
                        }}
                        className="rounded-md border border-rose-500/30 bg-rose-500/15 px-1.5 py-0.5 text-[10px] font-medium text-rose-300 hover:bg-rose-500/25"
                      >
                        {t.queue.cancel}
                      </button>
                    ) : (
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          removeQueueItem(item.jobId);
                        }}
                        className="rounded-md p-0.5 text-slate-500 opacity-70 transition hover:bg-slate-800 hover:text-slate-200 group-hover:opacity-100"
                        title={t.queue.remove}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </div>

                  {running && (
                    <div className="h-1 w-full overflow-hidden rounded-full bg-slate-800">
                      <div
                        className="h-full rounded-full bg-gradient-to-r from-emerald-400 to-cyan-400 transition-all duration-200"
                        style={{
                          width: `${Math.min(100, Math.max(8, item.percent))}%`,
                        }}
                      />
                    </div>
                  )}

                  <div className="flex items-center justify-between gap-2">
                    {renderPhaseBadge(item)}
                    {item.project && (
                      <span className="font-mono text-[10px] text-slate-400">
                        {item.project.cues.length} {t.queue.cuesCount}
                      </span>
                    )}
                  </div>

                  {item.errorMessage && (
                    <div className="truncate text-[10px] text-rose-400">
                      {item.errorMessage}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
