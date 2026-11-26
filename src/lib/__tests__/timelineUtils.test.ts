// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
import {
  MAX_CANVAS_BACKING_WIDTH,
  MAX_TIMELINE_ZOOM_PCT,
  MIN_TIMELINE_ZOOM_PCT,
  clearMediaSession,
  computeButtonZoomPct,
  computeFollowPlayheadScrollLeft,
  computeSafeCanvasSize,
  computeWheelZoomPct,
  findCueForTime,
  formatRulerTimestamp,
  inferAudioMimeType,
  isAnyModalOpen,
  isTextEntryElement,
  resolveTimelineMediaState,
  shouldCaptureTimelineSpace,
} from "../timelineUtils.ts";

describe("timelineUtils", () => {
  it("zooms smoothly up to 2000% on wheel up and down to 100% on wheel down", () => {
    let zoom = MIN_TIMELINE_ZOOM_PCT;
    assert.equal(zoom, 100);

    for (let i = 0; i < 60; i += 1) {
      zoom = computeWheelZoomPct(zoom, -100);
      assert.equal(zoom % 25, 0);
    }
    assert.equal(zoom, MAX_TIMELINE_ZOOM_PCT);
    assert.equal(MAX_TIMELINE_ZOOM_PCT, 2000);
    assert.equal(computeWheelZoomPct(MAX_TIMELINE_ZOOM_PCT, -120), 2000);

    for (let i = 0; i < 60; i += 1) {
      zoom = computeWheelZoomPct(zoom, 100);
      assert.equal(zoom % 25, 0);
    }
    assert.equal(zoom, MIN_TIMELINE_ZOOM_PCT);
    assert.equal(computeWheelZoomPct(MIN_TIMELINE_ZOOM_PCT, 120), 100);
    assert.equal(computeWheelZoomPct(350, 0), 350);
  });

  it("steps cleanly between 100% and 2000% for toolbar +/- buttons", () => {
    assert.equal(computeButtonZoomPct(100, "out"), 100);
    assert.equal(computeButtonZoomPct(100, "in"), 125);
    assert.equal(computeButtonZoomPct(400, "in"), 450);
    assert.equal(computeButtonZoomPct(400, "out"), 375);
    assert.equal(computeButtonZoomPct(450, "out"), 400);
    assert.equal(computeButtonZoomPct(1000, "in"), 1100);
    assert.equal(computeButtonZoomPct(1100, "out"), 1000);
    assert.equal(computeButtonZoomPct(2000, "in"), 2000);
  });

  it("clamps canvas backing width to 16384px at 2000% zoom and scales X accordingly", () => {
    const sized = computeSafeCanvasSize(28000, 58, 2);
    assert.equal(sized.backingWidth, MAX_CANVAS_BACKING_WIDTH);
    assert.equal(sized.backingHeight, 116);
    assert.equal(sized.scaleX, 16384 / 28000);
    assert.equal(sized.scaleY, 2);

    const normal = computeSafeCanvasSize(1000, 58, 2);
    assert.equal(normal.backingWidth, 2000);
    assert.equal(normal.scaleX, 2);
  });

  it("formats ruler timestamps with tenths for high-zoom sub-second ticks", () => {
    assert.equal(formatRulerTimestamp(0), "00:00");
    assert.equal(formatRulerTimestamp(200), "00:00.2");
    assert.equal(formatRulerTimestamp(1500), "00:01.5");
    assert.equal(formatRulerTimestamp(65000), "01:05");
    assert.equal(formatRulerTimestamp(3661500), "1:01:01.5");
  });

  it("resolves timeline media state across empty startup, queued audio, completed project, and standalone SRT/VTT", () => {
    // 1. Empty startup (nothing loaded)
    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: null,
        selectedQueueItem: null,
      }),
      {
        hasLoadedProjectOrFile: false,
        hasAudioSource: false,
        audioPath: null,
        audioFileName: null,
        isSubtitleOnly: false,
      },
    );

    // 2. Queued audio/video file before ASR (both with null activeProject and placeholder cues: [])
    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: null,
        selectedQueueItem: {
          audioPath: "/media/interview_ep1.webm",
          audioFileName: "interview_ep1.webm",
        },
      }),
      {
        hasLoadedProjectOrFile: true,
        hasAudioSource: true,
        audioPath: "/media/interview_ep1.webm",
        audioFileName: "interview_ep1.webm",
        isSubtitleOnly: false,
      },
    );
    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: {
          audioPath: "/media/interview_ep1.mp3",
          audioFileName: "interview_ep1.mp3",
          cues: [],
        },
        selectedQueueItem: {
          audioPath: "/media/interview_ep1.mp3",
          audioFileName: "interview_ep1.mp3",
        },
      }),
      {
        hasLoadedProjectOrFile: true,
        hasAudioSource: true,
        audioPath: "/media/interview_ep1.mp3",
        audioFileName: "interview_ep1.mp3",
        isSubtitleOnly: false,
      },
    );

    // 3. Completed audio + subtitle project
    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: {
          audioPath: "C:\\Studio\\Podcast_Ep14.mp4",
          audioFileName: "Podcast_Ep14.mp4",
          cues: [
            {
              id: 1,
              startMs: 500,
              endMs: 3200,
              sourceText: "Hello world",
              translatedText: "Zdravo svete",
              cps: 4.4,
              warnings: [],
            },
          ],
        },
        selectedQueueItem: null,
      }),
      {
        hasLoadedProjectOrFile: true,
        hasAudioSource: true,
        audioPath: "C:\\Studio\\Podcast_Ep14.mp4",
        audioFileName: "Podcast_Ep14.mp4",
        isSubtitleOnly: false,
      },
    );

    // 4. Standalone imported .srt / .vtt without companion audio
    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: {
          audioPath: "/subs/movie_subtitles.srt",
          audioFileName: "movie_subtitles.srt",
          cues: [
            {
              id: 1,
              startMs: 1000,
              endMs: 4000,
              sourceText: "Imported subtitle",
              translatedText: "",
              cps: 5.7,
              warnings: [],
            },
          ],
        },
        selectedQueueItem: null,
      }),
      {
        hasLoadedProjectOrFile: true,
        hasAudioSource: false,
        audioPath: null,
        audioFileName: null,
        isSubtitleOnly: true,
      },
    );

    assert.deepEqual(
      resolveTimelineMediaState({
        activeProject: {
          audioPath: "captions.VTT",
          audioFileName: "captions.VTT",
          cues: [],
        },
        selectedQueueItem: null,
      }),
      {
        hasLoadedProjectOrFile: true,
        hasAudioSource: false,
        audioPath: null,
        audioFileName: null,
        isSubtitleOnly: true,
      },
    );
  });

  it("infers MIME types from RIFF/ID3/OggS/fLaC magic headers and falls back to extensions", () => {
    const wavBytes = new Uint8Array([
      0x52, 0x49, 0x46, 0x46, 0x24, 0x00, 0x00, 0x00, 0x57, 0x41, 0x56, 0x45,
    ]);
    // Transcoded WAV payload from an .mp4 or .mkv path must resolve to audio/wav
    assert.equal(inferAudioMimeType("/tmp/video.mp4", wavBytes.buffer), "audio/wav");
    assert.equal(inferAudioMimeType("/tmp/clip.mkv", wavBytes), "audio/wav");

    const id3Bytes = new Uint8Array([0x49, 0x44, 0x33, 0x04]);
    assert.equal(inferAudioMimeType("unknown.bin", id3Bytes), "audio/mpeg");

    const oggBytes = new Uint8Array([0x4f, 0x67, 0x67, 0x53]);
    assert.equal(inferAudioMimeType("voice.opus", oggBytes), "audio/ogg");

    const flacBytes = new Uint8Array([0x66, 0x4c, 0x61, 0x43]);
    assert.equal(inferAudioMimeType("track.dat", flacBytes), "audio/flac");

    // Extension fallbacks
    assert.equal(inferAudioMimeType("/a/b/speech.MP3"), "audio/mpeg");
    assert.equal(inferAudioMimeType("/a/b/speech.wav"), "audio/wav");
    assert.equal(inferAudioMimeType("/a/b/podcast.m4a"), "audio/mp4");
    assert.equal(inferAudioMimeType("/a/b/clip.aac"), "audio/aac");
    assert.equal(inferAudioMimeType("/a/b/clip.webm"), "audio/webm");
  });

  it("captures Space transport exclusively outside text entry fields and yields to modals (V05-L1)", () => {
    const closedModals = {
      isSettingsOpen: false,
      isExportOpen: false,
      isSetupOpen: false,
      isUpdaterOpen: false,
      isCheckpointsOpen: false,
      isBugReportOpen: false,
      onlineConfirm: { open: false },
    };

    assert.equal(isAnyModalOpen(closedModals), false);
    assert.equal(
      isAnyModalOpen({ ...closedModals, onlineConfirm: { open: true } }),
      true,
    );

    // Focused <button> / checkbox / range slider are NOT text-entry elements -> Space must trigger transport
    assert.equal(isTextEntryElement({ tagName: "BUTTON" }), false);
    assert.equal(
      isTextEntryElement({ tagName: "INPUT", type: "checkbox" }),
      false,
    );
    assert.equal(
      isTextEntryElement({ tagName: "INPUT", type: "range" }),
      false,
    );
    assert.equal(isTextEntryElement({ tagName: "TEXTAREA" }), true);
    assert.equal(
      isTextEntryElement({ tagName: "INPUT", type: "text" }),
      true,
    );
    assert.equal(isTextEntryElement({ isContentEditable: true }), true);

    assert.equal(
      shouldCaptureTimelineSpace({
        code: "Space",
        ctrlKey: false,
        metaKey: false,
        altKey: false,
        target: { tagName: "BUTTON" },
        modalState: closedModals,
      }),
      true,
    );

    assert.equal(
      shouldCaptureTimelineSpace({
        code: "Space",
        ctrlKey: false,
        metaKey: false,
        altKey: false,
        target: { tagName: "TEXTAREA" },
        modalState: closedModals,
      }),
      false,
    );

    assert.equal(
      shouldCaptureTimelineSpace({
        code: "Space",
        ctrlKey: false,
        metaKey: false,
        altKey: false,
        target: { tagName: "BUTTON" },
        modalState: { ...closedModals, onlineConfirm: { open: true } },
      }),
      false,
    );
  });

  it("holds nearest preceding cue across breathing gaps and pages viewport during Follow Playhead", () => {
    const cues = [
      {
        id: 1,
        startMs: 500,
        endMs: 2000,
        sourceText: "First",
        translatedText: "Prvi",
        cps: 5,
        warnings: [],
      },
      {
        id: 2,
        startMs: 2500,
        endMs: 4000,
        sourceText: "Second",
        translatedText: "Drugi",
        cps: 5,
        warnings: [],
      },
    ];

    assert.equal(findCueForTime(cues, 2200, false), null);
    assert.equal(findCueForTime(cues, 2200, true)?.id, 1);
    assert.equal(findCueForTime(cues, 100, true)?.id, 1);

    assert.equal(
      computeFollowPlayheadScrollLeft({
        playheadMs: 1000,
        totalMs: 10000,
        contentWidthPx: 4000,
        viewportWidthPx: 1000,
        currentScrollLeft: 0,
      }),
      null,
    );

    const scrolled = computeFollowPlayheadScrollLeft({
      playheadMs: 3000,
      totalMs: 10000,
      contentWidthPx: 4000,
      viewportWidthPx: 1000,
      currentScrollLeft: 0,
    });
    assert.equal(scrolled, 1050);
  });

  it("clears navigator.mediaSession playbackState and all action handlers cleanly (V05-L1)", () => {
    const clearedActions: string[] = [];
    const mockSession = {
      playbackState: "playing",
      metadata: { title: "clip.mp3" },
      setActionHandler(action: string, handler: unknown) {
        if (handler === null) {
          clearedActions.push(action);
        }
      },
    };

    clearMediaSession(mockSession);
    assert.equal(mockSession.playbackState, "none");
    assert.equal(mockSession.metadata, null);
    assert.ok(clearedActions.includes("play"));
    assert.ok(clearedActions.includes("pause"));
    assert.ok(clearedActions.includes("stop"));
  });
});
