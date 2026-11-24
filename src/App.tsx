import { Bug } from "lucide-react";
import { useEffect } from "react";
import { BugReportModal } from "./components/BugReportModal";
import { DualEditorBand } from "./components/DualEditorBand";
import { ExportModal } from "./components/ExportModal";
import { FilesPanel } from "./components/FilesPanel";
import { OnlineConfirmModal } from "./components/OnlineConfirmModal";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupWizard } from "./components/SetupWizard";
import { TopBar } from "./components/TopBar";
import { UpdaterModal } from "./components/UpdaterModal";
import { WaveformTimeline } from "./components/WaveformTimeline";
import { normalizeUiLanguage } from "./i18n";
import { useAppStore } from "./stores/appStore";

export function App() {
  const initApp = useAppStore((s) => s.initApp);
  const config = useAppStore((s) => s.config);
  const setBugReportOpen = useAppStore((s) => s.setBugReportOpen);
  const lang = normalizeUiLanguage(config.uiLanguage);

  useEffect(() => {
    void initApp();
  }, [initApp]);

  return (
    <div className="window-shell">
      <TopBar />

      <main id="canvas">
        <div id="canvas-stage">
          <FilesPanel />
          <SettingsPanel />
        </div>

        <DualEditorBand />

        <WaveformTimeline />
      </main>

      <footer id="footer">
        <span className="footer-text">
          <span>
            {lang === "en"
              ? "Free tool crafted & maintained by"
              : "Besplatni alat koji izrađuje i održava"}
          </span>
          <strong className="footer-brand">IG-Systems</strong>
          <span className="footer-sep" aria-hidden="true">
            •
          </span>
          <span className="footer-year">© 2026</span>
          <span className="footer-sep" aria-hidden="true">
            •
          </span>
          <button
            id="footer-bug-btn"
            type="button"
            className="footer-bug-btn"
            onClick={() => setBugReportOpen(true)}
            title={
              lang === "en"
                ? "Report a bug or suggest a change"
                : "Prijavi bug ili predloži izmenu"
            }
            aria-label={
              lang === "en"
                ? "Report a bug or suggest a change"
                : "Prijavi bug ili predloži izmenu"
            }
          >
            <Bug size={13} aria-hidden="true" />
          </button>
        </span>
      </footer>

      {/* Modals */}
      <SetupWizard />
      <ExportModal />
      <OnlineConfirmModal />
      <UpdaterModal />
      <BugReportModal />
    </div>
  );
}

export default App;
