import { open } from "@tauri-apps/plugin-dialog";
import {
  CheckCircle2,
  FileDown,
  FolderOpen,
  Loader2,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime } from "../lib/subtitleUtils";
import { useAppStore } from "../stores/appStore";
import type { ExportFormat, ExportOptions } from "../types/models";

export function ExportModal() {
  const isOpen = useAppStore((s) => s.isExportOpen);
  const setExportOpen = useAppStore((s) => s.setExportOpen);
  const config = useAppStore((s) => s.config);
  const activeProject = useAppStore((s) => s.activeProject);
  const runExportSubtitles = useAppStore((s) => s.runExportSubtitles);

  const [options, setOptions] = useState<ExportOptions>(config.export);
  const [exporting, setExporting] = useState(false);
  const [writtenFiles, setWrittenFiles] = useState<string[]>([]);

  useEffect(() => {
    if (isOpen) {
      setOptions(config.export);
      setWrittenFiles([]);
    }
  }, [isOpen, config.export]);

  if (!isOpen || !activeProject) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const toggleFormat = (fmt: ExportFormat) => {
    const exists = options.formats.includes(fmt);
    const nextFormats = exists
      ? options.formats.filter((f) => f !== fmt)
      : [...options.formats, fmt];
    if (nextFormats.length === 0) {
      return;
    }
    setOptions({ ...options, formats: nextFormats });
  };

  const handleChooseFolder = async () => {
    if (isTauriRuntime()) {
      try {
        const dir = await open({
          directory: true,
          multiple: false,
        });
        if (typeof dir === "string" && dir) {
          setOptions({
            ...options,
            saveBesideAudio: false,
            outputDir: dir,
          });
        }
      } catch {
        // Ignore dialog cancel
      }
    }
  };

  const handleExport = async () => {
    setExporting(true);
    try {
      const res = await runExportSubtitles(options);
      if (res) {
        setWrittenFiles(res.writtenFiles);
      }
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-slate-800 bg-slate-900/60 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-cyan-500/30 bg-cyan-500/15 text-cyan-300">
              <FileDown className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-slate-100">
                {t.export.title}
              </h3>
              <p className="text-xs text-slate-400">{t.export.subtitle}</p>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setExportOpen(false)}
            className="rounded-lg p-1 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <div className="space-y-4 p-5 text-xs">
          {/* Formats */}
          <div className="space-y-2">
            <label className="block text-[11px] font-bold uppercase tracking-wider text-slate-400">
              {t.export.formatsLabel}
            </label>
            <div className="space-y-1.5">
              {(
                [
                  ["srt", t.export.formatSrt],
                  ["vtt", t.export.formatVtt],
                  ["txt", t.export.formatTxt],
                ] as const
              ).map(([fmt, label]) => (
                <label
                  key={fmt}
                  className={`flex cursor-pointer items-center gap-2.5 rounded-xl border p-2.5 transition ${
                    options.formats.includes(fmt)
                      ? "border-cyan-500/40 bg-cyan-950/20 text-slate-100"
                      : "border-slate-800 bg-slate-900/50 text-slate-400"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={options.formats.includes(fmt)}
                    onChange={() => toggleFormat(fmt)}
                    className="h-4 w-4 rounded border-slate-700 bg-slate-900 text-cyan-500"
                  />
                  <span className="font-medium">{label}</span>
                </label>
              ))}
            </div>
          </div>

          {/* Options: includeSourceSrt & utf8Bom */}
          <div className="space-y-2 border-t border-slate-800/80 pt-3">
            <label className="flex cursor-pointer items-center gap-2.5 text-slate-200">
              <input
                type="checkbox"
                checked={options.includeSourceSrt}
                onChange={(e) =>
                  setOptions({
                    ...options,
                    includeSourceSrt: e.target.checked,
                  })
                }
                className="h-4 w-4 rounded border-slate-700 bg-slate-900 text-emerald-500"
              />
              <span className="font-medium">{t.export.includeSourceSrt}</span>
            </label>

            <label className="flex cursor-pointer items-center gap-2.5 text-slate-200">
              <input
                type="checkbox"
                checked={options.utf8Bom}
                onChange={(e) =>
                  setOptions({
                    ...options,
                    utf8Bom: e.target.checked,
                  })
                }
                className="h-4 w-4 rounded border-slate-700 bg-slate-900 text-emerald-500"
              />
              <span className="font-medium">{t.export.utf8Bom}</span>
            </label>
          </div>

          {/* Output Location */}
          <div className="space-y-2 border-t border-slate-800/80 pt-3">
            <label className="block text-[11px] font-bold uppercase tracking-wider text-slate-400">
              {t.export.locationLabel}
            </label>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() =>
                  setOptions({ ...options, saveBesideAudio: true })
                }
                className={`rounded-xl border px-3 py-2 text-left text-xs font-semibold transition ${
                  options.saveBesideAudio
                    ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-200"
                    : "border-slate-800 bg-slate-900 text-slate-400"
                }`}
              >
                {t.export.saveBesideAudio}
              </button>

              <button
                type="button"
                onClick={() =>
                  setOptions({ ...options, saveBesideAudio: false })
                }
                className={`rounded-xl border px-3 py-2 text-left text-xs font-semibold transition ${
                  !options.saveBesideAudio
                    ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-200"
                    : "border-slate-800 bg-slate-900 text-slate-400"
                }`}
              >
                {t.export.customFolder}
              </button>
            </div>

            {!options.saveBesideAudio && (
              <div className="flex gap-2 pt-1">
                <input
                  type="text"
                  value={options.outputDir ?? ""}
                  onChange={(e) =>
                    setOptions({
                      ...options,
                      outputDir: e.target.value || null,
                    })
                  }
                  placeholder="C:/Subtitles"
                  className="flex-1 rounded-xl border border-slate-800 bg-slate-900 px-3 py-1.5 font-mono text-xs text-slate-200 focus:border-cyan-500 focus:outline-none"
                />
                <button
                  type="button"
                  onClick={() => void handleChooseFolder()}
                  className="inline-flex items-center gap-1.5 rounded-xl border border-slate-700 bg-slate-800 px-3 py-1.5 text-xs font-semibold text-slate-200 hover:bg-slate-700"
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                  <span>{t.export.chooseFolder}</span>
                </button>
              </div>
            )}
          </div>

          {/* Written Files Result */}
          {writtenFiles.length > 0 && (
            <div className="space-y-1.5 rounded-xl border border-emerald-500/30 bg-emerald-950/25 p-3">
              <div className="flex items-center gap-1.5 font-semibold text-emerald-300">
                <CheckCircle2 className="h-4 w-4" />
                <span>{t.export.exportSuccess}</span>
              </div>
              <ul className="space-y-1 font-mono text-[11px] text-slate-300">
                {writtenFiles.map((file) => (
                  <li key={file} className="truncate">
                    • {file}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2.5 border-t border-slate-800 bg-slate-900/70 px-5 py-3.5">
          <button
            type="button"
            onClick={() => setExportOpen(false)}
            className="rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 transition hover:bg-slate-700"
          >
            {t.export.close}
          </button>

          <button
            type="button"
            disabled={exporting || options.formats.length === 0}
            onClick={() => void handleExport()}
            className="inline-flex items-center gap-2 rounded-xl bg-gradient-to-r from-emerald-500 to-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-emerald-400 hover:to-cyan-400 disabled:opacity-50"
          >
            {exporting ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{t.export.exporting}</span>
              </>
            ) : (
              <>
                <FileDown className="h-4 w-4" />
                <span>{t.export.exportNow}</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
