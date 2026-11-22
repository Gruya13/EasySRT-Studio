import { convertFileSrc } from "@tauri-apps/api/core";
import { Pause, Play, Square, Volume2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { formatTimestampSrt, isTauriRuntime } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";

export function AudioSegmentPlayer() {
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const playbackRequest = useAppStore((s) => s.playbackRequest);
  const nudgeCueTime = useAppStore((s) => s.nudgeCueTime);

  const audioRef = useRef<HTMLAudioElement | null>(null);
  const segmentEndSecRef = useRef<number | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTimeMs, setCurrentTimeMs] = useState(0);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const audioSrc = useMemo(() => {
    if (!activeProject?.audioPath) {
      return "";
    }
    if (isTauriRuntime()) {
      try {
        return convertFileSrc(activeProject.audioPath);
      } catch {
        return "";
      }
    }
    return "";
  }, [activeProject?.audioPath]);

  const activeCue = useMemo(() => {
    if (!activeProject || activeProject.cues.length === 0) {
      return null;
    }
    if (activeCueId !== null) {
      return (
        activeProject.cues.find((c) => c.id === activeCueId) ??
        activeProject.cues[0]
      );
    }
    return activeProject.cues[0];
  }, [activeProject, activeCueId]);

  useEffect(() => {
    if (!playbackRequest) {
      return;
    }
    const audio = audioRef.current;
    const startSec = Math.max(0, playbackRequest.startMs / 1000);
    const endSec = Math.max(startSec + 0.1, playbackRequest.endMs / 1000);
    segmentEndSecRef.current = endSec;
    setCurrentTimeMs(playbackRequest.startMs);

    if (audio && audioSrc) {
      audio.currentTime = startSec;
      void audio.play().catch(() => {
        setIsPlaying(false);
      });
    }
  }, [playbackRequest, audioSrc]);

  const handleTimeUpdate = () => {
    const audio = audioRef.current;
    if (!audio) {
      return;
    }
    setCurrentTimeMs(Math.round(audio.currentTime * 1000));
    if (
      segmentEndSecRef.current !== null &&
      audio.currentTime >= segmentEndSecRef.current
    ) {
      audio.pause();
      segmentEndSecRef.current = null;
      setIsPlaying(false);
    }
  };

  const togglePlayPause = () => {
    const audio = audioRef.current;
    if (!audio || !activeCue) {
      return;
    }
    if (isPlaying) {
      audio.pause();
      setIsPlaying(false);
      return;
    }
    segmentEndSecRef.current = activeCue.endMs / 1000;
    audio.currentTime = activeCue.startMs / 1000;
    void audio.play().catch(() => {
      setIsPlaying(false);
    });
  };

  const stopPlayback = () => {
    const audio = audioRef.current;
    if (!audio) {
      return;
    }
    audio.pause();
    if (activeCue) {
      audio.currentTime = activeCue.startMs / 1000;
      setCurrentTimeMs(activeCue.startMs);
    }
    segmentEndSecRef.current = null;
    setIsPlaying(false);
  };

  if (!activeProject) {
    return null;
  }

  return (
    <footer className="sticky bottom-0 z-20 border-t border-slate-800/90 bg-slate-950/95 px-4 py-2.5 backdrop-blur-md">
      <audio
        ref={audioRef}
        src={audioSrc || undefined}
        onPlay={() => setIsPlaying(true)}
        onPause={() => setIsPlaying(false)}
        onEnded={() => setIsPlaying(false)}
        onTimeUpdate={handleTimeUpdate}
        className="hidden"
      />

      <div className="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-3">
        {/* Left: Playback controls & current cue text preview */}
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              disabled={!activeCue}
              onClick={togglePlayPause}
              className="flex h-9 w-9 items-center justify-center rounded-xl bg-gradient-to-br from-emerald-500 to-cyan-500 text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-40"
              title={isPlaying ? t.player.pause : t.player.play}
            >
              {isPlaying ? (
                <Pause className="h-4 w-4 fill-current" />
              ) : (
                <Play className="h-4 w-4 fill-current" />
              )}
            </button>

            <button
              type="button"
              disabled={!activeCue}
              onClick={stopPlayback}
              className="flex h-9 w-9 items-center justify-center rounded-xl border border-slate-800 bg-slate-900 text-slate-300 transition hover:bg-slate-800 disabled:opacity-40"
              title={t.player.stopSegment}
            >
              <Square className="h-3.5 w-3.5 fill-current" />
            </button>
          </div>

          <div className="min-w-0 flex-1">
            {activeCue ? (
              <div className="flex flex-col">
                <div className="flex items-center gap-2 font-mono text-[11px] text-cyan-300">
                  <Volume2 className="h-3.5 w-3.5 text-emerald-400" />
                  <span>
                    {t.player.activeCue} #{activeCue.id}
                  </span>
                  <span>•</span>
                  <span>
                    {formatTimestampSrt(activeCue.startMs)} →{" "}
                    {formatTimestampSrt(activeCue.endMs)}
                  </span>
                  {currentTimeMs > 0 && (
                    <>
                      <span>•</span>
                      <span className="text-slate-400">
                        {formatTimestampSrt(currentTimeMs)}
                      </span>
                    </>
                  )}
                </div>
                <div className="truncate text-xs font-medium text-slate-200">
                  {activeCue.translatedText.trim() || activeCue.sourceText}
                </div>
              </div>
            ) : (
              <span className="text-xs text-slate-500">{t.player.noAudio}</span>
            )}
          </div>
        </div>

        {/* Right: -0.5s / +0.5s fine-tune buttons for active cue */}
        {activeCue && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="mr-1 hidden text-[11px] text-slate-400 lg:inline">
              {t.player.fineTuneLabel}:
            </span>
            <button
              type="button"
              onClick={() => nudgeCueTime(activeCue.id, "start", -500)}
              className="rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1 font-mono text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/40 hover:text-cyan-200"
            >
              {t.player.startMinus}
            </button>
            <button
              type="button"
              onClick={() => nudgeCueTime(activeCue.id, "start", 500)}
              className="rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1 font-mono text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/40 hover:text-cyan-200"
            >
              {t.player.startPlus}
            </button>
            <button
              type="button"
              onClick={() => nudgeCueTime(activeCue.id, "end", -500)}
              className="rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1 font-mono text-[11px] font-medium text-slate-300 transition hover:border-emerald-500/40 hover:text-emerald-200"
            >
              {t.player.endMinus}
            </button>
            <button
              type="button"
              onClick={() => nudgeCueTime(activeCue.id, "end", 500)}
              className="rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1 font-mono text-[11px] font-medium text-slate-300 transition hover:border-emerald-500/40 hover:text-emerald-200"
            >
              {t.player.endPlus}
            </button>
          </div>
        )}
      </div>
    </footer>
  );
}
