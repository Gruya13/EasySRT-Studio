import { useEffect, useMemo, useRef, useState } from "react";
import { normalizeUiLanguage } from "../i18n";
import { formatTimestampSrt } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";

export function DualEditorBand() {
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const isRunningBatch = useAppStore((s) => s.isRunningBatch);

  const updateCue = useAppStore((s) => s.updateCue);
  const translateActiveProject = useAppStore((s) => s.translateActiveProject);
  const polishActiveProject = useAppStore((s) => s.polishActiveProject);
  const condenseActiveProjectCps = useAppStore(
    (s) => s.condenseActiveProjectCps,
  );

  const lang = normalizeUiLanguage(config.uiLanguage);

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

  const cueKey = `${activeProject?.jobId ?? ""}:${activeCue?.id ?? ""}`;
  const sourceKey = `${cueKey}:${activeCue?.sourceText ?? ""}`;
  const transKey = `${cueKey}:${activeCue?.translatedText ?? ""}`;

  const [lastCueKey, setLastCueKey] = useState(cueKey);
  const [lastSourceKey, setLastSourceKey] = useState(sourceKey);
  const [lastTransKey, setLastTransKey] = useState(transKey);

  const [origDraft, setOrigDraft] = useState(activeCue?.sourceText ?? "");
  const [prevDraft, setPrevDraft] = useState(activeCue?.translatedText ?? "");
  const [origSavedFlash, setOrigSavedFlash] = useState(false);
  const [prevSavedFlash, setPrevSavedFlash] = useState(false);

  if (cueKey !== lastCueKey) {
    setLastCueKey(cueKey);
    setOrigSavedFlash(false);
    setPrevSavedFlash(false);
  }
  if (sourceKey !== lastSourceKey) {
    setLastSourceKey(sourceKey);
    setOrigDraft(activeCue?.sourceText ?? "");
  }
  if (transKey !== lastTransKey) {
    setLastTransKey(transKey);
    setPrevDraft(activeCue?.translatedText ?? "");
  }

  const origFlashTimerRef = useRef<number | null>(null);
  const prevFlashTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (origFlashTimerRef.current !== null) {
        window.clearTimeout(origFlashTimerRef.current);
      }
      if (prevFlashTimerRef.current !== null) {
        window.clearTimeout(prevFlashTimerRef.current);
      }
    };
  }, []);

  const origDirty = Boolean(activeCue && origDraft !== activeCue.sourceText);
  const prevDirty = Boolean(activeCue && prevDraft !== activeCue.translatedText);

  const saveOriginal = () => {
    if (!activeCue || !origDirty) return;
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
    setOrigDraft(activeCue.sourceText);
    setOrigSavedFlash(false);
  };

  const savePrevod = () => {
    if (!activeCue || !prevDirty) return;
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
    setPrevDraft(activeCue.translatedText);
    setPrevSavedFlash(false);
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
    : lang === "en"
      ? "No cue selected"
      : "Nema aktivnog titla";

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
    activeProject?.cues.some((c) => c.warnings.length > 0 || c.cps > jobOptions.segmentRules.maxCps),
  );

  return (
    <section
      id="editor-band"
      aria-label={lang === "en" ? "Subtitle Editor" : "Editor titlova"}
    >
      {/* Left Card: Original transcript (Indigo glow #818cf8) */}
      <article id="card-original" className="editor-card original">
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
            <span className="ec-title">Original transcript</span>
            <span className="ec-lang-pill">{srcLangLabel}</span>
            <span id="orig-clip-meta" className="ec-clip-meta">
              {clipMeta}
            </span>
          </div>

          <span
            id="orig-save-status"
            className={`ec-save-status${origDirty ? " dirty" : ""}${
              !origDirty && origSavedFlash ? " saved-flash orig" : ""
            }`}
          >
            {origDirty
              ? lang === "en"
                ? "Unsaved"
                : "Nesačuvano"
              : origSavedFlash
                ? lang === "en"
                  ? "✓ Saved"
                  : "✓ Sačuvano"
                : lang === "en"
                  ? "Saved"
                  : "Sačuvano"}
          </span>
        </header>

        <div className="editor-card-body">
          <textarea
            id="textarea-original"
            className="editor-textarea"
            spellCheck={false}
            disabled={!activeCue}
            aria-label={
              lang === "en" ? "Source transcript" : "Izvorni transkript"
            }
            placeholder={
              activeCue || hasCues
                ? ""
                : lang === "en"
                  ? "Select or add a subtitle cue..."
                  : "Izaberite ili dodajte titl na timeline-u..."
            }
            value={origDraft}
            onChange={(e) => {
              setOrigDraft(e.target.value);
              if (origSavedFlash) setOrigSavedFlash(false);
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
              onClick={revertOriginal}
            >
              {lang === "en" ? "Revert" : "Poništi"}
            </button>
            <button
              id="btn-save-original"
              className="ec-btn-save orig"
              type="button"
              disabled={!origDirty}
              onClick={saveOriginal}
            >
              {lang === "en" ? "Save edit" : "Sačuvaj izmenu"}
            </button>
          </div>
        </footer>
      </article>

      {/* Right Card: Prevod (Emerald glow #10b981) */}
      <article id="card-prevod" className="editor-card prevod">
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
            <span className="ec-title">
              {lang === "en" ? "Translation" : "Prevod"}
            </span>
            <span className="ec-lang-pill">{dstLangLabel}</span>
            <span id="prev-clip-meta" className="ec-clip-meta">
              {clipMeta}
            </span>
          </div>

          <span
            id="prev-save-status"
            className={`ec-save-status${prevDirty ? " dirty" : ""}${
              !prevDirty && prevSavedFlash ? " saved-flash prev" : ""
            }`}
          >
            {prevDirty
              ? lang === "en"
                ? "Unsaved"
                : "Nesačuvano"
              : prevSavedFlash
                ? lang === "en"
                  ? "✓ Saved"
                  : "✓ Sačuvano"
                : lang === "en"
                  ? "Saved"
                  : "Sačuvano"}
          </span>
        </header>

        <div className="editor-card-body">
          <textarea
            id="textarea-prevod"
            className="editor-textarea"
            spellCheck={false}
            disabled={!activeCue}
            aria-label={
              lang === "en" ? "Subtitle translation" : "Prevod titla"
            }
            placeholder={
              activeCue || hasCues
                ? ""
                : lang === "en"
                  ? "Select or add a subtitle cue..."
                  : "Izaberite ili dodajte titl na timeline-u..."
            }
            value={prevDraft}
            onChange={(e) => {
              setPrevDraft(e.target.value);
              if (prevSavedFlash) setPrevSavedFlash(false);
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
                title={
                  lang === "en"
                    ? "Automatically condense high-CPS cues"
                    : "Automatski sažmi preduge titlove (CPS)"
                }
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
              onClick={revertPrevod}
            >
              {lang === "en" ? "Revert" : "Poništi"}
            </button>
            <button
              id="btn-save-prevod"
              className="ec-btn-save prev"
              type="button"
              disabled={!prevDirty}
              onClick={savePrevod}
            >
              {lang === "en" ? "Save edit" : "Sačuvaj izmenu"}
            </button>
          </div>
        </footer>
      </article>
    </section>
  );
}
