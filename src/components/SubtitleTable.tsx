import {
  AlertTriangle,
  Keyboard,
  Merge,
  Play,
  Plus,
  Redo2,
  Replace,
  Scissors,
  Search,
  Sparkles,
  Subtitles,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import {
  formatDurationShort,
  formatTimestampSrt,
  parseTimestampSrt,
} from "../lib/subtitleUtils";
import {
  useAppStore,
  type ReplaceTargetColumn,
} from "../stores/appStore";
import type { SubtitleCue } from "../types/models";

interface TimestampCellProps {
  ms: number;
  onChange: (nextMs: number) => void;
}

function TimestampCell({ ms, onChange }: TimestampCellProps) {
  const [draft, setDraft] = useState(() => formatTimestampSrt(ms));
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) {
      setDraft(formatTimestampSrt(ms));
    }
  }, [ms, editing]);

  const commit = () => {
    setEditing(false);
    const parsed = parseTimestampSrt(draft);
    if (parsed !== null && parsed !== ms) {
      onChange(parsed);
    } else {
      setDraft(formatTimestampSrt(ms));
    }
  };

  return (
    <input
      type="text"
      value={draft}
      onFocus={() => setEditing(true)}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.currentTarget.blur();
        } else if (e.key === "Escape") {
          setDraft(formatTimestampSrt(ms));
          setEditing(false);
        }
      }}
      className="w-28 rounded-lg border border-slate-800 bg-slate-950/90 px-2 py-1 font-mono text-xs text-slate-200 transition focus:border-cyan-500 focus:outline-none"
    />
  );
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

export function SubtitleTable() {
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const jobOptions = useAppStore((s) => s.jobOptions);
  const isRunningBatch = useAppStore((s) => s.isRunningBatch);
  const activeCueId = useAppStore((s) => s.activeCueId);
  const canUndo = useAppStore((s) => s.canUndo);
  const canRedo = useAppStore((s) => s.canRedo);

  const updateCue = useAppStore((s) => s.updateCue);
  const splitCueAtMidpoint = useAppStore((s) => s.splitCueAtMidpoint);
  const mergeCueWithNext = useAppStore((s) => s.mergeCueWithNext);
  const deleteCueById = useAppStore((s) => s.deleteCueById);
  const addCueAtEnd = useAppStore((s) => s.addCueAtEnd);
  const replaceInCues = useAppStore((s) => s.replaceInCues);
  const undoCueEdit = useAppStore((s) => s.undoCueEdit);
  const redoCueEdit = useAppStore((s) => s.redoCueEdit);
  const condenseActiveProjectCps = useAppStore(
    (s) => s.condenseActiveProjectCps,
  );
  const playCueSegment = useAppStore((s) => s.playCueSegment);
  const setActiveCueId = useAppStore((s) => s.setActiveCueId);

  const [filterMode, setFilterMode] = useState<"all" | "warnings">("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [isReplaceOpen, setIsReplaceOpen] = useState(false);
  const [replaceSearchText, setReplaceSearchText] = useState("");
  const [replaceWithText, setReplaceWithText] = useState("");
  const [replaceTargetColumn, setReplaceTargetColumn] =
    useState<ReplaceTargetColumn>("both");
  const [replaceCaseSensitive, setReplaceCaseSensitive] = useState(false);

  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const replaceSearchInputRef = useRef<HTMLInputElement | null>(null);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const maxCps = jobOptions.segmentRules.maxCps;
  const maxCharsPerLine = jobOptions.segmentRules.maxCharsPerLine;

  const isCueWarning = (cue: SubtitleCue) =>
    cue.cps > maxCps ||
    cue.warnings.length > 0 ||
    (cue.translatedText || cue.sourceText)
      .split("\n")
      .some((line) => [...line].length > maxCharsPerLine);

  const filteredCues = useMemo(() => {
    if (!activeProject) {
      return [];
    }
    return activeProject.cues.filter((cue) => {
      if (filterMode === "warnings" && !isCueWarning(cue)) {
        return false;
      }
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        return (
          cue.sourceText.toLowerCase().includes(q) ||
          cue.translatedText.toLowerCase().includes(q) ||
          String(cue.id).includes(q)
        );
      }
      return true;
    });
  }, [activeProject, filterMode, searchQuery, maxCps, maxCharsPerLine]);

  const replaceMatchCount = useMemo(() => {
    if (!activeProject || !replaceSearchText) {
      return 0;
    }
    const escaped = replaceSearchText.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const regex = new RegExp(escaped, replaceCaseSensitive ? "g" : "gi");
    const checkSource =
      replaceTargetColumn === "both" || replaceTargetColumn === "source";
    const checkTranslated =
      replaceTargetColumn === "both" || replaceTargetColumn === "translated";

    let count = 0;
    for (const cue of activeProject.cues) {
      if (checkSource && cue.sourceText) {
        const m = cue.sourceText.match(regex);
        if (m) {
          count += m.length;
        }
      }
      if (checkTranslated && cue.translatedText) {
        const m = cue.translatedText.match(regex);
        if (m) {
          count += m.length;
        }
      }
    }
    return count;
  }, [
    activeProject,
    replaceSearchText,
    replaceTargetColumn,
    replaceCaseSensitive,
  ]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!activeProject || activeProject.cues.length === 0) {
        return;
      }

      const mod = e.ctrlKey || e.metaKey;
      const keyLower = e.key.toLowerCase();
      const inEditable = isEditableElement(e.target);

      // Ctrl+F: Focus Search input
      if (mod && !e.shiftKey && !e.altKey && keyLower === "f") {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }

      // Ctrl+H: Open & focus Search & Replace bar
      if (mod && !e.shiftKey && !e.altKey && keyLower === "h") {
        e.preventDefault();
        setIsReplaceOpen(true);
        if (!replaceSearchText && searchQuery) {
          setReplaceSearchText(searchQuery);
        }
        setTimeout(() => {
          replaceSearchInputRef.current?.focus();
          replaceSearchInputRef.current?.select();
        }, 0);
        return;
      }

      // Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z: Undo / Redo when not actively typing inside an input/textarea
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

      // Alt+ArrowDown / Alt+ArrowUp: Move activeCueId to next/previous visible cue and play/highlight it
      if (
        e.altKey &&
        !mod &&
        (e.key === "ArrowDown" || e.key === "ArrowUp")
      ) {
        if (filteredCues.length === 0) {
          return;
        }
        e.preventDefault();
        const currentIdx = filteredCues.findIndex(
          (c) => c.id === activeCueId,
        );
        const nextIdx =
          e.key === "ArrowDown"
            ? currentIdx === -1
              ? 0
              : Math.min(filteredCues.length - 1, currentIdx + 1)
            : currentIdx === -1
              ? filteredCues.length - 1
              : Math.max(0, currentIdx - 1);
        const targetCue = filteredCues[nextIdx];
        if (targetCue) {
          playCueSegment(targetCue);
          document
            .getElementById(`cue-row-${targetCue.id}`)
            ?.scrollIntoView({ block: "nearest", behavior: "smooth" });
        }
        return;
      }

      // Space: Play active cue segment when not focused inside an input/textarea/button
      if (
        e.code === "Space" &&
        !mod &&
        !e.altKey &&
        !e.shiftKey &&
        !inEditable &&
        !(e.target instanceof HTMLButtonElement)
      ) {
        if (activeCueId !== null) {
          const targetCue = activeProject.cues.find(
            (c) => c.id === activeCueId,
          );
          if (targetCue) {
            e.preventDefault();
            playCueSegment(targetCue);
          }
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    activeProject,
    activeCueId,
    filteredCues,
    replaceSearchText,
    searchQuery,
    undoCueEdit,
    redoCueEdit,
    playCueSegment,
  ]);

  if (!activeProject) {
    return (
      <div className="flex min-h-[280px] flex-col items-center justify-center gap-3 rounded-2xl border border-slate-800/80 bg-slate-900/40 p-8 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-2xl border border-slate-800 bg-slate-900 text-slate-500">
          <Subtitles className="h-6 w-6" />
        </div>
        <p className="max-w-md text-xs leading-relaxed text-slate-400">
          {t.table.noProject}
        </p>
      </div>
    );
  }

  const warningCount = activeProject.cues.filter(isCueWarning).length;

  const getCpsBadgeStyle = (cue: SubtitleCue) => {
    const hasLineTooLong =
      cue.warnings.includes("line_too_long") ||
      (cue.translatedText || cue.sourceText)
        .split("\n")
        .some((line) => [...line].length > maxCharsPerLine);

    if (cue.cps > maxCps + 4 || hasLineTooLong) {
      return "border-rose-500/40 bg-rose-500/15 text-rose-300";
    }
    if (cue.cps > maxCps) {
      return "border-amber-500/40 bg-amber-500/15 text-amber-300";
    }
    return "border-emerald-500/35 bg-emerald-500/15 text-emerald-300";
  };

  const formatWarningLabels = (cue: SubtitleCue) => {
    const labels: string[] = [];
    const warningsDict = t.table.warnings as Record<string, string>;
    for (const w of cue.warnings) {
      if (warningsDict[w]) {
        labels.push(warningsDict[w]);
      } else {
        labels.push(w);
      }
    }
    return labels;
  };

  const handleReplaceNext = () => {
    if (!replaceSearchText) {
      return;
    }
    replaceInCues({
      searchText: replaceSearchText,
      replaceText: replaceWithText,
      targetColumn: replaceTargetColumn,
      caseSensitive: replaceCaseSensitive,
      mode: "next",
      startFromCueId: activeCueId,
    });
  };

  const handleReplaceAll = () => {
    if (!replaceSearchText) {
      return;
    }
    replaceInCues({
      searchText: replaceSearchText,
      replaceText: replaceWithText,
      targetColumn: replaceTargetColumn,
      caseSensitive: replaceCaseSensitive,
      mode: "all",
    });
  };

  return (
    <section className="flex flex-col overflow-hidden rounded-2xl border border-slate-800/90 bg-slate-900/70 shadow-lg">
      {/* Toolbar: Project Stats, Undo/Redo, Filter Toggle, Search & Replace Toggle */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-800 bg-slate-950/60 px-4 py-3">
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-sm font-bold text-slate-100">{t.table.title}</h2>
          <span className="rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-0.5 font-mono text-xs text-cyan-300">
            {activeProject.audioFileName}
          </span>
          <div className="flex items-center gap-2 font-mono text-xs text-slate-400">
            <span>
              {t.table.statsTotal}:{" "}
              <strong className="text-slate-200">
                {activeProject.cues.length}
              </strong>
            </span>
            <span>•</span>
            <span>
              {t.table.statsWarnings}:{" "}
              <strong
                className={
                  warningCount > 0 ? "text-amber-300" : "text-emerald-400"
                }
              >
                {warningCount}
              </strong>
            </span>
            <span>•</span>
            <span>
              {t.table.statsDuration}:{" "}
              <strong className="text-slate-200">
                {formatDurationShort(activeProject.durationMs)}
              </strong>
            </span>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Undo / Redo Buttons */}
          <div className="flex items-center rounded-xl border border-slate-800 bg-slate-950 p-0.5">
            <button
              type="button"
              disabled={!canUndo}
              onClick={undoCueEdit}
              title={t.table.undo}
              className="rounded-lg p-1.5 text-slate-300 transition hover:bg-slate-800 hover:text-cyan-200 disabled:cursor-not-allowed disabled:opacity-30"
            >
              <Undo2 className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              disabled={!canRedo}
              onClick={redoCueEdit}
              title={t.table.redo}
              className="rounded-lg p-1.5 text-slate-300 transition hover:bg-slate-800 hover:text-cyan-200 disabled:cursor-not-allowed disabled:opacity-30"
            >
              <Redo2 className="h-3.5 w-3.5" />
            </button>
          </div>

          {/* Filter: All vs Warnings Only */}
          <div className="flex items-center rounded-xl border border-slate-800 bg-slate-950 p-0.5 text-xs font-semibold">
            <button
              type="button"
              onClick={() => setFilterMode("all")}
              className={`rounded-lg px-2.5 py-1 transition ${
                filterMode === "all"
                  ? "bg-cyan-500/20 text-cyan-200"
                  : "text-slate-400 hover:text-slate-200"
              }`}
            >
              {t.table.filterAll} ({activeProject.cues.length})
            </button>
            <button
              type="button"
              onClick={() => setFilterMode("warnings")}
              className={`flex items-center gap-1 rounded-lg px-2.5 py-1 transition ${
                filterMode === "warnings"
                  ? "bg-amber-500/20 text-amber-200"
                  : "text-slate-400 hover:text-slate-200"
              }`}
            >
              <AlertTriangle className="h-3 w-3 text-amber-400" />
              <span>
                {t.table.filterWarnings} ({warningCount})
              </span>
            </button>
          </div>

          {warningCount > 0 && (
            <button
              type="button"
              data-testid="btn-condense-cps"
              disabled={isRunningBatch}
              onClick={condenseActiveProjectCps}
              title={t.table.condenseCpsTitle}
              className="inline-flex items-center gap-1.5 rounded-xl border border-amber-500/40 bg-amber-500/15 px-2.5 py-1.5 text-xs font-semibold text-amber-200 transition hover:bg-amber-500/25 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <Sparkles className="h-3.5 w-3.5 text-amber-300" />
              <span>{t.table.condenseCpsBtn}</span>
            </button>
          )}

          {/* Search Box (Ctrl+F) + Search & Replace Toggle (Ctrl+H) */}
          <div className="flex items-center gap-1.5">
            <div className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-slate-500" />
              <input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder={t.table.searchPlaceholder}
                className="w-44 rounded-xl border border-slate-800 bg-slate-950 py-1.5 pl-8 pr-3 text-xs text-slate-200 placeholder:text-slate-500 focus:w-56 focus:border-cyan-500 focus:outline-none"
              />
            </div>

            <button
              type="button"
              onClick={() => {
                const nextOpen = !isReplaceOpen;
                setIsReplaceOpen(nextOpen);
                if (nextOpen) {
                  if (!replaceSearchText && searchQuery) {
                    setReplaceSearchText(searchQuery);
                  }
                  setTimeout(() => {
                    replaceSearchInputRef.current?.focus();
                  }, 0);
                }
              }}
              title={t.table.toggleReplace}
              className={`inline-flex items-center gap-1 rounded-xl border px-2.5 py-1.5 text-xs font-semibold transition ${
                isReplaceOpen
                  ? "border-cyan-500/50 bg-cyan-500/20 text-cyan-200"
                  : "border-slate-800 bg-slate-950 text-slate-300 hover:border-slate-700 hover:text-slate-100"
              }`}
            >
              <Replace className="h-3.5 w-3.5" />
            </button>
          </div>

          <button
            type="button"
            onClick={addCueAtEnd}
            className="inline-flex items-center gap-1 rounded-xl border border-emerald-500/30 bg-emerald-500/15 px-2.5 py-1.5 text-xs font-semibold text-emerald-300 transition hover:bg-emerald-500/25"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t.table.addCue}</span>
          </button>
        </div>
      </div>

      {/* Collapsible Search & Replace Bar (Ctrl+H) */}
      {isReplaceOpen && (
        <div className="flex flex-wrap items-center justify-between gap-2.5 border-b border-slate-800 bg-slate-950/90 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            {/* Find Text */}
            <div className="relative">
              <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-slate-500" />
              <input
                ref={replaceSearchInputRef}
                type="text"
                value={replaceSearchText}
                onChange={(e) => setReplaceSearchText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    handleReplaceNext();
                  } else if (e.key === "Escape") {
                    setIsReplaceOpen(false);
                  }
                }}
                placeholder={t.table.replaceFindPlaceholder}
                className="w-44 rounded-xl border border-slate-800 bg-slate-900 py-1.5 pl-8 pr-2.5 text-xs text-slate-100 placeholder:text-slate-500 focus:border-cyan-500 focus:outline-none"
              />
            </div>

            {/* Replace With Text */}
            <div className="relative">
              <Replace className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-slate-500" />
              <input
                type="text"
                value={replaceWithText}
                onChange={(e) => setReplaceWithText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    handleReplaceNext();
                  } else if (e.key === "Escape") {
                    setIsReplaceOpen(false);
                  }
                }}
                placeholder={t.table.replaceWithPlaceholder}
                className="w-44 rounded-xl border border-slate-800 bg-slate-900 py-1.5 pl-8 pr-2.5 text-xs text-slate-100 placeholder:text-slate-500 focus:border-emerald-500 focus:outline-none"
              />
            </div>

            {/* Target Column Selector */}
            <select
              value={replaceTargetColumn}
              onChange={(e) =>
                setReplaceTargetColumn(e.target.value as ReplaceTargetColumn)
              }
              className="rounded-xl border border-slate-800 bg-slate-900 px-2.5 py-1.5 text-xs font-medium text-slate-200 focus:border-cyan-500 focus:outline-none"
            >
              <option value="both">{t.table.replaceTargetBoth}</option>
              <option value="source">{t.table.replaceTargetSource}</option>
              <option value="translated">
                {t.table.replaceTargetTranslated}
              </option>
            </select>

            {/* Case-Sensitive Toggle (Aa) */}
            <button
              type="button"
              onClick={() => setReplaceCaseSensitive(!replaceCaseSensitive)}
              title={t.table.replaceCaseSensitive}
              className={`rounded-xl border px-2.5 py-1.5 font-mono text-xs font-bold transition ${
                replaceCaseSensitive
                  ? "border-cyan-500/50 bg-cyan-500/20 text-cyan-200"
                  : "border-slate-800 bg-slate-900 text-slate-400 hover:text-slate-200"
              }`}
            >
              Aa
            </button>

            {/* Match Count Indicator */}
            <span className="rounded-lg border border-slate-800 bg-slate-900/80 px-2.5 py-1 font-mono text-[11px] text-slate-300">
              {replaceMatchCount} {t.table.replaceMatches}
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={!replaceSearchText || replaceMatchCount === 0}
              onClick={handleReplaceNext}
              className="rounded-xl border border-cyan-500/40 bg-cyan-500/15 px-3 py-1.5 text-xs font-semibold text-cyan-200 transition hover:bg-cyan-500/25 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {t.table.replaceNext}
            </button>
            <button
              type="button"
              disabled={!replaceSearchText || replaceMatchCount === 0}
              onClick={handleReplaceAll}
              className="rounded-xl border border-emerald-500/40 bg-emerald-500/15 px-3 py-1.5 text-xs font-semibold text-emerald-200 transition hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {t.table.replaceAll}
            </button>
            <button
              type="button"
              onClick={() => setIsReplaceOpen(false)}
              title={t.table.closeReplace}
              className="rounded-lg border border-slate-800 bg-slate-900 p-1.5 text-slate-400 transition hover:text-slate-200"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      )}

      {/* Subtitle Cues Table */}
      <div className="max-h-[54vh] overflow-x-auto overflow-y-auto">
        <table className="w-full border-collapse text-left text-xs">
          <thead className="sticky top-0 z-10 border-b border-slate-800 bg-slate-950/95 text-[11px] font-bold uppercase tracking-wider text-slate-400 backdrop-blur">
            <tr>
              <th className="w-16 px-3 py-2.5">{t.table.colIndex}</th>
              <th className="w-36 px-2 py-2.5">{t.table.colStart}</th>
              <th className="w-36 px-2 py-2.5">{t.table.colEnd}</th>
              <th className="px-3 py-2.5">{t.table.colSource}</th>
              <th className="px-3 py-2.5">{t.table.colTranslation}</th>
              <th className="w-28 px-3 py-2.5 text-center">{t.table.colCps}</th>
              <th className="w-28 px-3 py-2.5 text-right">
                {t.table.colActions}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-800/70">
            {filteredCues.map((cue, idx) => {
              const isSelected = cue.id === activeCueId;
              const warningLabels = formatWarningLabels(cue);
              const isLastInAll =
                cue.id ===
                activeProject.cues[activeProject.cues.length - 1]?.id;

              return (
                <tr
                  id={`cue-row-${cue.id}`}
                  key={cue.id}
                  onClick={() => setActiveCueId(cue.id)}
                  className={`transition ${
                    isSelected
                      ? "bg-emerald-950/25"
                      : idx % 2 === 0
                        ? "bg-slate-950/30 hover:bg-slate-900/70"
                        : "bg-slate-900/30 hover:bg-slate-900/70"
                  }`}
                >
                  {/* # + Play Segment Button */}
                  <td className="px-3 py-2.5 align-top">
                    <div className="flex items-center gap-1.5">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          playCueSegment(cue);
                        }}
                        title={t.table.playSegment}
                        className={`flex h-7 w-7 items-center justify-center rounded-lg border transition ${
                          isSelected
                            ? "border-emerald-500/50 bg-emerald-500/20 text-emerald-300"
                            : "border-slate-700 bg-slate-800 text-slate-300 hover:border-cyan-500/40 hover:text-cyan-300"
                        }`}
                      >
                        <Play className="h-3.5 w-3.5 fill-current" />
                      </button>
                      <span className="font-mono text-xs font-semibold text-slate-400">
                        {cue.id}
                      </span>
                    </div>
                  </td>

                  {/* Start Timestamp */}
                  <td className="px-2 py-2.5 align-top">
                    <TimestampCell
                      ms={cue.startMs}
                      onChange={(nextMs) =>
                        updateCue(cue.id, { startMs: nextMs })
                      }
                    />
                  </td>

                  {/* End Timestamp */}
                  <td className="px-2 py-2.5 align-top">
                    <TimestampCell
                      ms={cue.endMs}
                      onChange={(nextMs) =>
                        updateCue(cue.id, { endMs: nextMs })
                      }
                    />
                  </td>

                  {/* Source Text (Editable) */}
                  <td className="px-3 py-2.5 align-top">
                    <textarea
                      rows={2}
                      value={cue.sourceText}
                      onChange={(e) =>
                        updateCue(cue.id, { sourceText: e.target.value })
                      }
                      className="w-full resize-y rounded-xl border border-slate-800/90 bg-slate-950/70 p-2 text-xs leading-relaxed text-slate-200 focus:border-cyan-500 focus:outline-none"
                    />
                  </td>

                  {/* Translated Text (Editable) */}
                  <td className="px-3 py-2.5 align-top">
                    <textarea
                      rows={2}
                      value={cue.translatedText}
                      onChange={(e) =>
                        updateCue(cue.id, { translatedText: e.target.value })
                      }
                      placeholder={cue.sourceText}
                      className="w-full resize-y rounded-xl border border-slate-800/90 bg-slate-950/90 p-2 text-xs font-medium leading-relaxed text-emerald-100 placeholder:text-slate-600 focus:border-emerald-500 focus:outline-none"
                    />
                  </td>

                  {/* CPS Badge & Warnings */}
                  <td className="px-3 py-2.5 text-center align-top">
                    <div className="flex flex-col items-center gap-1">
                      <span
                        className={`inline-flex items-center gap-1 rounded-full border px-2.5 py-0.5 font-mono text-xs font-bold ${getCpsBadgeStyle(
                          cue,
                        )}`}
                      >
                        {cue.cps.toFixed(1)}
                      </span>
                      {warningLabels.length > 0 && (
                        <div className="flex flex-col items-center gap-0.5">
                          {warningLabels.map((lbl) => (
                            <span
                              key={lbl}
                              className="text-[10px] leading-tight text-amber-300/90"
                            >
                              {lbl}
                            </span>
                          ))}
                        </div>
                      )}
                    </div>
                  </td>

                  {/* Row Actions: Split, Merge Next, Delete */}
                  <td className="px-3 py-2.5 text-right align-top">
                    <div className="inline-flex items-center gap-1">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          splitCueAtMidpoint(cue.id);
                        }}
                        title={t.table.splitCue}
                        className="rounded-lg border border-slate-800 bg-slate-900 p-1.5 text-slate-300 transition hover:border-cyan-500/40 hover:bg-cyan-500/15 hover:text-cyan-200"
                      >
                        <Scissors className="h-3.5 w-3.5" />
                      </button>

                      <button
                        type="button"
                        disabled={isLastInAll}
                        onClick={(e) => {
                          e.stopPropagation();
                          mergeCueWithNext(cue.id);
                        }}
                        title={t.table.mergeNext}
                        className="rounded-lg border border-slate-800 bg-slate-900 p-1.5 text-slate-300 transition hover:border-cyan-500/40 hover:bg-cyan-500/15 hover:text-cyan-200 disabled:opacity-30"
                      >
                        <Merge className="h-3.5 w-3.5" />
                      </button>

                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          deleteCueById(cue.id);
                        }}
                        title={t.table.deleteCue}
                        className="rounded-lg border border-slate-800 bg-slate-900 p-1.5 text-slate-400 transition hover:border-rose-500/40 hover:bg-rose-500/15 hover:text-rose-300"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* Keyboard Shortcuts Footer Hint */}
      <div className="flex items-center gap-2 border-t border-slate-800/80 bg-slate-950/60 px-4 py-2 text-[11px] text-slate-400">
        <Keyboard className="h-3.5 w-3.5 text-cyan-400" />
        <span className="font-mono">{t.table.shortcutsHint}</span>
      </div>
    </section>
  );
}
