import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import {
  formatTimestampSrt,
  normalizeTranslationStyle,
} from "../lib/subtitleUtils";
import {
  EDITOR_SOURCE_FONT_DEFAULT,
  EDITOR_SOURCE_FONT_MAX,
  EDITOR_SOURCE_FONT_MIN,
  EDITOR_TRANSLATED_FONT_DEFAULT,
  EDITOR_TRANSLATED_FONT_MAX,
  EDITOR_TRANSLATED_FONT_MIN,
  clampEditorFontSize,
  getTranslationStyleCacheStatus,
  useAppStore,
} from "../stores/appStore";
import type { TranslationStyle } from "../types/models";

interface DraftSessionSnapshot {
  cueKey: string;
  jobId: string | null;
  cueId: number | null;
  sourceText: string;
  translatedText: string;
  origDraft: string;
  prevDraft: string;
}

export function DualEditorBand() {
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const isRunningBatch = useAppStore((s) => s.isRunningBatch);

  const updateCue = useAppStore((s) => s.updateCue);
  const switchTranslationStyle = useAppStore((s) => s.switchTranslationStyle);
  const setEditorFontSize = useAppStore((s) => s.setEditorFontSize);
  const translateActiveProject = useAppStore((s) => s.translateActiveProject);
  const polishActiveProject = useAppStore((s) => s.polishActiveProject);
  const condenseActiveProjectCps = useAppStore(
    (s) => s.condenseActiveProjectCps,
  );

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const sourceFontSize = clampEditorFontSize(
    "source",
    config.editorSourceFontSize ?? EDITOR_SOURCE_FONT_DEFAULT,
  );
  const translatedFontSize = clampEditorFontSize(
    "translated",
    config.editorTranslatedFontSize ?? EDITOR_TRANSLATED_FONT_DEFAULT,
  );

  const sourceFontSizeRef = useRef(sourceFontSize);
  sourceFontSizeRef.current = sourceFontSize;
  const translatedFontSizeRef = useRef(translatedFontSize);
  translatedFontSizeRef.current = translatedFontSize;

  const activeCue = useMemo(() => {
    if (
      !activeProject ||
      activeProject.cues.length === 0 ||
      activeCueId === null
    ) {
      return null;
    }
    return activeProject.cues.find((c) => c.id === activeCueId) ?? null;
  }, [activeProject, activeCueId]);

  const activeStyle: TranslationStyle = normalizeTranslationStyle(
    activeProject?.options.translationStyle ?? jobOptions.translationStyle,
  );
  const styleCache = useMemo(
    () => getTranslationStyleCacheStatus(activeProject),
    [activeProject],
  );

  const cueKey = `${activeProject?.jobId ?? ""}:${activeCue?.id ?? ""}`;
  const sourceKey = `${cueKey}:${activeCue?.sourceText ?? ""}`;
  const transKey = `${cueKey}:${activeCue?.translatedText ?? ""}`;

  const [origDraft, setOrigDraft] = useState(activeCue?.sourceText ?? "");
  const [prevDraft, setPrevDraft] = useState(activeCue?.translatedText ?? "");
  const [origSavedFlash, setOrigSavedFlash] = useState(false);
  const [prevSavedFlash, setPrevSavedFlash] = useState(false);

  const origCardRef = useRef<HTMLElement | null>(null);
  const prevCardRef = useRef<HTMLElement | null>(null);
  const origStageRef = useRef<HTMLDivElement | null>(null);
  const prevStageRef = useRef<HTMLDivElement | null>(null);
  const origTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const prevTextareaRef = useRef<HTMLTextAreaElement | null>(null);

  const origFlashTimerRef = useRef<number | null>(null);
  const prevFlashTimerRef = useRef<number | null>(null);

  const draftSessionRef = useRef<DraftSessionSnapshot>({
    cueKey,
    jobId: activeProject?.jobId ?? null,
    cueId: activeCue?.id ?? null,
    sourceText: activeCue?.sourceText ?? "",
    translatedText: activeCue?.translatedText ?? "",
    origDraft: activeCue?.sourceText ?? "",
    prevDraft: activeCue?.translatedText ?? "",
  });

  const lastSyncedSourceKeyRef = useRef(sourceKey);
  const lastSyncedTransKeyRef = useRef(transKey);

  const commitPendingDraftSession = useCallback(() => {
    const snap = draftSessionRef.current;
    if (snap.cueId === null || !snap.jobId) return;
    const currentProjectJobId =
      useAppStore.getState().activeProject?.jobId ?? null;
    if (currentProjectJobId !== snap.jobId) return;

    const patch: { sourceText?: string; translatedText?: string } = {};
    if (snap.origDraft !== snap.sourceText) {
      patch.sourceText = snap.origDraft;
    }
    if (snap.prevDraft !== snap.translatedText) {
      patch.translatedText = snap.prevDraft;
    }
    if (patch.sourceText !== undefined || patch.translatedText !== undefined) {
      snap.sourceText = snap.origDraft;
      snap.translatedText = snap.prevDraft;
      useAppStore.getState().updateCue(snap.cueId, patch);
    }
  }, []);

  // Zero-loss draft auto-commit on cue switch (BUG-M3) + external sync (undo/style/script)
  useLayoutEffect(() => {
    const snap = draftSessionRef.current;
    const cueChanged = snap.cueKey !== cueKey;

    if (cueChanged) {
      commitPendingDraftSession();
      const nextSource = activeCue?.sourceText ?? "";
      const nextTranslated = activeCue?.translatedText ?? "";
      draftSessionRef.current = {
        cueKey,
        jobId: activeProject?.jobId ?? null,
        cueId: activeCue?.id ?? null,
        sourceText: nextSource,
        translatedText: nextTranslated,
        origDraft: nextSource,
        prevDraft: nextTranslated,
      };
      lastSyncedSourceKeyRef.current = sourceKey;
      lastSyncedTransKeyRef.current = transKey;
      setOrigDraft(nextSource);
      setPrevDraft(nextTranslated);
      setOrigSavedFlash(false);
      setPrevSavedFlash(false);
      return;
    }

    if (sourceKey !== lastSyncedSourceKeyRef.current) {
      const nextSource = activeCue?.sourceText ?? "";
      lastSyncedSourceKeyRef.current = sourceKey;
      draftSessionRef.current.sourceText = nextSource;
      draftSessionRef.current.origDraft = nextSource;
      setOrigDraft(nextSource);
    }

    if (transKey !== lastSyncedTransKeyRef.current) {
      const nextTranslated = activeCue?.translatedText ?? "";
      lastSyncedTransKeyRef.current = transKey;
      draftSessionRef.current.translatedText = nextTranslated;
      draftSessionRef.current.prevDraft = nextTranslated;
      setPrevDraft(nextTranslated);
    }
  }, [
    cueKey,
    sourceKey,
    transKey,
    activeProject?.jobId,
    activeCue?.id,
    activeCue?.sourceText,
    activeCue?.translatedText,
    commitPendingDraftSession,
  ]);

  // Commit any unsaved draft on unmount and clear flash timers
  useEffect(() => {
    return () => {
      commitPendingDraftSession();
      if (origFlashTimerRef.current !== null) {
        window.clearTimeout(origFlashTimerRef.current);
      }
      if (prevFlashTimerRef.current !== null) {
        window.clearTimeout(prevFlashTimerRef.current);
      }
    };
  }, [commitPendingDraftSession]);

  // Auto-size centered cinema textareas to content height (clamped to stage height with overflow scroll)
  const resizeTextarea = useCallback(
    (
      textarea: HTMLTextAreaElement | null,
      stage: HTMLDivElement | null,
    ) => {
      if (!textarea) return;
      textarea.style.height = "0px";
      const stageHeight = stage
        ? Math.max(40, stage.clientHeight - 20)
        : 160;
      const targetHeight = Math.min(
        stageHeight,
        Math.max(32, textarea.scrollHeight),
      );
      textarea.style.height = `${targetHeight}px`;
    },
    [],
  );

  useLayoutEffect(() => {
    resizeTextarea(origTextareaRef.current, origStageRef.current);
  }, [origDraft, sourceFontSize, activeCue?.id, resizeTextarea]);

  useLayoutEffect(() => {
    resizeTextarea(prevTextareaRef.current, prevStageRef.current);
  }, [prevDraft, translatedFontSize, activeCue?.id, resizeTextarea]);

  useEffect(() => {
    const stages = [origStageRef.current, prevStageRef.current].filter(
      (el): el is HTMLDivElement => el !== null,
    );
    if (stages.length === 0) return;

    const handleResize = () => {
      resizeTextarea(origTextareaRef.current, origStageRef.current);
      resizeTextarea(prevTextareaRef.current, prevStageRef.current);
    };

    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(handleResize);
      for (const stage of stages) {
        ro.observe(stage);
      }
      return () => ro.disconnect();
    }
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [resizeTextarea]);

  // Ctrl/Cmd + MouseWheel font-size zoom per editor card (Workstream 4)
  useEffect(() => {
    const origEl = origCardRef.current;
    const prevEl = prevCardRef.current;

    const handleOrigWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.deltaY === 0) return;
      e.preventDefault();
      const delta = e.deltaY < 0 ? 1 : -1;
      void setEditorFontSize("source", sourceFontSizeRef.current + delta);
    };

    const handlePrevWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.deltaY === 0) return;
      e.preventDefault();
      const delta = e.deltaY < 0 ? 1 : -1;
      void setEditorFontSize(
        "translated",
        translatedFontSizeRef.current + delta,
      );
    };

    origEl?.addEventListener("wheel", handleOrigWheel, { passive: false });
    prevEl?.addEventListener("wheel", handlePrevWheel, { passive: false });
    return () => {
      origEl?.removeEventListener("wheel", handleOrigWheel);
      prevEl?.removeEventListener("wheel", handlePrevWheel);
    };
  }, [setEditorFontSize]);

  const origDirty = Boolean(activeCue && origDraft !== activeCue.sourceText);
  const prevDirty = Boolean(
    activeCue && prevDraft !== activeCue.translatedText,
  );

  const saveOriginal = () => {
    if (!activeCue || !origDirty) return;
    draftSessionRef.current.sourceText = origDraft;
    draftSessionRef.current.origDraft = origDraft;
    updateCue(activeCue.id, { sourceText: origDraft });
    setOrigSavedFlash(true);
    if (origFlashTimerRef.current !== null) {
      window.clearTimeout(origFlashTimerRef.current);
    }
    origFlashTimerRef.current = window.setTimeout(() => {
      setOrigSavedFlash(false);
    }, 2000);
  };

  const revertOriginal = () => {
    if (!activeCue) return;
    draftSessionRef.current.origDraft = activeCue.sourceText;
    setOrigDraft(activeCue.sourceText);
    setOrigSavedFlash(false);
  };

  const savePrevod = () => {
    if (!activeCue || !prevDirty) return;
    draftSessionRef.current.translatedText = prevDraft;
    draftSessionRef.current.prevDraft = prevDraft;
    updateCue(activeCue.id, { translatedText: prevDraft });
    setPrevSavedFlash(true);
    if (prevFlashTimerRef.current !== null) {
      window.clearTimeout(prevFlashTimerRef.current);
    }
    prevFlashTimerRef.current = window.setTimeout(() => {
      setPrevSavedFlash(false);
    }, 2000);
  };

  const revertPrevod = () => {
    if (!activeCue) return;
    draftSessionRef.current.prevDraft = activeCue.translatedText;
    setPrevDraft(activeCue.translatedText);
    setPrevSavedFlash(false);
  };

  const handleStyleSwitch = (nextStyle: TranslationStyle) => {
    if (prevDirty) {
      savePrevod();
    }
    switchTranslationStyle(nextStyle);
  };

  const calcMetrics = (text: string) => {
    const chars = Array.from(text.replace(/\r?\n/g, "")).length;
    const durSec = activeCue
      ? Math.max(0.2, (activeCue.endMs - activeCue.startMs) / 1000)
      : 1;
    const cps = activeCue ? chars / durSec : 0;
    const maxCps = jobOptions.segmentRules.maxCps;
    const maxTotalChars =
      jobOptions.segmentRules.maxCharsPerLine *
      jobOptions.segmentRules.maxLines;

    let color = "#6f7a8d";
    if (activeCue) {
      if (cps > maxCps + 4) {
        color = "#fb7185";
      } else if (cps > maxCps || chars > maxTotalChars) {
        color = "#fbbf24";
      }
    }

    return {
      label: `${chars} ${lang === "en" ? "ch." : "zn."} • ${cps.toFixed(1)} CPS`,
      color,
    };
  };

  const origMetrics = calcMetrics(origDraft);
  const prevMetrics = calcMetrics(prevDraft);

  const clipMeta = activeCue
    ? `#${activeCue.id} • ${formatTimestampSrt(activeCue.startMs)} → ${formatTimestampSrt(activeCue.endMs)}`
    : t.editor.noCueSelected;

  const srcLangLabel = (
    activeProject?.detectedLanguage ||
    activeProject?.options.sourceLanguage ||
    jobOptions.sourceLanguage ||
    "AUTO"
  ).toUpperCase();

  const dstLangRaw =
    activeProject?.options.targetLanguage || jobOptions.targetLanguage || "SR";
  const dstLangLabel =
    dstLangRaw === "none" ? "—" : dstLangRaw.toUpperCase();

  const hasCues = Boolean(activeProject && activeProject.cues.length > 0);
  const hasAnyWarnings = Boolean(
    activeProject?.cues.some(
      (c) => c.warnings.length > 0 || c.cps > jobOptions.segmentRules.maxCps,
    ),
  );
  const showUncachedStyleBadge =
    hasCues &&
    !styleCache[activeStyle] &&
    (styleCache.natural || styleCache.formal);

  return (
    <section id="editor-band" aria-label={t.table.title}>
      {/* Left Card: Original transcript (Indigo glow #818cf8) */}
      <article
        ref={origCardRef}
        id="card-original"
        className="editor-card original"
      >
        <header className="editor-card-head">
          <div className="ec-head-left">
            <span className="ec-icon" aria-hidden="true">
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
            <span className="ec-title">{t.editor.originalTitle}</span>
            <span className="ec-lang-pill">{srcLangLabel}</span>
            <span id="orig-clip-meta" className="ec-clip-meta">
              {clipMeta}
            </span>
          </div>

          <div className="ec-head-right">
            <div
              className="ec-font-stepper"
              role="group"
              aria-label={t.editor.fontSizeHint}
              title={t.editor.fontSizeHint}
            >
              <button
                id="btn-orig-font-dec"
                type="button"
                className="ec-font-btn"
                disabled={sourceFontSize <= EDITOR_SOURCE_FONT_MIN}
                aria-label={t.editor.fontSizeDecrease}
                onClick={() =>
                  void setEditorFontSize("source", sourceFontSize - 1)
                }
              >
                A−
              </button>
              <span className="ec-font-val">{sourceFontSize}</span>
              <button
                id="btn-orig-font-inc"
                type="button"
                className="ec-font-btn"
                disabled={sourceFontSize >= EDITOR_SOURCE_FONT_MAX}
                aria-label={t.editor.fontSizeIncrease}
                onClick={() =>
                  void setEditorFontSize("source", sourceFontSize + 1)
                }
              >
                A+
              </button>
            </div>

            <span
              id="orig-save-status"
              className={`ec-save-status${origDirty ? " dirty" : ""}${
                !origDirty && origSavedFlash ? " saved-flash orig" : ""
              }`}
            >
              {origDirty
                ? t.editor.unsaved
                : origSavedFlash
                  ? t.editor.savedCheck
                  : t.editor.saved}
            </span>
          </div>
        </header>

        <div
          ref={origStageRef}
          className={`editor-card-body cinema-stage${activeCue ? " interactive" : ""}`}
          onClick={(e) => {
            if (e.target !== origTextareaRef.current && activeCue) {
              origTextareaRef.current?.focus();
            }
          }}
        >
          <textarea
            ref={origTextareaRef}
            id="textarea-original"
            className="editor-textarea cinema-textarea"
            rows={1}
            spellCheck={false}
            disabled={!activeCue}
            style={{ fontSize: `${sourceFontSize}px` }}
            aria-label={t.editor.originalTitle}
            placeholder={activeCue || hasCues ? "" : t.editor.selectOrAddCue}
            value={origDraft}
            onChange={(e) => {
              const nextVal = e.target.value;
              draftSessionRef.current.origDraft = nextVal;
              setOrigDraft(nextVal);
              if (origSavedFlash) setOrigSavedFlash(false);
            }}
            onBlur={() => {
              if (origDirty) {
                saveOriginal();
              }
            }}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
                e.preventDefault();
                saveOriginal();
                return;
              }
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                saveOriginal();
                e.currentTarget.blur();
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                e.currentTarget.blur();
              }
            }}
          />
        </div>

        <footer className="editor-card-foot">
          <span
            id="orig-metrics"
            className="ec-metrics"
            style={{ color: origMetrics.color }}
          >
            {origMetrics.label}
          </span>
          <div className="ec-actions">
            <button
              id="btn-revert-original"
              className="ec-btn-ghost"
              type="button"
              disabled={!origDirty}
              onMouseDown={(e) => e.preventDefault()}
              onClick={revertOriginal}
            >
              {t.editor.revert}
            </button>
            <button
              id="btn-save-original"
              className="ec-btn-save orig"
              type="button"
              disabled={!origDirty}
              onMouseDown={(e) => e.preventDefault()}
              onClick={saveOriginal}
            >
              {t.editor.saveEdit}
            </button>
          </div>
        </footer>
      </article>

      {/* Right Card: Prevod (Emerald glow #10b981) */}
      <article
        ref={prevCardRef}
        id="card-prevod"
        className="editor-card prevod"
      >
        <header className="editor-card-head">
          <div className="ec-head-left">
            <span className="ec-icon" aria-hidden="true">
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
            <span className="ec-title">{t.editor.translationTitle}</span>
            <span className="ec-lang-pill">{dstLangLabel}</span>
            <span id="prev-clip-meta" className="ec-clip-meta">
              {clipMeta}
            </span>
          </div>

          <div className="ec-head-right">
            <div
              className="ec-style-seg"
              role="group"
              aria-label={t.workspace.translationStyle}
            >
              <button
                id="btn-style-natural"
                type="button"
                className={`ec-style-btn${activeStyle === "natural" ? " active" : ""}`}
                disabled={isRunningBatch}
                aria-pressed={activeStyle === "natural"}
                title={t.workspace.translationStyleNaturalHint}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => handleStyleSwitch("natural")}
              >
                {t.editor.styleNaturalShort}
                {styleCache.natural ? " ✓" : ""}
              </button>
              <button
                id="btn-style-formal"
                type="button"
                className={`ec-style-btn${activeStyle === "formal" ? " active" : ""}`}
                disabled={isRunningBatch}
                aria-pressed={activeStyle === "formal"}
                title={t.workspace.translationStyleFormalHint}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => handleStyleSwitch("formal")}
              >
                {t.editor.styleFormalShort}
                {styleCache.formal ? " ✓" : ""}
              </button>
            </div>

            <div
              className="ec-font-stepper"
              role="group"
              aria-label={t.editor.fontSizeHint}
              title={t.editor.fontSizeHint}
            >
              <button
                id="btn-prev-font-dec"
                type="button"
                className="ec-font-btn"
                disabled={translatedFontSize <= EDITOR_TRANSLATED_FONT_MIN}
                aria-label={t.editor.fontSizeDecrease}
                onClick={() =>
                  void setEditorFontSize("translated", translatedFontSize - 1)
                }
              >
                A−
              </button>
              <span className="ec-font-val">{translatedFontSize}</span>
              <button
                id="btn-prev-font-inc"
                type="button"
                className="ec-font-btn"
                disabled={translatedFontSize >= EDITOR_TRANSLATED_FONT_MAX}
                aria-label={t.editor.fontSizeIncrease}
                onClick={() =>
                  void setEditorFontSize("translated", translatedFontSize + 1)
                }
              >
                A+
              </button>
            </div>

            <span
              id="prev-save-status"
              className={`ec-save-status${prevDirty ? " dirty" : ""}${
                !prevDirty && prevSavedFlash ? " saved-flash prev" : ""
              }`}
            >
              {prevDirty
                ? t.editor.unsaved
                : prevSavedFlash
                  ? t.editor.savedCheck
                  : t.editor.saved}
            </span>
          </div>
        </header>

        <div
          ref={prevStageRef}
          className={`editor-card-body cinema-stage${activeCue ? " interactive" : ""}`}
          onClick={(e) => {
            if (e.target !== prevTextareaRef.current && activeCue) {
              prevTextareaRef.current?.focus();
            }
          }}
        >
          {showUncachedStyleBadge && (
            <button
              id="btn-translate-uncached-style"
              type="button"
              className="ec-style-uncached-badge"
              disabled={!hasCues || isRunningBatch}
              onMouseDown={(e) => e.preventDefault()}
              onClick={(e) => {
                e.stopPropagation();
                void translateActiveProject();
              }}
            >
              <span>
                {activeStyle === "formal"
                  ? t.editor.styleFormalNotGenerated
                  : t.editor.styleNaturalNotGenerated}
              </span>
            </button>
          )}

          <textarea
            ref={prevTextareaRef}
            id="textarea-prevod"
            className="editor-textarea cinema-textarea"
            rows={1}
            spellCheck={false}
            disabled={!activeCue}
            style={{ fontSize: `${translatedFontSize}px` }}
            aria-label={t.editor.translationTitle}
            placeholder={activeCue || hasCues ? "" : t.editor.selectOrAddCue}
            value={prevDraft}
            onChange={(e) => {
              const nextVal = e.target.value;
              draftSessionRef.current.prevDraft = nextVal;
              setPrevDraft(nextVal);
              if (prevSavedFlash) setPrevSavedFlash(false);
            }}
            onBlur={() => {
              if (prevDirty) {
                savePrevod();
              }
            }}
            onKeyDown={(e) => {
              if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
                e.preventDefault();
                savePrevod();
                return;
              }
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                savePrevod();
                e.currentTarget.blur();
                return;
              }
              if (e.key === "Escape") {
                e.preventDefault();
                e.currentTarget.blur();
              }
            }}
          />
        </div>

        <footer className="editor-card-foot">
          <span
            id="prev-metrics"
            className="ec-metrics"
            style={{ color: prevMetrics.color }}
          >
            {prevMetrics.label}
          </span>

          <div className="ec-actions">
            <div className="ec-ai-group">
              <button
                type="button"
                className="ec-ai-btn"
                disabled={!hasCues || isRunningBatch}
                onClick={translateActiveProject}
                title={
                  lang === "en"
                    ? "Translate active project"
                    : "Prevedi aktivni projekat"
                }
              >
                {lang === "en" ? "Translate" : "Prevedi"}
              </button>
              <button
                type="button"
                className="ec-ai-btn polish"
                disabled={!hasCues || isRunningBatch}
                onClick={polishActiveProject}
                title={
                  lang === "en"
                    ? "Polish Serbian subtitles"
                    : "Poliraj srpski prevod"
                }
              >
                {lang === "en" ? "Polish" : "Poliraj"}
              </button>
              <button
                type="button"
                className="ec-ai-btn cps"
                disabled={!hasCues || isRunningBatch}
                onClick={condenseActiveProjectCps}
                title={t.table.condenseCpsTitle}
                style={
                  hasAnyWarnings
                    ? {
                        borderColor: "rgba(245, 158, 11, 0.58)",
                        boxShadow: "0 0 8px rgba(245, 158, 11, 0.22)",
                      }
                    : undefined
                }
              >
                {lang === "en" ? "Fit CPS" : "Sažmi CPS"}
              </button>
            </div>

            <button
              id="btn-revert-prevod"
              className="ec-btn-ghost"
              type="button"
              disabled={!prevDirty}
              onMouseDown={(e) => e.preventDefault()}
              onClick={revertPrevod}
            >
              {t.editor.revert}
            </button>
            <button
              id="btn-save-prevod"
              className="ec-btn-save prev"
              type="button"
              disabled={!prevDirty}
              onMouseDown={(e) => e.preventDefault()}
              onClick={savePrevod}
            >
              {t.editor.saveEdit}
            </button>
          </div>
        </footer>
      </article>
    </section>
  );
}
