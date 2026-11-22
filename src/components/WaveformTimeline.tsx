import { convertFileSrc } from "@tauri-apps/api/core";
import { Merge, Plus, Redo2, Scissors, Trash2, Undo2 } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type { SubtitleCue } from "../types/models";

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

export function WaveformTimeline() {
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const playbackRequest = useAppStore((s) => s.playbackRequest);
  const canUndo = useAppStore((s) => s.canUndo);
  const canRedo = useAppStore((s) => s.canRedo);

  const setActiveCueId = useAppStore((s) => s.setActiveCueId);
  const updateCue = useAppStore((s) => s.updateCue);
  const splitCueAtMidpoint = useAppStore((s) => s.splitCueAtMidpoint);
  const mergeCueWithNext = useAppStore((s) => s.mergeCueWithNext);
  const deleteCueById = useAppStore((s) => s.deleteCueById);
  const addCueAtEnd = useAppStore((s) => s.addCueAtEnd);
  const undoCueEdit = useAppStore((s) => s.undoCueEdit);
  const redoCueEdit = useAppStore((s) => s.redoCueEdit);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const [zoomPct, setZoomPct] = useState(100);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playheadMs, setPlayheadMs] = useState(0);
  const [audioLoaded, setAudioLoaded] = useState(false);
  const [viewportWidth, setViewportWidth] = useState(1000);
  const [trimDrag, setTrimDrag] = useState<TrimDragState | null>(null);
  const [tracks, setTracks] = useState<TrackState>({
    mp3: { muted: false, locked: false },
    original: { hidden: false, locked: false },
    prevod: { hidden: false, locked: false },
  });

  const audioRef = useRef<HTMLAudioElement | null>(null);
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

  const cues = activeProject?.cues ?? [];
  const maxCueEndMs = useMemo(
    () => cues.reduce((max, c) => Math.max(max, c.endMs), 0),
    [cues],
  );
  const audioEndMs = useMemo(
    () => Math.max(activeProject?.durationMs ?? 0, maxCueEndMs, 1000),
    [activeProject?.durationMs, maxCueEndMs],
  );
  const totalMs = useMemo(
    () => Math.max(Math.round(audioEndMs * 1.04), maxCueEndMs + 500, 1000),
    [audioEndMs, maxCueEndMs],
  );

  const audioEndMsRef = useRef(audioEndMs);
  audioEndMsRef.current = audioEndMs;

  const activeCue = useMemo(() => {
    if (cues.length === 0) return null;
    if (activeCueId !== null) {
      return cues.find((c) => c.id === activeCueId) ?? cues[0];
    }
    return cues[0];
  }, [cues, activeCueId]);

  const audioSrc = useMemo(() => {
    const path = activeProject?.audioPath;
    if (!path || /\.(srt|vtt)$/i.test(path.trim())) {
      return "";
    }
    if (isTauriRuntime()) {
      try {
        return convertFileSrc(path);
      } catch {
        return "";
      }
    }
    return "";
  }, [activeProject?.audioPath]);

  // Reset audioLoaded when source changes
  useEffect(() => {
    setAudioLoaded(false);
  }, [audioSrc]);

  // Sync mute state onto <audio>
  useEffect(() => {
    if (audioRef.current) {
      audioRef.current.muted = tracks.mp3.muted;
    }
  }, [tracks.mp3.muted]);

  const seekPlayhead = useCallback(
    (ms: number) => {
      const clamped = Math.max(0, Math.min(audioEndMsRef.current, ms));
      playheadRef.current = clamped;
      setPlayheadMs(clamped);
      if (audioLoaded && audioRef.current) {
        try {
          audioRef.current.currentTime = clamped / 1000;
        } catch {
          // Ignore seek errors before metadata
        }
      }
    },
    [audioLoaded],
  );

  // When switching projects, stop playback and position playhead on active/first cue start
  useEffect(() => {
    setIsPlaying(false);
    segmentEndMsRef.current = null;
    if (audioRef.current) {
      audioRef.current.pause();
    }
    const targetCue =
      activeCueId !== null
        ? cues.find((c) => c.id === activeCueId) ?? cues[0]
        : cues[0];
    const start = targetCue ? targetCue.startMs : 0;
    const clamped = Math.max(0, Math.min(audioEndMs, start));
    playheadRef.current = clamped;
    setPlayheadMs(clamped);
    // Only reset on job change
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProject?.jobId]);

  // Handle external playbackRequest (e.g. playCueSegment)
  useEffect(() => {
    if (!playbackRequest) return;
    segmentEndMsRef.current = playbackRequest.endMs;
    seekPlayhead(playbackRequest.startMs);
    setIsPlaying(true);
  }, [playbackRequest, seekPlayhead]);

  // Playback loop (real audio sync + requestAnimationFrame fallback)
  useEffect(() => {
    if (!isPlaying) {
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      if (audioRef.current && !audioRef.current.paused) {
        audioRef.current.pause();
      }
      return;
    }

    let cancelled = false;
    let usingLiveAudio = false;

    const startClock = () => {
      lastRafTickRef.current = performance.now();

      const tick = (now: number) => {
        if (cancelled) return;

        let nextMs: number;
        if (usingLiveAudio && audioRef.current) {
          nextMs = audioRef.current.currentTime * 1000;
        } else {
          const dt = Math.max(0, now - lastRafTickRef.current);
          lastRafTickRef.current = now;
          nextMs = playheadRef.current + dt;
        }

        const segEnd = segmentEndMsRef.current;
        const limit = segEnd !== null ? Math.min(segEnd, audioEndMsRef.current) : audioEndMsRef.current;

        if (nextMs >= limit) {
          playheadRef.current = limit;
          setPlayheadMs(limit);
          segmentEndMsRef.current = null;
          if (audioRef.current && !audioRef.current.paused) {
            audioRef.current.pause();
          }
          setIsPlaying(false);
          return;
        }

        playheadRef.current = nextMs;
        setPlayheadMs(nextMs);
        rafIdRef.current = window.requestAnimationFrame(tick);
      };

      rafIdRef.current = window.requestAnimationFrame(tick);
    };

    const audioEl = audioRef.current;
    if (audioLoaded && audioEl) {
      try {
        if (Math.abs(audioEl.currentTime * 1000 - playheadRef.current) > 80) {
          audioEl.currentTime = playheadRef.current / 1000;
        }
        audioEl.muted = tracks.mp3.muted;
        const playPromise = audioEl.play();
        if (playPromise && typeof playPromise.then === "function") {
          usingLiveAudio = true;
          playPromise
            .then(() => {
              if (!cancelled) {
                startClock();
              }
            })
            .catch(() => {
              usingLiveAudio = false;
              if (!cancelled) {
                startClock();
              }
            });
        } else {
          usingLiveAudio = true;
          startClock();
        }
      } catch {
        usingLiveAudio = false;
        startClock();
      }
    } else {
      startClock();
    }

    return () => {
      cancelled = true;
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
    };
  }, [isPlaying, audioLoaded, tracks.mp3.muted]);

  const togglePlayTimeline = useCallback(() => {
    if (isPlaying) {
      segmentEndMsRef.current = null;
      setIsPlaying(false);
      return;
    }
    segmentEndMsRef.current = null;
    if (playheadRef.current >= audioEndMsRef.current) {
      seekPlayhead(0);
    }
    setIsPlaying(true);
  }, [isPlaying, seekPlayhead]);

  const stopTimeline = useCallback(() => {
    segmentEndMsRef.current = null;
    setIsPlaying(false);
    seekPlayhead(0);
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

  // Draw HiDPI MP3 Waveform Canvas
  const drawWaveform = useCallback(() => {
    const canvas = canvasRef.current;
    const parent = mp3ClipRef.current;
    if (!canvas || !parent) return;

    const rect = parent.getBoundingClientRect();
    const width = Math.max(1, Math.floor(rect.width));
    const height = Math.max(1, Math.floor(rect.height));
    const dpr = window.devicePixelRatio || 1;

    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const midY = height / 2;
    ctx.fillStyle = "rgba(56, 189, 248, 0.14)";
    ctx.fillRect(0, midY - 0.5, width, 1);

    const stepX = 3;
    const barW = 2;
    const maxHalf = Math.max(4, height * 0.41);
    const words = activeProject?.words ?? [];

    for (let x = 2; x < width - 2; x += stepX) {
      const tMs = (x / width) * audioEndMs;
      const activeWord =
        words.length > 0
          ? words.find((w) => tMs >= w.startMs && tMs <= w.endMs)
          : undefined;
      const activeSeg = cues.find((c) => tMs >= c.startMs && tMs <= c.endMs);

      let amp = 0.06;
      if (activeWord || activeSeg) {
        const spanStart = activeWord ? activeWord.startMs : activeSeg!.startMs;
        const spanEnd = activeWord ? activeWord.endMs : activeSeg!.endMs;
        const segId = activeSeg ? activeSeg.id : 1;
        const segPos = (tMs - spanStart) / Math.max(1, spanEnd - spanStart);
        const edgeFade = Math.sin(
          Math.min(1, Math.min(segPos * 7, (1 - segPos) * 7)) * (Math.PI / 2),
        );
        const wave =
          0.48 +
          0.24 * Math.sin(tMs * 0.017 + segId * 1.7) +
          0.18 * Math.cos(tMs * 0.043) +
          0.1 * Math.sin(tMs * 0.097);
        amp = Math.max(
          0.14,
          Math.min(0.95, wave * (0.45 + 0.55 * edgeFade)),
        );
        ctx.fillStyle = "rgba(56, 189, 248, 0.72)";
      } else {
        amp = 0.04 + 0.04 * Math.abs(Math.sin(tMs * 0.031));
        ctx.fillStyle = "rgba(56, 189, 248, 0.24)";
      }

      const barH = Math.max(2, amp * maxHalf * 2);
      ctx.fillRect(x, midY - barH / 2, barW, barH);
    }
  }, [activeProject?.words, cues, audioEndMs]);

  useEffect(() => {
    const id = window.requestAnimationFrame(drawWaveform);
    return () => window.cancelAnimationFrame(id);
  }, [drawWaveform, zoomPct, viewportWidth]);

  useEffect(() => {
    const clipEl = mp3ClipRef.current;
    if (!clipEl || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      window.requestAnimationFrame(drawWaveform);
    });
    ro.observe(clipEl);
    return () => ro.disconnect();
  }, [drawWaveform]);

  // Global keyboard shortcuts (Space, Ctrl+Z, Ctrl+Y, Alt+ArrowUp/Down)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      const keyLower = e.key.toLowerCase();
      const inEditable = isEditableElement(e.target);

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

      if (
        e.altKey &&
        !mod &&
        cues.length > 0 &&
        (e.key === "ArrowDown" || e.key === "ArrowUp")
      ) {
        e.preventDefault();
        const curIdx = cues.findIndex((c) => c.id === activeCue?.id);
        const nextIdx =
          e.key === "ArrowDown"
            ? curIdx === -1
              ? 0
              : Math.min(cues.length - 1, curIdx + 1)
            : curIdx === -1
              ? cues.length - 1
              : Math.max(0, curIdx - 1);
        const target = cues[nextIdx];
        if (target) {
          setActiveCueId(target.id);
          seekPlayhead(target.startMs);
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    togglePlayTimeline,
    undoCueEdit,
    redoCueEdit,
    cues,
    activeCue?.id,
    setActiveCueId,
    seekPlayhead,
  ]);

  // Compute adaptive ruler ticks & gridlines
  const rulerTicks = useMemo(() => {
    const contentWidthPx = viewportWidth * (zoomPct / 100);
    const pxPerSec = contentWidthPx / Math.max(0.1, totalMs / 1000);
    const majorSteps = [
      500, 1000, 2000, 5000, 10000, 15000, 30000, 60000, 120000, 300000,
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
        label: isMajor && ms < totalMs ? formatRulerSec(ms) : null,
      });
    }
    return ticks;
  }, [viewportWidth, zoomPct, totalMs]);

  const scrubFromClientX = (clientX: number) => {
    const contentEl = contentRef.current;
    if (!contentEl) return;
    const rect = contentEl.getBoundingClientRect();
    const ratio = Math.max(
      0,
      Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)),
    );
    segmentEndMsRef.current = null;
    seekPlayhead(ratio * totalMs);
  };

  // Clip trim drag handlers
  const beginTrimDrag = (
    e: React.PointerEvent<HTMLDivElement>,
    cue: SubtitleCue,
    edge: "start" | "end",
    trackKind: "original" | "prevod",
  ) => {
    e.stopPropagation();
    if (tracks[trackKind].locked) return;
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
    if (!trimDrag) return;
    e.stopPropagation();
    const dx = e.clientX - trimDrag.startClientX;
    const deltaMs = (dx / trimDrag.contentWidthPx) * totalMs;
    if (trimDrag.edge === "start") {
      const nextStart = Math.max(
        0,
        Math.min(trimDrag.origEndMs - 200, Math.round(trimDrag.origStartMs + deltaMs)),
      );
      setTrimDrag({ ...trimDrag, previewStartMs: nextStart });
    } else {
      const nextEnd = Math.min(
        totalMs,
        Math.max(trimDrag.origStartMs + 200, Math.round(trimDrag.origEndMs + deltaMs)),
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
          onClick={() => setActiveCueId(cue.id)}
          onDoubleClick={() => {
            setActiveCueId(cue.id);
            segmentEndMsRef.current = null;
            seekPlayhead(startMs);
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
  const playheadPct = Math.max(
    0,
    Math.min(100, (playheadMs / Math.max(1, totalMs)) * 100),
  );
  const mp3ClipPct = Math.max(
    1,
    Math.min(100, (audioEndMs / Math.max(1, totalMs)) * 100),
  );

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
    <section id="timeline-panel" aria-label="Timeline">
      <audio
        ref={audioRef}
        src={audioSrc || undefined}
        preload="metadata"
        onLoadedMetadata={() => setAudioLoaded(true)}
        onCanPlay={() => setAudioLoaded(true)}
        onError={() => setAudioLoaded(false)}
        onEnded={() => {
          segmentEndMsRef.current = null;
          setIsPlaying(false);
        }}
        className="hidden"
      />

      <div className="tl-toolbar">
        <div className="tl-transport">
          <button
            id="tl-btn-play"
            className={`tl-icon-btn${isPlaying ? " playing" : ""}`}
            type="button"
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
              {formatTimecode(playheadMs, showHours)}
            </span>
            <span className="tl-tc-sep">/</span>
            <span className="tl-tc-tot">
              {formatTimecode(audioEndMs, showHours)}
            </span>
          </div>
        </div>

        {/* Compact Cue Editing Controls */}
        <div className="tl-edit-controls">
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!canUndo}
            onClick={undoCueEdit}
            title={t.table.undo}
          >
            <Undo2 size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!canRedo}
            onClick={redoCueEdit}
            title={t.table.redo}
          >
            <Redo2 size={12} />
          </button>

          <span className="tl-sep" aria-hidden="true" />

          <button
            type="button"
            className="tl-icon-btn"
            disabled={!activeCue}
            onClick={() => activeCue && splitCueAtMidpoint(activeCue.id)}
            title={t.table.splitCue}
          >
            <Scissors size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn"
            disabled={!activeCue || isLastCue}
            onClick={() => activeCue && mergeCueWithNext(activeCue.id)}
            title={t.table.mergeNext}
          >
            <Merge size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn add"
            disabled={!activeProject}
            onClick={addCueAtEnd}
            title={t.table.addCue}
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            className="tl-icon-btn danger"
            disabled={!activeCue}
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
            title={lang === "en" ? "Zoom out" : "Umanji zoom"}
            onClick={() => setZoomPct((z) => Math.max(100, z - 25))}
          >
            −
          </button>
          <input
            id="tl-zoom-slider"
            className="tl-zoom-slider"
            type="range"
            min={100}
            max={500}
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
            title={lang === "en" ? "Zoom in" : "Uvećaj zoom"}
            onClick={() => setZoomPct((z) => Math.min(500, z + 25))}
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

          {/* Track 2: Original transcript */}
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
                <span className="tl-th-title">Original transcript</span>
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
                <span className="tl-th-title">
                  {lang === "en" ? "Translation" : "Prevod"}
                </span>
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
            style={{ width: `${zoomPct}%` }}
          >
            <div
              ref={rulerRef}
              id="tl-ruler"
              onPointerDown={(e) => {
                scrubbingRef.current = true;
                e.currentTarget.setPointerCapture(e.pointerId);
                scrubFromClientX(e.clientX);
              }}
              onPointerMove={(e) => {
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
              {rulerTicks.map((tick) => (
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
              {rulerTicks
                .filter((t) => t.isMajor && t.ms > 0 && t.ms < totalMs)
                .map((tick) => (
                  <div
                    key={`g-${tick.ms}`}
                    className="tl-gridline"
                    style={{ left: `${tick.pct}%` }}
                  />
                ))}
            </div>

            {/* Lane 1: MP3 */}
            <div
              id="lane-mp3"
              className={`tl-lane${tracks.mp3.muted ? " muted" : ""}${
                tracks.mp3.locked ? " locked" : ""
              }`}
            >
              <div
                ref={mp3ClipRef}
                className="mp3-clip"
                style={{ width: `${mp3ClipPct}%` }}
              >
                <span id="mp3-clip-tag" className="mp3-clip-tag">
                  {activeProject?.audioFileName ?? "audio_track.mp3"}
                </span>
                <canvas ref={canvasRef} id="mp3-waveform" />
              </div>
            </div>

            {/* Lane 2: Original transcript */}
            <div
              id="lane-original"
              className={`tl-lane${tracks.original.hidden ? " muted" : ""}${
                tracks.original.locked ? " locked" : ""
              }`}
            >
              {renderClips("original")}
            </div>

            {/* Lane 3: Prevod */}
            <div
              id="lane-prevod"
              className={`tl-lane${tracks.prevod.hidden ? " muted" : ""}${
                tracks.prevod.locked ? " locked" : ""
              }`}
            >
              {renderClips("prevod")}
            </div>

            {/* Vertical Playhead */}
            <div id="tl-playhead" style={{ left: `${playheadPct}%` }} />
          </div>
        </div>
      </div>
    </section>
  );
}
