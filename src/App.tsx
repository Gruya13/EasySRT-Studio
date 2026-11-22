import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { useEffect } from "react";
import { AudioSegmentPlayer } from "./components/AudioSegmentPlayer";
import { ExportModal } from "./components/ExportModal";
import { JobWorkspace } from "./components/JobWorkspace";
import { OnlineConfirmModal } from "./components/OnlineConfirmModal";
import { SettingsModal } from "./components/SettingsModal";
import { SetupWizard } from "./components/SetupWizard";
import { SubtitleTable } from "./components/SubtitleTable";
import { TopBar } from "./components/TopBar";
import { UpdaterModal } from "./components/UpdaterModal";
import { useAppStore } from "./stores/appStore";

export function App() {
  const initApp = useAppStore((s) => s.initApp);
  const banner = useAppStore((s) => s.banner);
  const setBanner = useAppStore((s) => s.setBanner);

  useEffect(() => {
    void initApp();
  }, [initApp]);

  return (
    <div className="flex min-h-screen flex-col bg-slate-950 text-slate-100 selection:bg-emerald-500/30 selection:text-emerald-200">
      <TopBar />

      {banner && (
        <div className="mx-auto mt-3 w-full max-w-7xl px-4">
          <div
            className={`flex items-center justify-between gap-3 rounded-xl border px-4 py-2.5 text-xs font-medium shadow-md ${
              banner.type === "error"
                ? "border-rose-500/40 bg-rose-950/40 text-rose-200"
                : banner.type === "success"
                  ? "border-emerald-500/40 bg-emerald-950/40 text-emerald-200"
                  : "border-cyan-500/40 bg-cyan-950/40 text-cyan-200"
            }`}
          >
            <div className="flex items-center gap-2">
              {banner.type === "error" ? (
                <AlertCircle className="h-4 w-4 shrink-0 text-rose-400" />
              ) : banner.type === "success" ? (
                <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-400" />
              ) : (
                <Info className="h-4 w-4 shrink-0 text-cyan-400" />
              )}
              <span>{banner.message}</span>
            </div>

            <button
              type="button"
              onClick={() => setBanner(null)}
              className="rounded-lg p-1 opacity-75 transition hover:bg-black/20 hover:opacity-100"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      )}

      <main className="mx-auto flex w-full max-w-7xl flex-1 flex-col gap-4 px-4 py-4">
        <JobWorkspace />
        <SubtitleTable />
      </main>

      <AudioSegmentPlayer />

      {/* Modals */}
      <SetupWizard />
      <SettingsModal />
      <ExportModal />
      <OnlineConfirmModal />
      <UpdaterModal />
    </div>
  );
}

export default App;
