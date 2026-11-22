import { Cloud, ShieldAlert, X } from "lucide-react";
import { useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { useAppStore } from "../stores/appStore";

export function OnlineConfirmModal() {
  const config = useAppStore((s) => s.config);
  const onlineConfirm = useAppStore((s) => s.onlineConfirm);
  const closeOnlineConfirm = useAppStore((s) => s.closeOnlineConfirm);
  const confirmOnlineAction = useAppStore((s) => s.confirmOnlineAction);

  const [rememberForSession, setRememberForSession] = useState(true);

  if (!onlineConfirm.open) {
    return null;
  }

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);

  const endpointDescription =
    onlineConfirm.provider === "deepSeek"
      ? `DeepSeek API (${config.translation.deepseekModel || "deepseek-chat"})`
      : `${config.translation.customBaseUrl || "Custom OpenAI API"} (${
          config.translation.customModel || "custom-model"
        })`;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg overflow-hidden rounded-2xl border border-amber-500/40 bg-slate-950 shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-slate-800 bg-amber-950/20 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-amber-500/40 bg-amber-500/15 text-amber-300">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-slate-100">
                {t.onlineConfirm.title}
              </h3>
              <span className="font-mono text-[11px] text-amber-300">
                {onlineConfirm.actionLabel}
              </span>
            </div>
          </div>

          <button
            type="button"
            onClick={closeOnlineConfirm}
            className="rounded-lg p-1 text-slate-400 transition hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <div className="space-y-4 p-5 text-xs">
          <p className="leading-relaxed text-slate-300">
            {t.onlineConfirm.description}
          </p>

          <div className="flex items-center gap-2.5 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 font-mono text-xs font-semibold text-amber-200">
            <Cloud className="h-4 w-4 shrink-0 text-amber-400" />
            <span className="truncate">{endpointDescription}</span>
          </div>

          <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-3 text-slate-400">
            {t.onlineConfirm.privacyNote}
          </div>

          <label className="flex cursor-pointer items-center gap-2.5 pt-1 text-slate-200">
            <input
              type="checkbox"
              checked={rememberForSession}
              onChange={(e) => setRememberForSession(e.target.checked)}
              className="h-4 w-4 rounded border-slate-700 bg-slate-900 text-amber-500 focus:ring-amber-500"
            />
            <span className="font-medium">
              {t.onlineConfirm.rememberSession}
            </span>
          </label>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2.5 border-t border-slate-800 bg-slate-900/70 px-5 py-3.5">
          <button
            type="button"
            onClick={closeOnlineConfirm}
            className="rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 transition hover:bg-slate-700"
          >
            {t.onlineConfirm.cancel}
          </button>
          <button
            type="button"
            onClick={() => confirmOnlineAction(rememberForSession)}
            className="rounded-xl bg-gradient-to-r from-amber-500 to-orange-500 px-4 py-2 text-xs font-bold text-slate-950 shadow transition hover:from-amber-400 hover:to-orange-400"
          >
            {t.onlineConfirm.confirm}
          </button>
        </div>
      </div>
    </div>
  );
}
