import type { SubtitleCue, SubtitleProject } from "../types/models.ts";

export interface ModalFlagsSnapshot {
  isSettingsOpen: boolean;
  isExportOpen: boolean;
  isSetupOpen: boolean;
  isUpdaterOpen: boolean;
  isCheckpointsOpen: boolean;
  isBugReportOpen: boolean;
  onlineConfirm: { open: boolean };
}

export function isAnyModalOpen(st: ModalFlagsSnapshot): boolean {
  return (
    st.isSettingsOpen ||
    st.isExportOpen ||
    st.isSetupOpen ||
    st.isUpdaterOpen ||
    st.isCheckpointsOpen ||
    st.isBugReportOpen ||
    st.onlineConfirm.open
  );
}

const TEXT_INPUT_TYPES = new Set([
  "",
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "number",
  "date",
  "time",
  "datetime-local",
  "month",
  "week",
]);

export interface TextEntryElementLike {
  tagName?: string;
  type?: string;
  isContentEditable?: boolean;
  getAttribute?: (name: string) => string | null;
}

/**
 * Returns true ONLY for actual text-typing fields (`textarea`, `select`, `contenteditable`,
 * or text/numeric/search `<input>`), and NEVER `<button>`, `<input type="checkbox">`,
 * `<input type="range">`, `<input type="radio">`, or `<input type="file">`.
 */
export function isTextEntryElement(target: unknown): boolean {
  if (!target || typeof target !== "object") {
    return false;
  }
  const el = target as TextEntryElementLike;
  const tag = (el.tagName ?? "").toUpperCase();
  if (tag === "TEXTAREA" || tag === "SELECT") {
    return true;
  }
  if (el.isContentEditable) {
    return true;
  }
  const ceAttr =
    typeof el.getAttribute === "function"
      ? el.getAttribute("contenteditable")
      : null;
  if (ceAttr === "true" || ceAttr === "") {
    return true;
  }
  if (tag === "INPUT") {
    const inputType = (el.type ?? "text").trim().toLowerCase();
    return TEXT_INPUT_TYPES.has(inputType);
  }
  return false;
}

/**
 * Returns whether global timeline `Space` / `Shift+Space` capture should hijack
 * the event (blocking button/checkbox focus activation even when empty).
 */
export function shouldCaptureTimelineSpace(input: {
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  target: unknown;
  modalState: ModalFlagsSnapshot;
}): boolean {
  if (input.code !== "Space") return false;
  if (input.ctrlKey || input.metaKey || input.altKey) return false;
  if (isAnyModalOpen(input.modalState)) return false;
  if (isTextEntryElement(input.target)) return false;
  return true;
}

export function findCueForTime(
  cueList: SubtitleCue[],
  ms: number,
  allowNearestPreceding: boolean,
): SubtitleCue | null {
  if (cueList.length === 0) return null;
  const activeHalfOpen = cueList.find((c) => ms >= c.startMs && ms < c.endMs);
  if (activeHalfOpen) return activeHalfOpen;
  const exact = cueList.find((c) => ms >= c.startMs && ms <= c.endMs);
  if (exact) return exact;
  if (!allowNearestPreceding) return null;
  let best: SubtitleCue | null = null;
  for (const cue of cueList) {
    if (cue.startMs <= ms && (!best || cue.startMs >= best.startMs)) {
      best = cue;
    }
  }
  return best ?? cueList[0];
}

/**
 * Computes next `scrollLeft` when `Follow Playhead` is active during playback:
 * returns `null` when playhead stays within `[scrollLeft, scrollLeft + 0.85 * viewportWidth]`,
 * otherwise scrolls so playhead sits near 15% from the left edge of the viewport.
 */
export function computeFollowPlayheadScrollLeft(input: {
  playheadMs: number;
  totalMs: number;
  contentWidthPx: number;
  viewportWidthPx: number;
  currentScrollLeft: number;
}): number | null {
  const {
    playheadMs,
    totalMs,
    contentWidthPx,
    viewportWidthPx,
    currentScrollLeft,
  } = input;
  if (totalMs <= 0 || contentWidthPx <= viewportWidthPx || viewportWidthPx <= 0) {
    return null;
  }
  const playheadX = Math.max(
    0,
    Math.min(contentWidthPx, (playheadMs / totalMs) * contentWidthPx),
  );
  const rightThreshold = currentScrollLeft + viewportWidthPx * 0.85;
  if (playheadX >= currentScrollLeft && playheadX <= rightThreshold) {
    return null;
  }
  const maxScrollLeft = Math.max(0, contentWidthPx - viewportWidthPx);
  return Math.max(
    0,
    Math.min(maxScrollLeft, Math.round(playheadX - viewportWidthPx * 0.15)),
  );
}

const MEDIA_SESSION_ACTIONS = [
  "play",
  "pause",
  "stop",
  "seekbackward",
  "seekforward",
  "previoustrack",
  "nexttrack",
  "seekto",
] as const;

export interface MediaSessionLike {
  playbackState?: string;
  metadata?: unknown;
  setActionHandler?: (action: string, handler: unknown) => void;
}

/**
 * Neutralizes `navigator.mediaSession` (`playbackState = "none"`, `metadata = null`,
 * and `null` action handlers) so external OS events (`playerctl`, headset buttons,
 * voice dictation resume hooks) cannot hijack idle/paused audio (`V05-L1`).
 */
export function clearMediaSession(
  mediaSession?: MediaSessionLike | null,
): void {
  const target =
    mediaSession !== undefined
      ? mediaSession
      : typeof navigator !== "undefined" && "mediaSession" in navigator
        ? (navigator.mediaSession as unknown as MediaSessionLike)
        : null;
  if (!target) return;

  try {
    target.playbackState = "none";
  } catch {
    // Ignore read-only environments
  }
  try {
    target.metadata = null;
  } catch {
    // Ignore read-only environments
  }
  if (typeof target.setActionHandler === "function") {
    for (const action of MEDIA_SESSION_ACTIONS) {
      try {
        target.setActionHandler(action, null);
      } catch {
        // Some WebKitGTK builds throw on unsupported actions like seekto
      }
    }
  }
}

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

/**
 * Infers an audio MIME type from magic header bytes (falling back to file extension)
 * so FFmpeg-transcoded WAV containers (e.g., from `.mp4`/`.mkv`/`.webm`) always get `"audio/wav"`.
 */
export function inferAudioMimeType(
  filePath: string,
  bytes?: ArrayBuffer | Uint8Array,
): string {
  if (bytes) {
    const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    if (
      view.length >= 12 &&
      view[0] === 0x52 && // R
      view[1] === 0x49 && // I
      view[2] === 0x46 && // F
      view[3] === 0x46 && // F
      view[8] === 0x57 && // W
      view[9] === 0x41 && // A
      view[10] === 0x56 && // V
      view[11] === 0x45 // E
    ) {
      return "audio/wav";
    }
    if (
      view.length >= 4 &&
      ((view[0] === 0x49 && view[1] === 0x44 && view[2] === 0x33) || // ID3
        (view[0] === 0xff && (view[1] & 0xe0) === 0xe0))
    ) {
      return "audio/mpeg";
    }
    if (
      view.length >= 4 &&
      view[0] === 0x4f && // O
      view[1] === 0x67 && // g
      view[2] === 0x67 && // g
      view[3] === 0x53 // S
    ) {
      return "audio/ogg";
    }
    if (
      view.length >= 4 &&
      view[0] === 0x66 && // f
      view[1] === 0x4c && // L
      view[2] === 0x61 && // a
      view[3] === 0x43 // C
    ) {
      return "audio/flac";
    }
  }

  const extMatch = filePath.trim().toLowerCase().match(/\.([a-z0-9]+)$/);
  const ext = extMatch ? extMatch[1] : "";
  switch (ext) {
    case "wav":
      return "audio/wav";
    case "mp3":
      return "audio/mpeg";
    case "ogg":
    case "opus":
      return "audio/ogg";
    case "flac":
      return "audio/flac";
    case "m4a":
    case "mp4":
      return "audio/mp4";
    case "aac":
      return "audio/aac";
    case "webm":
      return "audio/webm";
    default:
      return "audio/mpeg";
  }
}
