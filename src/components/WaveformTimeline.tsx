import { Merge, Plus, Redo2, Scissors, Trash2, Undo2 } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime } from "../lib/subtitleUtils";
import { readAudioPreviewBase64 } from "../lib/tauri";
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

function decodeBase64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** Extracts 99th-percentile normalized acoustic amplitude buckets from an AudioBuffer. */
function extractAudioPeaks(
  buffer: AudioBuffer,
  bucketCount = 2400,
): Float32Array {
  const totalSamples = buffer.length;
  if (totalSamples === 0 || bucketCount <= 0 || buffer.numberOfChannels === 0) {
    return new Float32Array(0);
  }

  const channels: Float32Array[] = [];
  for (let c = 0; c < buffer.numberOfChannels; c += 1) {
    channels.push(buffer.getChannelData(c));
  }

  const count = Math.min(bucketCount, totalSamples);
  const raw = new Float32Array(count);

  for (let b = 0; b < count; b += 1) {
    const start = Math.floor((b / count) * totalSamples);
    const end = Math.max(
      start + 1,
      Math.floor(((b + 1) / count) * totalSamples),
    );
    const step = Math.max(1, Math.floor((end - start) / 64));

    let peak = 0;
    let sumSq = 0;
    let samples = 0;
    for (let i = start; i < end; i += step) {
      let s = 0;
      for (let c = 0; c < channels.length; c += 1) {
        const v = Math.abs(channels[c][i] ?? 0);
        if (v > s) s = v;
      }
      if (s > peak) peak = s;
      sumSq += s * s;
      samples += 1;
    }

    const rms = samples > 0 ? Math.sqrt(sumSq / samples) : 0;
    raw[b] = 0.55 * peak + 0.45 * rms;
  }

  const sorted = Float32Array.from(raw).sort();
  const p99Idx = Math.min(
    sorted.length - 1,
    Math.floor(sorted.length * 0.99),
  );
  const scale = sorted[p99Idx] > 0.005 ? sorted[p99Idx] : 1;

  const out = new Float32Array(count);
  for (let b = 0; b < count; b += 1) {
    out[b] =
      raw[b] > 0.002 ? Math.min(1, Math.max(0.02, raw[b] / scale)) : 0.02;
  }
  return out;
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
  const [audioBlobUrl, setAudioBlobUrl] = useState<string>("");
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
  const audioRawBytesRef = useRef<ArrayBuffer | null>(null);
  const audioPeaksRef = useRef<Float32Array | null>(null);
  const webAudioCtxRef = useRef<AudioContext | null>(null);
  const webAudioSourceRef = useRef<AudioBufferSourceNode | null>(null);
  const webAudioGainRef = useRef<GainNode | null>(null);
  const webAudioBufferRef = useRef<AudioBuffer | null>(null);
  const webAudioStartCtxTimeRef = useRef(0);
  const webAudioStartOffsetSecRef = useRef(0);

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
  const audioEndMs = useMemo(
    () =>
      Math.max(
        activeProject?.durationMs ?? 0,
        liveAudioDurationMs,
        maxCueEndMs,
        1000,
      ),
    [activeProject?.durationMs, liveAudioDurationMs, maxCueEndMs],
  );
  const totalMs = useMemo(
    () => Math.max(Math.round(audioEndMs * 1.04), maxCueEndMs + 500, 1000),
    [audioEndMs, maxCueEndMs],
  );

  const audioEndMsRef = useRef(audioEndMs);
  audioEndMsRef.current = audioEndMs;

  const activeCue = useMemo(() => {
    if (cues.length === 0 || activeCueId === null) return null;
    return cues.find((c) => c.id === activeCueId) ?? null;
  }, [cues, activeCueId]);

  const stopWebAudio = useCallback(() => {
    if (webAudioSourceRef.current) {
      try {
        webAudioSourceRef.current.stop();
        webAudioSourceRef.current.disconnect();
      } catch {
        // Ignore already stopped source
      }
      webAudioSourceRef.current = null;
    }
  }, []);

  // Load real audio bytes into a blob: ObjectURL (works natively in Linux WebKitGTK & Windows WebView2)
  useEffect(() => {
    let active = true;
    let createdUrl = "";

    stopWebAudio();
    webAudioBufferRef.current = null;
    audioRawBytesRef.current = null;
    audioPeaksRef.current = null;
    setLiveAudioDurationMs(0);
    setAudioBlobUrl("");
    setWaveformVersion((v) => v + 1);

    const path = activeProject?.audioPath?.trim() ?? "";
    if (!path || /\.(srt|vtt)$/i.test(path)) {
      return;
    }

    const isDemoProject =
      !isTauriRuntime() || activeProject?.jobId === "demo-session-01";

    const decodePeaksFromBuffer = async (rawBuf: ArrayBuffer) => {
      if (typeof AudioContext === "undefined") {
        return;
      }
      try {
        if (!webAudioCtxRef.current) {
          webAudioCtxRef.current = new AudioContext();
        }
        const decoded = await webAudioCtxRef.current.decodeAudioData(
          rawBuf.slice(0),
        );
        if (!active) return;
        webAudioBufferRef.current = decoded;
        if (decoded.duration > 0 && !activeProject?.durationMs) {
          setLiveAudioDurationMs(Math.round(decoded.duration * 1000));
        }
        audioPeaksRef.current = extractAudioPeaks(decoded, 2400);
        setWaveformVersion((v) => v + 1);
      } catch {
        // Ignore decode failures for unsupported codecs; fallback rendering remains active
      }
    };

    const loadPreview = async () => {
      try {
        const preview = await readAudioPreviewBase64(path);
        if (!active) return;
        const bytes = decodeBase64ToBytes(preview.base64Data);
        const rawBuf = bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ) as ArrayBuffer;
        audioRawBytesRef.current = rawBuf;
        const blob = new Blob([rawBuf], { type: preview.mimeType });
        createdUrl = URL.createObjectURL(blob);
        setAudioBlobUrl(createdUrl);
        void decodePeaksFromBuffer(rawBuf);
      } catch {
        if (!active) return;
        if (isDemoProject) {
          const demoBuf = buildDemoWavBuffer(
            audioEndMsRef.current,
            cuesRef.current,
          );
          audioRawBytesRef.current = demoBuf;
          const blob = new Blob([demoBuf], { type: "audio/wav" });
          createdUrl = URL.createObjectURL(blob);
          setAudioBlobUrl(createdUrl);
          void decodePeaksFromBuffer(demoBuf);
        }
      }
    };

    void loadPreview();

    return () => {
      active = false;
      stopWebAudio();
      if (createdUrl) {
        URL.revokeObjectURL(createdUrl);
      }
    };
  }, [activeProject?.jobId, activeProject?.audioPath, stopWebAudio]);

  // Sync mute state onto <audio> and Web Audio gain
  useEffect(() => {
    if (audioRef.current) {
      audioRef.current.muted = tracks.mp3.muted;
    }
    if (webAudioGainRef.current && webAudioCtxRef.current) {
      webAudioGainRef.current.gain.setValueAtTime(
        tracks.mp3.muted ? 0 : 1,
        webAudioCtxRef.current.currentTime,
      );
    }
  }, [tracks.mp3.muted]);

  const seekPlayhead = useCallback(
    (ms: number, syncCue = true) => {
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

      if (audioRef.current && audioBlobUrl) {
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

      // If Web Audio fallback is actively playing, restart buffer from new offset
      if (webAudioSourceRef.current && webAudioBufferRef.current) {
        const startOffsetSec = Math.max(
          0,
          Math.min(
            webAudioBufferRef.current.duration - 0.01,
            clamped / 1000,
          ),
        );
        stopWebAudio();
        const ctx = webAudioCtxRef.current;
        const gain = webAudioGainRef.current;
        if (ctx && gain) {
          const src = ctx.createBufferSource();
          src.buffer = webAudioBufferRef.current;
          src.connect(gain);
          webAudioStartCtxTimeRef.current = ctx.currentTime;
          webAudioStartOffsetSecRef.current = startOffsetSec;
          src.start(0, startOffsetSec);
          webAudioSourceRef.current = src;
        }
      }
    },
    [audioBlobUrl, setActiveCueId, stopWebAudio],
  );

  // When switching projects, stop playback and position playhead on active/first cue start
  useEffect(() => {
    const currentJobId = activeProject?.jobId ?? null;
    const currentPhase = activeProject?.phase ?? null;
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
      stopWebAudio();
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
    cues.length,
    setActiveCueId,
    stopWebAudio,
  ]);

  // Handle external playbackRequest (e.g. playCueSegment)
  useEffect(() => {
    if (!playbackRequest) return;
    segmentEndMsRef.current = playbackRequest.endMs;
    activeCueIdRef.current = playbackRequest.cueId;
    setActiveCueId(playbackRequest.cueId);
    seekPlayhead(playbackRequest.startMs, false);
    setIsPlaying(true);
  }, [playbackRequest, seekPlayhead, setActiveCueId]);

  // Playback loop (<audio> blob primary + Web Audio API fallback + rAF clock)
  useEffect(() => {
    if (!isPlaying) {
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      stopWebAudio();
      if (audioRef.current && !audioRef.current.paused) {
        audioRef.current.pause();
      }
      return;
    }

    let cancelled = false;
    let clockSource: "audio" | "webaudio" | "raf" = "raf";

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

        let nextMs: number;
        if (clockSource === "audio" && audioRef.current) {
          nextMs = audioRef.current.currentTime * 1000;
        } else if (clockSource === "webaudio" && webAudioCtxRef.current) {
          nextMs =
            (webAudioStartOffsetSecRef.current +
              (webAudioCtxRef.current.currentTime -
                webAudioStartCtxTimeRef.current)) *
            1000;
        } else {
          const dt = Math.max(0, now - lastRafTickRef.current);
          lastRafTickRef.current = now;
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
          stopWebAudio();
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

    const startWebAudioFallback = async (): Promise<boolean> => {
      const rawBuf = audioRawBytesRef.current;
      if (!rawBuf || typeof AudioContext === "undefined") {
        return false;
      }
      try {
        if (!webAudioCtxRef.current) {
          webAudioCtxRef.current = new AudioContext();
        }
        const ctx = webAudioCtxRef.current;
        if (ctx.state === "suspended") {
          await ctx.resume();
        }
        if (!webAudioGainRef.current) {
          const gain = ctx.createGain();
          gain.connect(ctx.destination);
          webAudioGainRef.current = gain;
        }
        webAudioGainRef.current.gain.setValueAtTime(
          tracksRef.current.mp3.muted ? 0 : 1,
          ctx.currentTime,
        );
        if (!webAudioBufferRef.current) {
          webAudioBufferRef.current = await ctx.decodeAudioData(
            rawBuf.slice(0),
          );
          if (!activeProject?.durationMs && webAudioBufferRef.current.duration) {
            setLiveAudioDurationMs(
              Math.round(webAudioBufferRef.current.duration * 1000),
            );
          }
        }
        if (cancelled || !webAudioBufferRef.current) return false;
        stopWebAudio();
        const offsetSec = Math.max(
          0,
          Math.min(
            Math.max(0, webAudioBufferRef.current.duration - 0.02),
            playheadRef.current / 1000,
          ),
        );
        const src = ctx.createBufferSource();
        src.buffer = webAudioBufferRef.current;
        src.connect(webAudioGainRef.current);
        webAudioStartCtxTimeRef.current = ctx.currentTime;
        webAudioStartOffsetSecRef.current = offsetSec;
        src.start(0, offsetSec);
        webAudioSourceRef.current = src;
        return true;
      } catch {
        return false;
      }
    };

    const audioEl = audioRef.current;
    if (audioBlobUrl && audioEl && !audioEl.error) {
      try {
        if (
          Number.isFinite(audioEl.duration) &&
          audioEl.duration > 0 &&
          Math.abs(audioEl.currentTime * 1000 - playheadRef.current) > 80
        ) {
          audioEl.currentTime = Math.min(
            audioEl.duration,
            playheadRef.current / 1000,
          );
        }
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
            .catch(async () => {
              const ok = await startWebAudioFallback();
              if (!cancelled) {
                clockSource = ok ? "webaudio" : "raf";
                startClock();
              }
            });
        } else {
          clockSource = "audio";
          startClock();
        }
      } catch {
        void startWebAudioFallback().then((ok) => {
          if (!cancelled) {
            clockSource = ok ? "webaudio" : "raf";
            startClock();
          }
        });
      }
    } else if (audioRawBytesRef.current) {
      void startWebAudioFallback().then((ok) => {
        if (!cancelled) {
          clockSource = ok ? "webaudio" : "raf";
          startClock();
        }
      });
    } else {
      startClock();
    }

    return () => {
      cancelled = true;
      stopWebAudio();
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
    };
  }, [
    isPlaying,
    audioBlobUrl,
    activeProject?.durationMs,
    setActiveCueId,
    stopWebAudio,
  ]);

  const togglePlayTimeline = useCallback(() => {
    if (isPlaying) {
      segmentEndMsRef.current = null;
      setIsPlaying(false);
      return;
    }
    segmentEndMsRef.current = null;
    if (playheadRef.current >= audioEndMsRef.current) {
      seekPlayhead(0, true);
    }
    // Kick user-gesture audio unlock synchronously inside click/Space handler
    if (webAudioCtxRef.current?.state === "suspended") {
      void webAudioCtxRef.current.resume().catch(() => {});
    }
    if (audioRef.current && audioBlobUrl && !audioRef.current.error) {
      audioRef.current.muted = tracksRef.current.mp3.muted;
    }
    setIsPlaying(true);
  }, [isPlaying, audioBlobUrl, seekPlayhead]);

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
    const peaks = audioPeaksRef.current;

    if (peaks && peaks.length > 0) {
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

        const tMs = frac0 * audioEndMs;
        const inCue = cues.some((c) => tMs >= c.startMs && tMs <= c.endMs);
        ctx.fillStyle = inCue
          ? "rgba(56, 189, 248, 0.85)"
          : "rgba(56, 189, 248, 0.42)";

        const barH = Math.max(2, amp * maxHalf * 2);
        ctx.fillRect(x, midY - barH / 2, barW, barH);
      }
      return;
    }

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
  }, [activeProject?.words, cues, audioEndMs, waveformVersion]);

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
        src={audioBlobUrl || undefined}
        preload="auto"
        onLoadedMetadata={(e) => {
          const el = e.currentTarget;
          if (Number.isFinite(el.duration) && el.duration > 0) {
            if (!activeProject?.durationMs) {
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
              onPointerDown={(e) => {
                if (e.target !== e.currentTarget) return;
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
              {renderClips("original")}
            </div>

            {/* Lane 3: Prevod */}
            <div
              id="lane-prevod"
              className={`tl-lane${tracks.prevod.hidden ? " muted" : ""}${
                tracks.prevod.locked ? " locked" : ""
              }`}
              onPointerDown={(e) => {
                if (e.target !== e.currentTarget) return;
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
