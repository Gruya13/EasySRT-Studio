import { convertFileSrc } from "@tauri-apps/api/core";
import { Merge, Plus, Redo2, Scissors, Trash2, Undo2 } from "lucide-react";
import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime } from "../lib/subtitleUtils";
import { prepareAudioWaveform, readAudioPlaybackBytes } from "../lib/tauri";
import {
  MAX_TIMELINE_ZOOM_PCT,
  MIN_TIMELINE_ZOOM_PCT,
  computeButtonZoomPct,
  computeSafeCanvasSize,
  computeWheelZoomPct,
  formatRulerTimestamp,
  inferAudioMimeType,
  resolveTimelineMediaState,
} from "../lib/timelineUtils";
import { useAppStore } from "../stores/appStore";
import type { JobPhase, SubtitleCue } from "../types/models";

const PROCESSING_PHASES = new Set<JobPhase>([
  "queued",
  "convertingAudio",
  "transcribing",
  "segmenting",
  "translating",
  "polishing",
]);

const FRAME_STEP_MS = 40; // 1 frame @ 25fps
const LARGE_JUMP_MS = 1000; // 1 second jump
const MIN_SPLIT_MARGIN_MS = 80;
const WAVEFORM_TARGET_BUCKETS = 2000;

interface TrackState {
  mp3: { muted: boolean; locked: boolean };
  original: { hidden: boolean; locked: boolean };
  prevod: { hidden: boolean; locked: boolean };
}

interface TrimDragState {
  cueId: number;
  edge: "start" | "end";
  startClientX: number;
  origStartMs: number;
  origEndMs: number;
  contentWidthPx: number;
  previewStartMs: number;
  previewEndMs: number;
}

function isEditableElement(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    target.isContentEditable
  );
}

function formatTimecode(ms: number, showHours: boolean): string {
  const safe = Math.max(0, Math.round(ms));
  const hours = Math.floor(safe / 3_600_000);
  const mins = Math.floor((safe % 3_600_000) / 60_000);
  const secs = Math.floor((safe % 60_000) / 1000);
  const cs = Math.floor((safe % 1000) / 10);
  const hh = showHours ? String(hours).padStart(2, "0") : "00";
  return `${hh}:${String(mins).padStart(2, "0")}:${String(secs).padStart(
    2,
    "0",
  )}.${String(cs).padStart(2, "0")}`;
}

function formatRulerSec(ms: number): string {
  const totalSec = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function findCueForTime(
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

function buildDemoWavBuffer(
  durationMs: number,
  cues: SubtitleCue[],
): ArrayBuffer {
  const sampleRate = 16000;
  const clampedMs = Math.max(2000, Math.min(durationMs, 120000));
  const numSamples = Math.floor((clampedMs / 1000) * sampleRate);
  const dataSize = numSamples * 2;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  const writeAscii = (offset: number, text: string) => {
    for (let i = 0; i < text.length; i += 1) {
      view.setUint8(offset + i, text.charCodeAt(i));
    }
  };

  writeAscii(0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(8, "WAVE");
  writeAscii(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  writeAscii(36, "data");
  view.setUint32(40, dataSize, true);

  for (let i = 0; i < numSamples; i += 1) {
    const tSec = i / sampleRate;
    const tMs = tSec * 1000;
    const active = cues.find((c) => tMs >= c.startMs && tMs <= c.endMs);
    let sample = 0;
    if (active) {
      const spanSec = Math.max(0.1, (active.endMs - active.startMs) / 1000);
      const posSec = tSec - active.startMs / 1000;
      const env = Math.sin(
        Math.min(1, Math.min(posSec * 8, (spanSec - posSec) * 8)) *
          (Math.PI / 2),
      );
      const baseHz = 185 + ((active.id * 29) % 75);
      const wave =
        0.55 * Math.sin(2 * Math.PI * baseHz * tSec) +
        0.3 * Math.sin(2 * Math.PI * baseHz * 1.498 * tSec);
      sample = Math.round(wave * env * 0.14 * 32767);
    }
    view.setInt16(44 + i * 2, sample, true);
  }
  return buffer;
}

export function WaveformTimeline() {
  const config = useAppStore((s) => s.config);
  const batchQueue = useAppStore((s) => s.batchQueue);
  const activeJobId = useAppStore((s) => s.activeJobId);
  const activeProject = useAppStore((s) => s.activeProject);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const playbackRequest = useAppStore((s) => s.playbackRequest);
  const canUndo = useAppStore((s) => s.canUndo);
  const canRedo = useAppStore((s) => s.canRedo);

  const setActiveCueId = useAppStore((s) => s.setActiveCueId);
  const playCueSegment = useAppStore((s) => s.playCueSegment);
  const updateCue = useAppStore((s) => s.updateCue);
  const splitCueAtMidpoint = useAppStore((s) => s.splitCueAtMidpoint);
  const mergeCueWithNext = useAppStore((s) => s.mergeCueWithNext);
  const deleteCueById = useAppStore((s) => s.deleteCueById);
  const addCueAtEnd = useAppStore((s) => s.addCueAtEnd);
  const undoCueEdit = useAppStore((s) => s.undoCueEdit);
  const redoCueEdit = useAppStore((s) => s.redoCueEdit);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const selectedQueueItem = useMemo(
    () =>
      activeJobId
        ? (batchQueue.find((q) => q.jobId === activeJobId) ?? null)
        : null,
    [batchQueue, activeJobId],
  );

  const mediaState = useMemo(
    () => resolveTimelineMediaState({ activeProject, selectedQueueItem }),
    [activeProject, selectedQueueItem],
  );
  const { hasLoadedProjectOrFile, hasAudioSource, audioPath, audioFileName } =
    mediaState;
  const hasLoadedRef = useRef(hasLoadedProjectOrFile);
  hasLoadedRef.current = hasLoadedProjectOrFile;

  const [zoomPct, setZoomPct] = useState(MIN_TIMELINE_ZOOM_PCT);
  const zoomPctRef = useRef(zoomPct);
  zoomPctRef.current = zoomPct;
  const renderedZoomPctRef = useRef(zoomPct);
  const pendingZoomAnchorRef = useRef<{
    timeRatio: number;
    cursorX: number;
  } | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const isPlayingRef = useRef(isPlaying);
  isPlayingRef.current = isPlaying;
  const [playheadMs, setPlayheadMs] = useState(0);
  const [audioSrcUrl, setAudioSrcUrl] = useState<string>("");
  const [liveAudioDurationMs, setLiveAudioDurationMs] = useState(0);
  const [waveformVersion, setWaveformVersion] = useState(0);
  const [viewportWidth, setViewportWidth] = useState(1000);
  const [trimDrag, setTrimDrag] = useState<TrimDragState | null>(null);
  const [tracks, setTracks] = useState<TrackState>({
    mp3: { muted: false, locked: false },
    original: { hidden: false, locked: false },
    prevod: { hidden: false, locked: false },
  });

  const audioRef = useRef<HTMLAudioElement | null>(null);
  const audioPeaksRef = useRef<Float32Array | null>(null);
  const webAudioCtxRef = useRef<AudioContext | null>(null);
  const webAudioBufferRef = useRef<AudioBuffer | null>(null);
  const webAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const webAudioGainRef = useRef<GainNode | null>(null);
  const loadDecodePromiseRef = useRef<Promise<AudioBuffer | null> | null>(null);
  const webAudioStartCtxTimeRef = useRef(0);
  const webAudioStartOffsetSecRef = useRef(0);
  const webAudioRequestIdRef = useRef(0);

  const viewportRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const rulerRef = useRef<HTMLDivElement | null>(null);
  const mp3ClipRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  const playheadRef = useRef(0);
  const segmentEndMsRef = useRef<number | null>(null);
  const scrubbingRef = useRef(false);
  const rafIdRef = useRef<number | null>(null);
  const lastRafTickRef = useRef<number>(0);
  const lastResetJobIdRef = useRef<string | null>(null);
  const lastResetPhaseRef = useRef<JobPhase | null>(null);
  const lastResetCueCountRef = useRef<number>(0);
  const lastHandledPlaybackRequestRef = useRef<typeof playbackRequest>(null);

  const cues = activeProject?.cues ?? [];
  const cuesRef = useRef(cues);
  cuesRef.current = cues;
  const activeCueIdRef = useRef(activeCueId);
  activeCueIdRef.current = activeCueId;
  const tracksRef = useRef(tracks);
  tracksRef.current = tracks;

  const maxCueEndMs = useMemo(
    () => cues.reduce((max, c) => Math.max(max, c.endMs), 0),
    [cues],
  );
  const actualAudioDurationMs = useMemo(() => {
    if (!hasLoadedProjectOrFile) return 0;
    if (liveAudioDurationMs > 0) return liveAudioDurationMs;
    return Math.max(activeProject?.durationMs ?? 0, maxCueEndMs, 1000);
  }, [
    hasLoadedProjectOrFile,
    liveAudioDurationMs,
    activeProject?.durationMs,
    maxCueEndMs,
  ]);
  const audioEndMs = useMemo(() => {
    if (!hasLoadedProjectOrFile) return 0;
    return Math.max(
      activeProject?.durationMs ?? 0,
      liveAudioDurationMs,
      maxCueEndMs,
      1000,
    );
  }, [
    hasLoadedProjectOrFile,
    activeProject?.durationMs,
    liveAudioDurationMs,
    maxCueEndMs,
  ]);
  const totalMs = useMemo(() => {
    if (!hasLoadedProjectOrFile) return 0;
    return Math.max(Math.round(audioEndMs * 1.04), maxCueEndMs + 500, 1000);
  }, [hasLoadedProjectOrFile, audioEndMs, maxCueEndMs]);

  const audioEndMsRef = useRef(audioEndMs);
  audioEndMsRef.current = audioEndMs;

  const activeCue = useMemo(() => {
    if (cues.length === 0 || activeCueId === null) return null;
    return cues.find((c) => c.id === activeCueId) ?? null;
  }, [cues, activeCueId]);

  const ensureAudioContext = useCallback((): AudioContext | null => {
    if (typeof AudioContext === "undefined") {
      return null;
    }
    if (!webAudioCtxRef.current || webAudioCtxRef.current.state === "closed") {
      webAudioCtxRef.current = new AudioContext();
    }
    return webAudioCtxRef.current;
  }, []);

  const stopWebAudioSource = useCallback((invalidatePending = true) => {
    if (invalidatePending) {
      webAudioRequestIdRef.current += 1;
    }
    const src = webAudioSourceRef.current;
    if (src) {
      src.onended = null;
      try {
        src.stop();
      } catch {
        // Ignore stop errors if already ended
      }
      try {
        src.disconnect();
      } catch {
        // Ignore disconnect errors
      }
      webAudioSourceRef.current = null;
    }
    if (webAudioGainRef.current) {
      try {
        webAudioGainRef.current.disconnect();
      } catch {
        // Ignore disconnect errors
      }
      webAudioGainRef.current = null;
    }
  }, []);

  const startWebAudioAtOffset = useCallback(
    async (offsetSec: number): Promise<boolean> => {
      const reqId = ++webAudioRequestIdRef.current;
      let buf = webAudioBufferRef.current;
      if (!buf && loadDecodePromiseRef.current) {
        buf = await loadDecodePromiseRef.current;
        if (reqId !== webAudioRequestIdRef.current || !isPlayingRef.current) {
          return false;
        }
      }
      if (!buf || buf.duration <= 0) {
        return false;
      }

      const ctx = ensureAudioContext();
      if (!ctx) {
        return false;
      }
      if (ctx.state === "suspended") {
        try {
          await ctx.resume();
        } catch {
          // Proceed and attempt playback
        }
        if (reqId !== webAudioRequestIdRef.current || !isPlayingRef.current) {
          return false;
        }
      }

      stopWebAudioSource(false);
      const source = ctx.createBufferSource();
      source.buffer = buf;
      const gain = ctx.createGain();
      gain.gain.value = tracksRef.current.mp3.muted ? 0 : 1;
      source.connect(gain);
      gain.connect(ctx.destination);

      const safeOffset = Math.max(
        0,
        Math.min(Math.max(0, buf.duration - 0.005), offsetSec),
      );
      webAudioSourceRef.current = source;
      webAudioGainRef.current = gain;
      webAudioStartCtxTimeRef.current = ctx.currentTime;
      webAudioStartOffsetSecRef.current = safeOffset;
      source.onended = () => {
        if (webAudioSourceRef.current === source) {
          webAudioSourceRef.current = null;
        }
      };
      source.start(0, safeOffset);
      return true;
    },
    [ensureAudioContext, stopWebAudioSource],
  );

  // BUG-M2: Ensure any AudioContext instance and active source are closed on unmount
  useEffect(() => {
    return () => {
      stopWebAudioSource();
      if (webAudioCtxRef.current) {
        void webAudioCtxRef.current.close().catch(() => {});
        webAudioCtxRef.current = null;
      }
    };
  }, [stopWebAudioSource]);

  // Workstream 6 + SEC-H1: Load lightweight Rust waveform peaks + zero-Base64 binary Web Audio / blob playback
  useEffect(() => {
    let active = true;
    let createdBlobUrl = "";

    stopWebAudioSource();
    webAudioBufferRef.current = null;
    loadDecodePromiseRef.current = null;
    audioPeaksRef.current = null;
    setLiveAudioDurationMs(0);
    setAudioSrcUrl("");
    setWaveformVersion((v) => v + 1);

    if (!hasAudioSource || !audioPath) {
      return;
    }

    const decodeIntoWebAudio = async (
      rawBuf: ArrayBuffer,
    ): Promise<AudioBuffer | null> => {
      const ctx = ensureAudioContext();
      if (!ctx) return null;
      try {
        const decoded = await ctx.decodeAudioData(rawBuf.slice(0));
        if (!active) return null;
        webAudioBufferRef.current = decoded;
        if (decoded.duration > 0) {
          const decodedMs = Math.round(decoded.duration * 1000);
          setLiveAudioDurationMs((prev) => (prev > 0 ? prev : decodedMs));
        }
        return decoded;
      } catch (err) {
        console.warn("[WaveformTimeline] Web Audio decodeAudioData failed", err);
        return null;
      }
    };

    const loadPromise = (async (): Promise<AudioBuffer | null> => {
      try {
        const preview = await prepareAudioWaveform(
          audioPath,
          WAVEFORM_TARGET_BUCKETS,
        );
        if (!active) return null;

        audioPeaksRef.current = Float32Array.from(preview.peaks);
        if (preview.durationMs > 0) {
          setLiveAudioDurationMs(preview.durationMs);
        }
        setWaveformVersion((v) => v + 1);

        if (isTauriRuntime()) {
          try {
            const rawBuf = await readAudioPlaybackBytes(preview.canonicalPath);
            if (!active) return null;
            const mimeType = inferAudioMimeType(preview.canonicalPath, rawBuf);
            const blob = new Blob([rawBuf], { type: mimeType });
            createdBlobUrl = URL.createObjectURL(blob);
            setAudioSrcUrl(createdBlobUrl);
            return await decodeIntoWebAudio(rawBuf);
          } catch (err) {
            if (!active) return null;
            console.warn(
              "[WaveformTimeline] Binary playback fallback triggered",
              err,
            );
            const assetUrl = convertFileSrc(preview.canonicalPath);
            if (!assetUrl.startsWith("asset://")) {
              setAudioSrcUrl(assetUrl);
            }
            return null;
          }
        }

        const rawBuf = buildDemoWavBuffer(
          preview.durationMs || audioEndMsRef.current || 24000,
          cuesRef.current,
        );
        const blob = new Blob([rawBuf], { type: "audio/wav" });
        createdBlobUrl = URL.createObjectURL(blob);
        setAudioSrcUrl(createdBlobUrl);
        return await decodeIntoWebAudio(rawBuf);
      } catch (err) {
        if (!active) return null;
        console.warn(
          "[WaveformTimeline] Waveform/audio preparation failed",
          err,
        );
        if (!isTauriRuntime()) {
          const rawBuf = buildDemoWavBuffer(
            audioEndMsRef.current || 24000,
            cuesRef.current,
          );
          const blob = new Blob([rawBuf], { type: "audio/wav" });
          createdBlobUrl = URL.createObjectURL(blob);
          setAudioSrcUrl(createdBlobUrl);
          return await decodeIntoWebAudio(rawBuf);
        }
        return null;
      }
    })();

    loadDecodePromiseRef.current = loadPromise;

    return () => {
      active = false;
      stopWebAudioSource();
      if (createdBlobUrl) {
        URL.revokeObjectURL(createdBlobUrl);
      }
    };
  }, [hasAudioSource, audioPath, ensureAudioContext, stopWebAudioSource]);

  // Sync mute state onto both <audio> and Web Audio GainNode
  useEffect(() => {
    if (audioRef.current) {
      audioRef.current.muted = tracks.mp3.muted;
    }
    if (webAudioGainRef.current) {
      webAudioGainRef.current.gain.value = tracks.mp3.muted ? 0 : 1;
    }
  }, [tracks.mp3.muted]);

  const seekPlayhead = useCallback(
    (ms: number, syncCue = true) => {
      if (!hasLoadedRef.current) {
        playheadRef.current = 0;
        setPlayheadMs(0);
        return;
      }
      const clamped = Math.max(0, Math.min(audioEndMsRef.current, ms));
      playheadRef.current = clamped;
      setPlayheadMs(clamped);

      if (syncCue) {
        const matched = findCueForTime(cuesRef.current, clamped, false);
        const nextId = matched ? matched.id : null;
        if (nextId !== activeCueIdRef.current) {
          activeCueIdRef.current = nextId;
          setActiveCueId(nextId);
        }
      }

      if (
        isPlayingRef.current &&
        hasAudioSource &&
        webAudioBufferRef.current &&
        webAudioSourceRef.current
      ) {
        void startWebAudioAtOffset(clamped / 1000);
      }

      if (audioRef.current && audioSrcUrl) {
        try {
          if (
            !Number.isFinite(audioRef.current.duration) ||
            audioRef.current.duration > 0
          ) {
            audioRef.current.currentTime = clamped / 1000;
          }
        } catch {
          // Ignore seek errors before metadata
        }
      }
    },
    [audioSrcUrl, hasAudioSource, setActiveCueId, startWebAudioAtOffset],
  );

  // When switching projects or clearing to empty state, stop playback & reset playhead
  useEffect(() => {
    const currentJobId = activeProject?.jobId ?? selectedQueueItem?.jobId ?? null;
    const currentPhase = activeProject?.phase ?? selectedQueueItem?.phase ?? null;
    const prevJobId = lastResetJobIdRef.current;
    const prevPhase = lastResetPhaseRef.current;
    const prevCueCount = lastResetCueCountRef.current;

    const jobChanged = currentJobId !== prevJobId;
    const completedNow =
      !jobChanged &&
      currentPhase === "completed" &&
      prevPhase !== null &&
      PROCESSING_PHASES.has(prevPhase);
    const cuesArrivedAtZero =
      !jobChanged &&
      prevCueCount === 0 &&
      cues.length > 0 &&
      playheadRef.current === 0;

    lastResetJobIdRef.current = currentJobId;
    lastResetPhaseRef.current = currentPhase;
    lastResetCueCountRef.current = cues.length;

    if (jobChanged || completedNow) {
      setIsPlaying(false);
      segmentEndMsRef.current = null;
      stopWebAudioSource();
      if (audioRef.current) {
        audioRef.current.pause();
        try {
          if (
            !Number.isFinite(audioRef.current.duration) ||
            audioRef.current.duration > 0
          ) {
            audioRef.current.currentTime = 0;
          }
        } catch {
          // Ignore seek errors before metadata is loaded
        }
      }
      playheadRef.current = 0;
      setPlayheadMs(0);
      if (viewportRef.current) {
        viewportRef.current.scrollLeft = 0;
      }
      const zeroCue = findCueForTime(cuesRef.current, 0, false);
      const nextCueId = zeroCue ? zeroCue.id : null;
      activeCueIdRef.current = nextCueId;
      setActiveCueId(nextCueId);
      return;
    }

    if (cuesArrivedAtZero) {
      const zeroCue = findCueForTime(cuesRef.current, 0, false);
      const nextCueId = zeroCue ? zeroCue.id : null;
      if (nextCueId !== activeCueIdRef.current) {
        activeCueIdRef.current = nextCueId;
        setActiveCueId(nextCueId);
      }
    }
  }, [
    activeProject?.jobId,
    activeProject?.phase,
    selectedQueueItem?.jobId,
    selectedQueueItem?.phase,
    cues.length,
    setActiveCueId,
    stopWebAudioSource,
  ]);

  // Handle external playbackRequest (e.g. playCueSegment)
  useEffect(() => {
    if (
      !playbackRequest ||
      !hasLoadedProjectOrFile ||
      lastHandledPlaybackRequestRef.current === playbackRequest
    ) {
      return;
    }
    lastHandledPlaybackRequestRef.current = playbackRequest;
    const ctx = ensureAudioContext();
    if (ctx && ctx.state === "suspended") {
      void ctx.resume().catch(() => {});
    }
    segmentEndMsRef.current = playbackRequest.endMs;
    activeCueIdRef.current = playbackRequest.cueId;
    setActiveCueId(playbackRequest.cueId);
    seekPlayhead(playbackRequest.startMs, false);
    setIsPlaying(true);
  }, [
    playbackRequest,
    hasLoadedProjectOrFile,
    ensureAudioContext,
    seekPlayhead,
    setActiveCueId,
  ]);

  // Playback loop (Web Audio primary + blob/<audio> fallback + rAF clock fallback)
  useEffect(() => {
    if (!isPlaying || !hasLoadedProjectOrFile) {
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      stopWebAudioSource();
      if (audioRef.current && !audioRef.current.paused) {
        audioRef.current.pause();
      }
      return;
    }

    let cancelled = false;
    let clockSource: "webaudio" | "audio" | "raf" = "raf";

    const syncEditorToPlayhead = (ms: number) => {
      const matched = findCueForTime(cuesRef.current, ms, false);
      const nextId = matched ? matched.id : null;
      if (nextId === activeCueIdRef.current) {
        return;
      }
      const activeElId = document.activeElement?.id;
      if (
        activeElId === "textarea-original" ||
        activeElId === "textarea-prevod"
      ) {
        return;
      }
      activeCueIdRef.current = nextId;
      setActiveCueId(nextId);
    };

    const startClock = () => {
      lastRafTickRef.current = performance.now();

      const tick = (now: number) => {
        if (cancelled) return;

        const dt = Math.max(0, now - lastRafTickRef.current);
        lastRafTickRef.current = now;

        let nextMs: number;
        if (
          clockSource === "webaudio" &&
          webAudioCtxRef.current &&
          webAudioSourceRef.current
        ) {
          const elapsedSec = Math.max(
            0,
            webAudioCtxRef.current.currentTime -
              webAudioStartCtxTimeRef.current,
          );
          nextMs = (webAudioStartOffsetSecRef.current + elapsedSec) * 1000;
        } else if (clockSource === "audio" && audioRef.current) {
          nextMs = audioRef.current.currentTime * 1000;
        } else {
          nextMs = playheadRef.current + dt;
        }

        const segEnd = segmentEndMsRef.current;
        const limit =
          segEnd !== null
            ? Math.min(segEnd, audioEndMsRef.current)
            : audioEndMsRef.current;

        if (nextMs >= limit) {
          playheadRef.current = limit;
          setPlayheadMs(limit);
          syncEditorToPlayhead(limit);
          segmentEndMsRef.current = null;
          stopWebAudioSource();
          if (audioRef.current && !audioRef.current.paused) {
            audioRef.current.pause();
          }
          setIsPlaying(false);
          return;
        }

        playheadRef.current = nextMs;
        setPlayheadMs(nextMs);
        syncEditorToPlayhead(nextMs);
        rafIdRef.current = window.requestAnimationFrame(tick);
      };

      rafIdRef.current = window.requestAnimationFrame(tick);
    };

    const runAudioElementFallback = () => {
      const audioEl = audioRef.current;
      if (hasAudioSource && audioSrcUrl && audioEl && !audioEl.error) {
        try {
          if (audioEl.readyState === 0) {
            audioEl.load();
          }
          audioEl.currentTime = playheadRef.current / 1000;
          audioEl.muted = tracksRef.current.mp3.muted;
          const playPromise = audioEl.play();
          if (playPromise && typeof playPromise.then === "function") {
            playPromise
              .then(() => {
                if (!cancelled) {
                  clockSource = "audio";
                  startClock();
                }
              })
              .catch((err) => {
                console.warn("[WaveformTimeline] <audio> play() failed", err);
                if (!cancelled) {
                  clockSource = "raf";
                  startClock();
                }
              });
            return;
          }
          clockSource = "audio";
          startClock();
          return;
        } catch (err) {
          console.warn("[WaveformTimeline] <audio> playback threw", err);
        }
      }
      clockSource = "raf";
      startClock();
    };

    if (hasAudioSource) {
      void startWebAudioAtOffset(playheadRef.current / 1000).then((ok) => {
        if (cancelled) {
          if (ok) {
            stopWebAudioSource();
          }
          return;
        }
        if (ok) {
          clockSource = "webaudio";
          startClock();
        } else {
          runAudioElementFallback();
        }
      });
    } else {
      clockSource = "raf";
      startClock();
    }

    return () => {
      cancelled = true;
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      stopWebAudioSource();
      if (audioRef.current && !audioRef.current.paused) {
        audioRef.current.pause();
      }
    };
  }, [
    isPlaying,
    hasLoadedProjectOrFile,
    hasAudioSource,
    audioSrcUrl,
    activeProject?.durationMs,
    setActiveCueId,
    startWebAudioAtOffset,
    stopWebAudioSource,
  ]);

  const togglePlayTimeline = useCallback(() => {
    if (!hasLoadedRef.current) return;
    if (isPlaying) {
      segmentEndMsRef.current = null;
      setIsPlaying(false);
      return;
    }
    const ctx = ensureAudioContext();
    if (ctx && ctx.state === "suspended") {
      void ctx.resume().catch(() => {});
    }
    segmentEndMsRef.current = null;
    if (playheadRef.current >= audioEndMsRef.current) {
      seekPlayhead(0, true);
    }
    if (audioRef.current && audioSrcUrl && !audioRef.current.error) {
      audioRef.current.muted = tracksRef.current.mp3.muted;
    }
    setIsPlaying(true);
  }, [isPlaying, audioSrcUrl, ensureAudioContext, seekPlayhead]);

  const playFromStart = useCallback(() => {
    if (!hasLoadedRef.current) return;
    const ctx = ensureAudioContext();
    if (ctx && ctx.state === "suspended") {
      void ctx.resume().catch(() => {});
    }
    segmentEndMsRef.current = null;
    seekPlayhead(0, true);
    if (audioRef.current && audioSrcUrl && !audioRef.current.error) {
      audioRef.current.muted = tracksRef.current.mp3.muted;
    }
    setIsPlaying(true);
  }, [audioSrcUrl, ensureAudioContext, seekPlayhead]);

  const stopTimeline = useCallback(() => {
    segmentEndMsRef.current = null;
    setIsPlaying(false);
    seekPlayhead(0, true);
  }, [seekPlayhead]);

  // Measure viewport width for adaptive ruler ticks & HiDPI waveform
  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const updateSize = () => {
      setViewportWidth(Math.max(300, vp.clientWidth));
    };
    updateSize();
    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(updateSize);
      ro.observe(vp);
      return () => ro.disconnect();
    }
    window.addEventListener("resize", updateSize);
    return () => window.removeEventListener("resize", updateSize);
  }, []);

  // Mouse-wheel zoom (disabled on empty startup state)
  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;
    const handleWheel = (e: WheelEvent) => {
      if (!hasLoadedRef.current) {
        return;
      }
      if (Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
        return;
      }
      if (e.shiftKey) {
        e.preventDefault();
        vp.scrollLeft += e.deltaY;
        return;
      }
      if (e.deltaY === 0) {
        return;
      }
      e.preventDefault();
      const prevZoom = zoomPctRef.current;
      const nextZoom = computeWheelZoomPct(prevZoom, e.deltaY);
      if (nextZoom === prevZoom) {
        return;
      }
      const rect = vp.getBoundingClientRect();
      const cursorX = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
      if (!pendingZoomAnchorRef.current) {
        const currentContentWidth = Math.max(
          1,
          vp.clientWidth * (renderedZoomPctRef.current / 100),
        );
        const timeRatio = Math.max(
          0,
          Math.min(1, (vp.scrollLeft + cursorX) / currentContentWidth),
        );
        pendingZoomAnchorRef.current = { timeRatio, cursorX };
      } else {
        pendingZoomAnchorRef.current.cursorX = cursorX;
      }
      zoomPctRef.current = nextZoom;
      setZoomPct(nextZoom);
    };
    vp.addEventListener("wheel", handleWheel, { passive: false });
    return () => vp.removeEventListener("wheel", handleWheel);
  }, []);

  useLayoutEffect(() => {
    renderedZoomPctRef.current = zoomPct;
    const anchor = pendingZoomAnchorRef.current;
    const vp = viewportRef.current;
    if (!anchor || !vp) {
      pendingZoomAnchorRef.current = null;
      return;
    }
    pendingZoomAnchorRef.current = null;
    const newContentWidth = Math.max(1, vp.clientWidth * (zoomPct / 100));
    vp.scrollLeft = Math.max(
      0,
      anchor.timeRatio * newContentWidth - anchor.cursorX,
    );
  }, [zoomPct]);

  // Draw HiDPI Waveform Canvas from real Rust peaks only (zero fake procedural waves on empty/subtitle-only states)
  const drawWaveform = useCallback(() => {
    const canvas = canvasRef.current;
    const parent = mp3ClipRef.current;
    if (!canvas || !parent || !hasAudioSource) return;

    const rect = parent.getBoundingClientRect();
    const width = Math.max(1, Math.floor(rect.width));
    const height = Math.max(1, Math.floor(rect.height));
    const dpr = window.devicePixelRatio || 1;
    const { backingWidth, backingHeight, scaleX, scaleY } =
      computeSafeCanvasSize(width, height, dpr);

    canvas.width = backingWidth;
    canvas.height = backingHeight;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(scaleX, 0, 0, scaleY, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const midY = height / 2;
    ctx.fillStyle = "rgba(56, 189, 248, 0.14)";
    ctx.fillRect(0, midY - 0.5, width, 1);

    const peaks = audioPeaksRef.current;
    if (!peaks || peaks.length === 0) {
      return;
    }

    const stepX = 3;
    const barW = 2;
    const maxHalf = Math.max(4, height * 0.41);

    for (let x = 2; x < width - 2; x += stepX) {
      const frac0 = x / width;
      const frac1 = (x + stepX) / width;
      const b0 = Math.max(0, Math.floor(frac0 * peaks.length));
      const b1 = Math.min(
        peaks.length,
        Math.max(b0 + 1, Math.floor(frac1 * peaks.length)),
      );

      let amp = 0;
      for (let b = b0; b < b1; b += 1) {
        if (peaks[b] > amp) {
          amp = peaks[b];
        }
      }

      const tMs = frac0 * actualAudioDurationMs;
      const inCue =
        cues.length > 0 && cues.some((c) => tMs >= c.startMs && tMs <= c.endMs);
      ctx.fillStyle =
        cues.length === 0 || inCue
          ? "rgba(56, 189, 248, 0.85)"
          : "rgba(56, 189, 248, 0.42)";

      const barH = Math.max(2, amp * maxHalf * 2);
      ctx.fillRect(x, midY - barH / 2, barW, barH);
    }
  }, [hasAudioSource, cues, actualAudioDurationMs, waveformVersion]);

  useEffect(() => {
    if (!hasAudioSource) return;
    const id = window.requestAnimationFrame(drawWaveform);
    return () => window.cancelAnimationFrame(id);
  }, [hasAudioSource, drawWaveform, zoomPct, viewportWidth]);

  useEffect(() => {
    if (!hasAudioSource) return;
    const clipEl = mp3ClipRef.current;
    if (!clipEl || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      window.requestAnimationFrame(drawWaveform);
    });
    ro.observe(clipEl);
    return () => ro.disconnect();
  }, [hasAudioSource, drawWaveform]);

  // Global keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const st = useAppStore.getState();
      if (
        st.isSettingsOpen ||
        st.isExportOpen ||
        st.isSetupOpen ||
        st.isUpdaterOpen ||
        st.isCheckpointsOpen ||
        st.isBugReportOpen ||
        st.onlineConfirm.open
      ) {
        return;
      }

      const mod = e.ctrlKey || e.metaKey;
      const keyLower = e.key.toLowerCase();
      const inEditable = isEditableElement(e.target);

      // Alt + 1 (Original transcript) / Alt + 2 (Prevod)
      if (
        e.altKey &&
        !mod &&
        !e.shiftKey &&
        (e.code === "Digit1" ||
          e.code === "Numpad1" ||
          e.key === "1" ||
          e.code === "Digit2" ||
          e.code === "Numpad2" ||
          e.key === "2")
      ) {
        e.preventDefault();
        const isOrig =
          e.code === "Digit1" || e.code === "Numpad1" || e.key === "1";
        const targetId = isOrig ? "textarea-original" : "textarea-prevod";

        if (activeCueIdRef.current === null && cuesRef.current.length > 0) {
          const fallbackCue =
            findCueForTime(cuesRef.current, playheadRef.current, true) ??
            cuesRef.current[0];
          if (fallbackCue) {
            activeCueIdRef.current = fallbackCue.id;
            setActiveCueId(fallbackCue.id);
            seekPlayhead(fallbackCue.startMs, false);
          }
        }

        const focusBox = () => {
          const el = document.getElementById(
            targetId,
          ) as HTMLTextAreaElement | null;
          if (el && !el.disabled) {
            el.focus();
            const len = el.value.length;
            el.setSelectionRange(len, len);
            return true;
          }
          return false;
        };

        if (!focusBox()) {
          window.requestAnimationFrame(() => {
            window.requestAnimationFrame(() => {
              focusBox();
            });
          });
        }
        return;
      }

      if (!hasLoadedRef.current) {
        return;
      }

      // Shift + Space: Play from start (0 ms)
      if (
        e.code === "Space" &&
        !mod &&
        !e.altKey &&
        e.shiftKey &&
        !inEditable &&
        !(e.target instanceof HTMLButtonElement)
      ) {
        e.preventDefault();
        playFromStart();
        return;
      }

      // Space: Start / Stop playback at current marker
      if (
        e.code === "Space" &&
        !mod &&
        !e.altKey &&
        !e.shiftKey &&
        !inEditable &&
        !(e.target instanceof HTMLButtonElement)
      ) {
        e.preventDefault();
        togglePlayTimeline();
        return;
      }

      // Ctrl/Cmd+Z / Ctrl/Cmd+Y: Undo / Redo
      if (mod && !e.altKey && !inEditable) {
        if (keyLower === "z" && !e.shiftKey) {
          e.preventDefault();
          undoCueEdit();
          return;
        }
        if (keyLower === "y" || (keyLower === "z" && e.shiftKey)) {
          e.preventDefault();
          redoCueEdit();
          return;
        }
      }

      // C: Split segment under playhead marker
      if (
        !inEditable &&
        !mod &&
        !e.altKey &&
        !e.shiftKey &&
        (keyLower === "c" || e.code === "KeyC")
      ) {
        const ms = playheadRef.current;
        const cueUnderMarker = cuesRef.current.find(
          (c) =>
            ms >= c.startMs + MIN_SPLIT_MARGIN_MS &&
            ms <= c.endMs - MIN_SPLIT_MARGIN_MS,
        );
        if (cueUnderMarker) {
          e.preventDefault();
          splitCueAtMidpoint(cueUnderMarker.id, Math.round(ms));
        }
        return;
      }

      // ArrowLeft / ArrowRight (+ Shift): Step playhead by frame (40ms) or large jump (1000ms)
      if (
        !inEditable &&
        !mod &&
        !e.altKey &&
        (e.key === "ArrowLeft" || e.key === "ArrowRight")
      ) {
        e.preventDefault();
        const step = e.shiftKey ? LARGE_JUMP_MS : FRAME_STEP_MS;
        const delta = e.key === "ArrowRight" ? step : -step;
        segmentEndMsRef.current = null;
        seekPlayhead(playheadRef.current + delta, true);
        return;
      }

      // ArrowUp (Play next segment) / ArrowDown (Play previous segment)
      if (
        !inEditable &&
        !mod &&
        !e.altKey &&
        !e.shiftKey &&
        cuesRef.current.length > 0 &&
        (e.key === "ArrowUp" || e.key === "ArrowDown")
      ) {
        e.preventDefault();
        const list = cuesRef.current;
        const curIdx =
          activeCueIdRef.current !== null
            ? list.findIndex((c) => c.id === activeCueIdRef.current)
            : -1;
        let target: SubtitleCue | undefined;
        if (e.key === "ArrowUp") {
          target =
            curIdx !== -1
              ? list[Math.min(list.length - 1, curIdx + 1)]
              : (list.find((c) => c.startMs > playheadRef.current) ??
                list[list.length - 1]);
        } else {
          target =
            curIdx !== -1
              ? list[Math.max(0, curIdx - 1)]
              : ([...list]
                  .reverse()
                  .find((c) => c.startMs < playheadRef.current) ?? list[0]);
        }
        if (target) {
          const ctx = ensureAudioContext();
          if (ctx && ctx.state === "suspended") {
            void ctx.resume().catch(() => {});
          }
          playCueSegment(target);
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    ensureAudioContext,
    togglePlayTimeline,
    playFromStart,
    undoCueEdit,
    redoCueEdit,
    splitCueAtMidpoint,
    playCueSegment,
    setActiveCueId,
    seekPlayhead,
  ]);

  // Compute adaptive ruler ticks & gridlines (empty when no project/file loaded)
  const rulerTicks = useMemo(() => {
    if (!hasLoadedProjectOrFile || totalMs <= 0) {
      return [];
    }
    const contentWidthPx = viewportWidth * (zoomPct / 100);
    const pxPerSec = contentWidthPx / Math.max(0.1, totalMs / 1000);
    const majorSteps = [
      100, 200, 500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000,
      300000,
    ];
    const majorStepMs =
      majorSteps.find((step) => (step / 1000) * pxPerSec >= 64) ?? 300000;
    const minorStepMs =
      majorStepMs >= 5000 ? majorStepMs / 5 : majorStepMs / 2;

    const ticks: Array<{
      ms: number;
      pct: number;
      isMajor: boolean;
      label: string | null;
    }> = [];

    for (let ms = 0; ms <= totalMs; ms += minorStepMs) {
      const pct = (ms / totalMs) * 100;
      const isMajor = Math.round(ms) % majorStepMs === 0;
      ticks.push({
        ms,
        pct,
        isMajor,
        label: isMajor && ms < totalMs ? formatRulerTimestamp(ms) : null,
      });
    }
    return ticks;
  }, [hasLoadedProjectOrFile, viewportWidth, zoomPct, totalMs]);

  const scrubFromClientX = (clientX: number) => {
    if (!hasLoadedProjectOrFile || totalMs <= 0) return;
    const contentEl = contentRef.current;
    if (!contentEl) return;
    const rect = contentEl.getBoundingClientRect();
    const ratio = Math.max(
      0,
      Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)),
    );
    segmentEndMsRef.current = null;
    seekPlayhead(ratio * totalMs, true);
  };

  // Clip trim drag handlers
  const beginTrimDrag = (
    e: React.PointerEvent<HTMLDivElement>,
    cue: SubtitleCue,
    edge: "start" | "end",
    trackKind: "original" | "prevod",
  ) => {
    e.stopPropagation();
    if (!hasLoadedProjectOrFile || tracks[trackKind].locked) return;
    const contentEl = contentRef.current;
    if (!contentEl) return;

    e.currentTarget.setPointerCapture(e.pointerId);
    const rect = contentEl.getBoundingClientRect();
    setTrimDrag({
      cueId: cue.id,
      edge,
      startClientX: e.clientX,
      origStartMs: cue.startMs,
      origEndMs: cue.endMs,
      contentWidthPx: Math.max(1, rect.width),
      previewStartMs: cue.startMs,
      previewEndMs: cue.endMs,
    });
  };

  const moveTrimDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!trimDrag || totalMs <= 0) return;
    e.stopPropagation();
    const dx = e.clientX - trimDrag.startClientX;
    const deltaMs = (dx / trimDrag.contentWidthPx) * totalMs;
    if (trimDrag.edge === "start") {
      const nextStart = Math.max(
        0,
        Math.min(
          trimDrag.origEndMs - 200,
          Math.round(trimDrag.origStartMs + deltaMs),
        ),
      );
      setTrimDrag({ ...trimDrag, previewStartMs: nextStart });
    } else {
      const nextEnd = Math.min(
        totalMs,
        Math.max(
          trimDrag.origStartMs + 200,
          Math.round(trimDrag.origEndMs + deltaMs),
        ),
      );
      setTrimDrag({ ...trimDrag, previewEndMs: nextEnd });
    }
  };

  const endTrimDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!trimDrag) return;
    e.stopPropagation();
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    const { cueId, previewStartMs, previewEndMs, origStartMs, origEndMs } =
      trimDrag;
    setTrimDrag(null);
    setActiveCueId(cueId);
    if (previewStartMs !== origStartMs || previewEndMs !== origEndMs) {
      updateCue(cueId, {
        startMs: previewStartMs,
        endMs: previewEndMs,
      });
    }
  };

  const renderClips = (kind: "original" | "prevod") => {
    if (!hasLoadedProjectOrFile || totalMs <= 0 || tracks[kind].hidden) {
      return null;
    }
    return cues.map((cue) => {
      const isTrimming = trimDrag?.cueId === cue.id;
      const startMs = isTrimming ? trimDrag.previewStartMs : cue.startMs;
      const endMs = isTrimming ? trimDrag.previewEndMs : cue.endMs;
      const leftPct = (startMs / totalMs) * 100;
      const widthPct = Math.max(0.3, ((endMs - startMs) / totalMs) * 100);
      const isSelected = cue.id === activeCue?.id;
      const rawText =
        kind === "original" ? cue.sourceText : cue.translatedText;
      const displayText = rawText.trim() || (kind === "prevod" ? "—" : "");
      const isPlaceholder = !rawText.trim();

      return (
        <div
          key={`${kind}-${cue.id}`}
          data-clip-id={cue.id}
          data-track={kind}
          className={`sub-clip ${kind}${isSelected ? " selected" : ""}${
            isTrimming ? " dragging" : ""
          }`}
          style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
          title={`#${cue.id} (${formatRulerSec(startMs)}–${formatRulerSec(endMs)}): ${rawText}`}
          onClick={(e) => {
            e.stopPropagation();
            setActiveCueId(cue.id);
          }}
          onDoubleClick={(e) => {
            e.stopPropagation();
            setActiveCueId(cue.id);
            segmentEndMsRef.current = null;
            seekPlayhead(startMs, false);
          }}
        >
          <div
            className="clip-trim-handle left"
            onPointerDown={(e) => beginTrimDrag(e, cue, "start", kind)}
            onPointerMove={moveTrimDrag}
            onPointerUp={endTrimDrag}
            onPointerCancel={endTrimDrag}
          />
          <span className="clip-id">#{cue.id}</span>
          <span
            className={`clip-text${isPlaceholder ? " placeholder" : ""}`}
          >
            {displayText}
          </span>
          <div
            className="clip-trim-handle right"
            onPointerDown={(e) => beginTrimDrag(e, cue, "end", kind)}
            onPointerMove={moveTrimDrag}
            onPointerUp={endTrimDrag}
            onPointerCancel={endTrimDrag}
          />
        </div>
      );
    });
  };

  const showHours = totalMs >= 3_600_000;
  const playheadPct =
    hasLoadedProjectOrFile && totalMs > 0
      ? Math.max(0, Math.min(100, (playheadMs / totalMs) * 100))
      : 0;
  const mp3ClipPct =
    hasAudioSource && totalMs > 0
      ? Math.max(1, Math.min(100, (actualAudioDurationMs / totalMs) * 100))
      : 0;

  const srcLangSub = (
    activeProject?.detectedLanguage ||
    activeProject?.options.sourceLanguage ||
    jobOptions.sourceLanguage ||
    "EN"
  ).toUpperCase();
  const dstLangSub =
    (activeProject?.options.targetLanguage || jobOptions.targetLanguage) ===
    "none"
      ? "—"
      : (
          activeProject?.options.targetLanguage ||
          jobOptions.targetLanguage ||
          "SR"
        ).toUpperCase();

  const isLastCue =
    Boolean(activeCue) &&
    cues.length > 0 &&
    activeCue?.id === cues[cues.length - 1]?.id;

  return (
    <section
      id="timeline-panel"
      className={hasLoadedProjectOrFile ? undefined : "tl-empty"}
      aria-label="Timeline"
    >
      <audio
        ref={audioRef}
        src={hasAudioSource && audioSrcUrl ? audioSrcUrl : undefined}
        preload="metadata"
        onLoadedMetadata={(e) => {
          const el = e.currentTarget;
          if (Number.isFinite(el.duration) && el.duration > 0) {
            if (!activeProject?.durationMs && liveAudioDurationMs <= 0) {
              setLiveAudioDurationMs(Math.round(el.duration * 1000));
            }
            if (Math.abs(el.currentTime * 1000 - playheadRef.current) > 80) {
              el.currentTime = Math.min(
                el.duration,
                playheadRef.current / 1000,
              );
            }
          }
        }}
        onEnded={() => {
          segmentEndMsRef.current = null;
          setIsPlaying(false);
        }}
        onError={(e) => {
          console.warn(
            "[WaveformTimeline] <audio> load error",
            e.currentTarget.error?.code,
            e.currentTarget.error?.message,
          );
        }}
        className="hidden"
      />

      <div className="tl-toolbar">
        <div className="tl-transport">
          <button
            id="tl-btn-play"
            className={`tl-icon-btn${isPlaying ? " playing" : ""}`}
            type="button"
            disabled={!hasLoadedProjectOrFile}
            title={
              lang === "en"
                ? "Play / Pause (Space)"
                : "Pusti / Pauziraj (Space)"
            }
            onClick={togglePlayTimeline}
          >
            {isPlaying ? (
              <svg
                id="tl-icon-pause"
                width="11"
                height="11"
                viewBox="0 0 24 24"
                fill="currentColor"
              >
                <rect x="6" y="4" width="4" height="16" rx="1" />
                <rect x="14" y="4" width="4" height="16" rx="1" />
              </svg>
            ) : (
              <svg
                id="tl-icon-play"
                width="11"
                height="11"
                viewBox="0 0 24 24"
                fill="currentColor"
              >
                <polygon points="6 4 20 12 6 20 6 4" />
              </svg>
            )}
          </button>

          <button
            id="tl-btn-stop"
            className="tl-icon-btn"
            type="button"
            disabled={!hasLoadedProjectOrFile}
            title={
              lang === "en"
                ? "Stop and return to start"
                : "Zaustavi i vrati na početak"
            }
            onClick={stopTimeline}
          >
            <svg width="9" height="9" viewBox="0 0 24 24" fill="currentColor">
              <rect x="4" y="4" width="16" height="16" rx="2" />
            </svg>
          </button>

          <div className="tl-timecode" id="tl-timecode">
            <span id="tl-tc-cur" className="tl-tc-cur">
              {hasLoadedProjectOrFile
                ? formatTimecode(playheadMs, showHours)
                : "00:00:00.00"}
            </span>
            <span className="tl-tc-sep">/</span>
            <span className="tl-tc-tot">
              {hasLoadedProjectOrFile
                ? formatTimecode(audioEndMs, showHours)
                : "00:00:00.00"}
            </span>
          </div>
        </div>

        {/* Compact Cue Editing Controls */}
        <div className="tl-edit-controls">
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!hasLoadedProjectOrFile || !canUndo}
            onClick={undoCueEdit}
            title={t.table.undo}
          >
            <Undo2 size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!hasLoadedProjectOrFile || !canRedo}
            onClick={redoCueEdit}
            title={t.table.redo}
          >
            <Redo2 size={12} />
          </button>

          <span className="tl-sep" aria-hidden="true" />

          <button
            type="button"
            className="tl-icon-btn"
            disabled={
              !hasLoadedProjectOrFile ||
              (!activeCue &&
                !cues.some(
                  (c) =>
                    playheadMs >= c.startMs + MIN_SPLIT_MARGIN_MS &&
                    playheadMs <= c.endMs - MIN_SPLIT_MARGIN_MS,
                ))
            }
            onClick={() => {
              const markerCue = cues.find(
                (c) =>
                  playheadRef.current >= c.startMs + MIN_SPLIT_MARGIN_MS &&
                  playheadRef.current <= c.endMs - MIN_SPLIT_MARGIN_MS,
              );
              if (markerCue) {
                splitCueAtMidpoint(
                  markerCue.id,
                  Math.round(playheadRef.current),
                );
              } else if (activeCue) {
                splitCueAtMidpoint(activeCue.id);
              }
            }}
            title={`${t.table.splitCue} (C)`}
          >
            <Scissors size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!hasLoadedProjectOrFile || !activeCue || isLastCue}
            onClick={() => activeCue && mergeCueWithNext(activeCue.id)}
            title={t.table.mergeNext}
          >
            <Merge size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn add"
            disabled={!hasLoadedProjectOrFile || !activeProject}
            onClick={addCueAtEnd}
            title={t.table.addCue}
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn danger"
            disabled={!hasLoadedProjectOrFile || !activeCue}
            onClick={() => activeCue && deleteCueById(activeCue.id)}
            title={t.table.deleteCue}
          >
            <Trash2 size={12} />
          </button>
        </div>

        <div className="tl-zoom-controls">
          <button
            id="tl-zoom-out"
            className="tl-zoom-btn"
            type="button"
            disabled={!hasLoadedProjectOrFile}
            title={
              lang === "en"
                ? "Zoom out (Mouse wheel down)"
                : "Umanji zoom (Točkić miša dole)"
            }
            onClick={() => setZoomPct((z) => computeButtonZoomPct(z, "out"))}
          >
            −
          </button>
          <input
            id="tl-zoom-slider"
            className="tl-zoom-slider"
            type="range"
            disabled={!hasLoadedProjectOrFile}
            min={MIN_TIMELINE_ZOOM_PCT}
            max={MAX_TIMELINE_ZOOM_PCT}
            step={25}
            value={zoomPct}
            onChange={(e) => setZoomPct(Number(e.target.value))}
            aria-label="Timeline zoom"
          />
          <span id="tl-zoom-label" className="tl-zoom-label">
            {zoomPct}%
          </span>
          <button
            id="tl-zoom-in"
            className="tl-zoom-btn"
            type="button"
            disabled={!hasLoadedProjectOrFile}
            title={
              lang === "en"
                ? "Zoom in (Mouse wheel up)"
                : "Uvećaj zoom (Točkić miša gore)"
            }
            onClick={() => setZoomPct((z) => computeButtonZoomPct(z, "in"))}
          >
            +
          </button>
        </div>
      </div>

      <div className="tl-body">
        {/* Left Sticky Track Headers */}
        <div className="tl-headers-col">
          <div className="tl-corner-cell">TIMELINE</div>

          {/* Track 1: MP3 */}
          <div className="tl-track-header mp3">
            <div className="tl-th-info">
              <span className="tl-th-icon" aria-hidden="true">
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                >
                  <path d="M4 10v4" />
                  <path d="M8 6v12" />
                  <path d="M12 3v18" />
                  <path d="M16 8v8" />
                  <path d="M20 11v2" />
                </svg>
              </span>
              <div className="tl-th-text">
                <span className="tl-th-title">MP3</span>
                <span className="tl-th-sub">Audio</span>
              </div>
            </div>

            <div className="tl-th-actions">
              <button
                type="button"
                className={`tl-micro-btn js-mute-btn${
                  tracks.mp3.muted ? " active-mute" : ""
                }`}
                data-track="mp3"
                title={lang === "en" ? "Mute" : "Utišaj (Mute)"}
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    mp3: { ...prev.mp3, muted: !prev.mp3.muted },
                  }))
                }
              >
                M
              </button>
              <button
                type="button"
                className={`tl-micro-btn js-lock-btn${
                  tracks.mp3.locked ? " active-lock" : ""
                }`}
                data-track="mp3"
                title={lang === "en" ? "Lock track" : "Zaključaj traku"}
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    mp3: { ...prev.mp3, locked: !prev.mp3.locked },
                  }))
                }
              >
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.1"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="4" y="11" width="16" height="10" rx="2" />
                  <path d="M8 11V7a4 4 0 0 1 8 0v4" />
                </svg>
              </button>
            </div>
          </div>

          {/* Track 2: Original transcript (localized) */}
          <div className="tl-track-header original">
            <div className="tl-th-info">
              <span className="tl-th-icon" aria-hidden="true">
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                >
                  <path d="M4 7h16" />
                  <path d="M4 12h12" />
                  <path d="M4 17h14" />
                </svg>
              </span>
              <div className="tl-th-text">
                <span className="tl-th-title">{t.editor.originalTitle}</span>
                <span className="tl-th-sub">{srcLangSub}</span>
              </div>
            </div>

            <div className="tl-th-actions">
              <button
                type="button"
                className={`tl-micro-btn js-vis-btn${
                  tracks.original.hidden ? " active-hidden" : ""
                }`}
                data-track="original"
                title={
                  lang === "en" ? "Hide / show track" : "Sakrij / prikaži traku"
                }
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    original: {
                      ...prev.original,
                      hidden: !prev.original.hidden,
                    },
                  }))
                }
              >
                <svg
                  width="11"
                  height="11"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z" />
                  <circle cx="12" cy="12" r="2.5" />
                </svg>
              </button>
              <button
                type="button"
                className={`tl-micro-btn js-lock-btn${
                  tracks.original.locked ? " active-lock" : ""
                }`}
                data-track="original"
                title={lang === "en" ? "Lock track" : "Zaključaj traku"}
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    original: {
                      ...prev.original,
                      locked: !prev.original.locked,
                    },
                  }))
                }
              >
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.1"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="4" y="11" width="16" height="10" rx="2" />
                  <path d="M8 11V7a4 4 0 0 1 8 0v4" />
                </svg>
              </button>
            </div>
          </div>

          {/* Track 3: Prevod */}
          <div className="tl-track-header prevod">
            <div className="tl-th-info">
              <span className="tl-th-icon" aria-hidden="true">
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
                  <rect x="3" y="5" width="18" height="14" rx="2" />
                  <path d="M7 13h4" />
                  <path d="M13 13h4" />
                  <path d="M7 16h10" />
                </svg>
              </span>
              <div className="tl-th-text">
                <span className="tl-th-title">{t.editor.translationTitle}</span>
                <span className="tl-th-sub">{dstLangSub}</span>
              </div>
            </div>

            <div className="tl-th-actions">
              <button
                type="button"
                className={`tl-micro-btn js-vis-btn${
                  tracks.prevod.hidden ? " active-hidden" : ""
                }`}
                data-track="prevod"
                title={
                  lang === "en" ? "Hide / show track" : "Sakrij / prikaži traku"
                }
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    prevod: { ...prev.prevod, hidden: !prev.prevod.hidden },
                  }))
                }
              >
                <svg
                  width="11"
                  height="11"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z" />
                  <circle cx="12" cy="12" r="2.5" />
                </svg>
              </button>
              <button
                type="button"
                className={`tl-micro-btn js-lock-btn${
                  tracks.prevod.locked ? " active-lock" : ""
                }`}
                data-track="prevod"
                title={lang === "en" ? "Lock track" : "Zaključaj traku"}
                onClick={() =>
                  setTracks((prev) => ({
                    ...prev,
                    prevod: { ...prev.prevod, locked: !prev.prevod.locked },
                  }))
                }
              >
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.1"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="4" y="11" width="16" height="10" rx="2" />
                  <path d="M8 11V7a4 4 0 0 1 8 0v4" />
                </svg>
              </button>
            </div>
          </div>
        </div>

        {/* Right Scrollable Tracks Viewport */}
        <div ref={viewportRef} id="tl-viewport" className="tl-viewport">
          <div
            ref={contentRef}
            id="tl-content"
            className="tl-content"
            style={{ width: hasLoadedProjectOrFile ? `${zoomPct}%` : "100%" }}
          >
            <div
              ref={rulerRef}
              id="tl-ruler"
              className={hasLoadedProjectOrFile ? undefined : "empty"}
              onPointerDown={(e) => {
                if (!hasLoadedProjectOrFile) return;
                scrubbingRef.current = true;
                e.currentTarget.setPointerCapture(e.pointerId);
                scrubFromClientX(e.clientX);
              }}
              onPointerMove={(e) => {
                if (!hasLoadedProjectOrFile) return;
                if (scrubbingRef.current) {
                  scrubFromClientX(e.clientX);
                }
              }}
              onPointerUp={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
              onPointerCancel={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
            >
              {hasLoadedProjectOrFile &&
                rulerTicks.map((tick) => (
                  <Fragment key={`t-${tick.ms}`}>
                    <div
                      className={`tl-tick ${tick.isMajor ? "major" : "minor"}`}
                      style={{ left: `${tick.pct}%` }}
                    />
                    {tick.label !== null && (
                      <span
                        className="tl-tick-label"
                        style={{ left: `${tick.pct}%` }}
                      >
                        {tick.label}
                      </span>
                    )}
                  </Fragment>
                ))}
            </div>

            <div id="tl-gridlines" className="tl-gridlines">
              {hasLoadedProjectOrFile &&
                rulerTicks
                  .filter((tick) => tick.isMajor && tick.ms > 0 && tick.ms < totalMs)
                  .map((tick) => (
                    <div
                      key={`g-${tick.ms}`}
                      className="tl-gridline"
                      style={{ left: `${tick.pct}%` }}
                    />
                  ))}
            </div>

            {/* Lane 1: MP3 Audio */}
            <div
              id="lane-mp3"
              className={`tl-lane${tracks.mp3.muted ? " muted" : ""}${
                tracks.mp3.locked ? " locked" : ""
              }`}
              onPointerDown={(e) => {
                if (!hasLoadedProjectOrFile || tracks.mp3.locked) return;
                scrubbingRef.current = true;
                e.currentTarget.setPointerCapture(e.pointerId);
                scrubFromClientX(e.clientX);
              }}
              onPointerMove={(e) => {
                if (!hasLoadedProjectOrFile || tracks.mp3.locked) return;
                if (scrubbingRef.current) {
                  scrubFromClientX(e.clientX);
                }
              }}
              onPointerUp={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
              onPointerCancel={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
            >
              {hasAudioSource && audioFileName && (
                <div
                  ref={mp3ClipRef}
                  className="mp3-clip"
                  style={{ width: `${mp3ClipPct}%` }}
                >
                  <span id="mp3-clip-tag" className="mp3-clip-tag">
                    {audioFileName}
                  </span>
                  <canvas ref={canvasRef} id="mp3-waveform" />
                </div>
              )}
            </div>

            {/* Lane 2: Original transcript */}
            <div
              id="lane-original"
              className={`tl-lane${tracks.original.hidden ? " hidden-track" : ""}${
                tracks.original.locked ? " locked" : ""
              }`}
              onPointerDown={(e) => {
                if (!hasLoadedProjectOrFile) return;
                if (e.target !== e.currentTarget) return;
                scrubbingRef.current = true;
                e.currentTarget.setPointerCapture(e.pointerId);
                scrubFromClientX(e.clientX);
              }}
              onPointerMove={(e) => {
                if (!hasLoadedProjectOrFile) return;
                if (scrubbingRef.current) {
                  scrubFromClientX(e.clientX);
                }
              }}
              onPointerUp={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
              onPointerCancel={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
            >
              {renderClips("original")}
            </div>

            {/* Lane 3: Prevod */}
            <div
              id="lane-prevod"
              className={`tl-lane${tracks.prevod.hidden ? " hidden-track" : ""}${
                tracks.prevod.locked ? " locked" : ""
              }`}
              onPointerDown={(e) => {
                if (!hasLoadedProjectOrFile) return;
                if (e.target !== e.currentTarget) return;
                scrubbingRef.current = true;
                e.currentTarget.setPointerCapture(e.pointerId);
                scrubFromClientX(e.clientX);
              }}
              onPointerMove={(e) => {
                if (!hasLoadedProjectOrFile) return;
                if (scrubbingRef.current) {
                  scrubFromClientX(e.clientX);
                }
              }}
              onPointerUp={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
              onPointerCancel={(e) => {
                scrubbingRef.current = false;
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              }}
            >
              {renderClips("prevod")}
            </div>

            {/* Vertical Playhead (only rendered when a file/project is loaded) */}
            {hasLoadedProjectOrFile && (
              <div id="tl-playhead" style={{ left: `${playheadPct}%` }} />
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
