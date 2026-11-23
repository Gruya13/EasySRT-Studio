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
import { useAppStore } from "./stores/appStore";

export function App() {
  const initApp = useAppStore((s) => s.initApp);

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
          <span>Besplatni alat koji izrađuje i održava</span>
          <strong className="footer-brand">IG-Systems</strong>
          <span className="footer-sep" aria-hidden="true">
            •
          </span>
          <span className="footer-year">© 2026</span>
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
