// @ts-ignore Node built-in strict assert without @types/node
import assert from "node:assert/strict";
// @ts-ignore Node built-in test runner without @types/node
import { describe, it } from "node:test";
import {
  MAX_CANVAS_BACKING_WIDTH,
  MAX_TIMELINE_ZOOM_PCT,
  MIN_TIMELINE_ZOOM_PCT,
  computeButtonZoomPct,
  computeSafeCanvasSize,
  computeWheelZoomPct,
  formatRulerTimestamp,
  resolveTimelineMediaState,
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
});
