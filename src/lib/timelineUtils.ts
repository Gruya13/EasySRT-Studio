import type { SubtitleProject } from "../types/models.ts";

export const MIN_TIMELINE_ZOOM_PCT = 100;
export const MAX_TIMELINE_ZOOM_PCT = 2000;
export const MAX_CANVAS_BACKING_WIDTH = 16384;

export interface TimelineMediaState {
  hasLoadedProjectOrFile: boolean;
  hasAudioSource: boolean;
  audioPath: string | null;
  audioFileName: string | null;
  isSubtitleOnly: boolean;
}

export function isSubtitleOnlyPath(path?: string | null): boolean {
  const trimmed = (path ?? "").trim();
  return trimmed.length > 0 && /\.(srt|vtt)$/i.test(trimmed);
}

function extractFileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * Pure media-presence resolver for `WaveformTimeline` (Workstream 6).
 * Distinguishes:
 * 1. Empty startup (`activeProject === null && selectedQueueItem === null`)
 * 2. Queued audio/video file before transcription
 * 3. Completed audio+subtitle project
 * 4. Standalone imported `.srt` / `.vtt` file without companion audio
 */
export function resolveTimelineMediaState(input: {
  activeProject: Pick<
    SubtitleProject,
    "audioPath" | "audioFileName" | "cues"
  > | null;
  selectedQueueItem: {
    audioPath: string;
    audioFileName: string;
  } | null;
}): TimelineMediaState {
  const { activeProject, selectedQueueItem } = input;
  const hasLoadedProjectOrFile = Boolean(activeProject || selectedQueueItem);

  if (!hasLoadedProjectOrFile) {
    return {
      hasLoadedProjectOrFile: false,
      hasAudioSource: false,
      audioPath: null,
      audioFileName: null,
      isSubtitleOnly: false,
    };
  }

  const rawPath = (
    activeProject?.audioPath?.trim() ||
    selectedQueueItem?.audioPath?.trim() ||
    ""
  ).trim();
  const rawName = (
    activeProject?.audioFileName?.trim() ||
    selectedQueueItem?.audioFileName?.trim() ||
    ""
  ).trim();

  const subtitleOnly =
    !rawPath ||
    isSubtitleOnlyPath(rawPath) ||
    (!rawPath && isSubtitleOnlyPath(rawName));

  if (subtitleOnly) {
    return {
      hasLoadedProjectOrFile: true,
      hasAudioSource: false,
      audioPath: null,
      audioFileName: null,
      isSubtitleOnly: true,
    };
  }

  return {
    hasLoadedProjectOrFile: true,
    hasAudioSource: true,
    audioPath: rawPath,
    audioFileName: rawName || extractFileName(rawPath),
    isSubtitleOnly: false,
  };
}

/** Compute next timeline zoom percentage from a mouse wheel `deltaY` scroll direction. */
export function computeWheelZoomPct(
  currentZoomPct: number,
  deltaY: number,
): number {
  const clampedCurrent = Math.min(
    MAX_TIMELINE_ZOOM_PCT,
    Math.max(MIN_TIMELINE_ZOOM_PCT, currentZoomPct),
  );
  if (!Number.isFinite(deltaY) || deltaY === 0) {
    return clampedCurrent;
  }
  const step = Math.max(25, Math.round((clampedCurrent * 0.15) / 25) * 25);
  if (deltaY < 0) {
    return Math.min(MAX_TIMELINE_ZOOM_PCT, clampedCurrent + step);
  }
  return Math.max(MIN_TIMELINE_ZOOM_PCT, clampedCurrent - step);
}

/** Compute next timeline zoom percentage from the `-` / `+` toolbar zoom buttons. */
export function computeButtonZoomPct(
  currentZoomPct: number,
  direction: "in" | "out",
): number {
  const clampedCurrent = Math.min(
    MAX_TIMELINE_ZOOM_PCT,
    Math.max(MIN_TIMELINE_ZOOM_PCT, currentZoomPct),
  );
  if (direction === "in") {
    const step =
      clampedCurrent >= 1000 ? 100 : clampedCurrent >= 400 ? 50 : 25;
    return Math.min(MAX_TIMELINE_ZOOM_PCT, clampedCurrent + step);
  }
  const step = clampedCurrent > 1000 ? 100 : clampedCurrent > 400 ? 50 : 25;
  return Math.max(MIN_TIMELINE_ZOOM_PCT, clampedCurrent - step);
}

/**
 * Clamp HTML5 `<canvas>` backing store dimensions to the browser/WebView limit (`16384px`)
 * and compute exact scale factors so HiDPI waveforms stay sharp without blanking at 2000% zoom.
 */
export function computeSafeCanvasSize(
  cssWidth: number,
  cssHeight: number,
  dpr: number,
): {
  backingWidth: number;
  backingHeight: number;
  scaleX: number;
  scaleY: number;
} {
  const safeWidth = Math.max(1, Math.floor(cssWidth));
  const safeHeight = Math.max(1, Math.floor(cssHeight));
  const safeDpr = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;

  const backingWidth = Math.min(
    MAX_CANVAS_BACKING_WIDTH,
    Math.max(1, Math.round(safeWidth * safeDpr)),
  );
  const backingHeight = Math.max(1, Math.round(safeHeight * safeDpr));

  return {
    backingWidth,
    backingHeight,
    scaleX: backingWidth / safeWidth,
    scaleY: backingHeight / safeHeight,
  };
}

/** Format timeline ruler timestamps with tenths (`MM:SS.d`) when zoomed into sub-second intervals. */
export function formatRulerTimestamp(ms: number): string {
  const safeMs = Math.max(0, Math.round(ms));
  const tenths = Math.floor((safeMs % 1000) / 100);
  const totalSec = Math.floor(safeMs / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  const base =
    h > 0
      ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
      : `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return tenths > 0 ? `${base}.${tenths}` : base;
}
