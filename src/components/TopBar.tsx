import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { getDictionary, normalizeUiLanguage } from "../i18n";
import { isTauriRuntime } from "../lib/subtitleUtils";
import { setWindowMaximized } from "../lib/tauri";
import { useAppStore } from "../stores/appStore";

export function TopBar() {
  const config = useAppStore((s) => s.config);
  const setupStatus = useAppStore((s) => s.setupStatus);
  const activeProject = useAppStore((s) => s.activeProject);
  const banner = useAppStore((s) => s.banner);
  const appVersion = useAppStore((s) => s.appVersion);
  const availableUpdateVersion = useAppStore(
    (s) => s.availableUpdateVersion,
  );

  const setBanner = useAppStore((s) => s.setBanner);
  const setSetupOpen = useAppStore((s) => s.setSetupOpen);
  const setExportOpen = useAppStore((s) => s.setExportOpen);
  const setUpdaterOpen = useAppStore((s) => s.setUpdaterOpen);
  const setBugReportOpen = useAppStore((s) => s.setBugReportOpen);

  const [maximized, setMaximized] = useState(false);

  const lang = normalizeUiLanguage(config.uiLanguage);
  const t = getDictionary(lang);
  const needsSetupDot =
    !(setupStatus?.coreRuntimeReady ?? false) ||
    (config.translation.provider === "localLlama" &&
      !(setupStatus?.hasLlmModel ?? false));
  const canExport = Boolean(activeProject && activeProject.cues.length > 0);

  useEffect(() => {
    if (!isTauriRuntime()) {
      return;
    }
    let mounted = true;
    const syncMax = async () => {
      try {
        const isMax = await setWindowMaximized(true);
        if (mounted) {
          setMaximized(isMax);
        }
      } catch {
        // Ignore in browser/non-window environments
      }
    };
    void syncMax();
    return () => {
      mounted = false;
    };
  }, []);

  const handleMinimize = async () => {
    try {
      if (isTauriRuntime()) {
        await getCurrentWindow().minimize();
      }
    } catch {
      // Browser compatibility fallback
    }
  };

  const handleToggleMaximize = async () => {
    if (isTauriRuntime()) {
      try {
        const isMax = await setWindowMaximized(null);
        setMaximized(isMax);
        return;
      } catch {
        // Fallback toggle below
      }
    }
    setMaximized((prev) => !prev);
  };

  const handleClose = async () => {
    try {
      if (isTauriRuntime()) {
        await getCurrentWindow().close();
      }
    } catch {
      // Browser compatibility fallback
    }
  };

  return (
    <header id="titlebar" data-tauri-drag-region>
      <div className="titlebar-left" data-tauri-drag-region>
        <button
          type="button"
          className="tb-quick-btn"
          onClick={() => setSetupOpen(true)}
          title={t.topBar.setupWizard}
        >
          {needsSetupDot && (
            <span className="tb-quick-dot" aria-hidden="true" />
          )}
          <span>{lang === "en" ? "Models" : "Modeli"}</span>
        </button>

        <button
          type="button"
          className="tb-quick-btn"
          disabled={!canExport}
          onClick={() => setExportOpen(true)}
          title={t.topBar.export}
        >
          <span>{lang === "en" ? "Export" : "Izvoz"}</span>
        </button>

        <button
          type="button"
          className="tb-quick-btn"
          onClick={() => setBugReportOpen(true)}
          title={t.topBar.reportBug}
        >
          <span>{t.topBar.reportBug}</span>
        </button>

        {banner && (
          <div
            className={`tb-toast ${banner.type}`}
            role="status"
            title={banner.message}
          >
            <span className="tb-toast-msg">{banner.message}</span>
            <button
              type="button"
              className="tb-toast-close"
              onClick={() => setBanner(null)}
              aria-label={
                lang === "sr" ? "Zatvori obaveštenje" : "Dismiss notification"
              }
              title={lang === "sr" ? "Zatvori" : "Dismiss"}
            >
              ×
            </button>
          </div>
        )}
      </div>

      <div className="titlebar-center">
        <span className="app-name">EasySRT Studio</span>
        <button
          type="button"
          className="app-version"
          onClick={() => setUpdaterOpen(true)}
          title={
            availableUpdateVersion
              ? lang === "en"
                ? `Update available: v${availableUpdateVersion} (click to install from GitHub)`
                : `Dostupno ažuriranje: v${availableUpdateVersion} (klikni za instalaciju sa GitHub-a)`
              : lang === "en"
                ? "Check version & GitHub updates"
                : "Proveri verziju i ažuriranja sa GitHub-a"
          }
        >
          {availableUpdateVersion && (
            <span className="app-version-dot" aria-hidden="true" />
          )}
          <span>v{appVersion}</span>
        </button>
      </div>

      <div className="window-controls">
        <button
          id="btn-minimize"
          className="win-btn"
          type="button"
          aria-label="Minimize"
          title={lang === "en" ? "Minimize" : "Umanji"}
          onClick={() => void handleMinimize()}
        >
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.15"
            shapeRendering="crispEdges"
          >
            <line x1="0" y1="5" x2="10" y2="5" />
          </svg>
        </button>

        <button
          id="btn-maximize"
          className="win-btn"
          type="button"
          aria-label={maximized ? "Restore" : "Maximize"}
          title={
            maximized
              ? lang === "en"
                ? "Restore"
                : "Vrati veličinu"
              : lang === "en"
                ? "Maximize"
                : "Uvećaj"
          }
          onClick={() => void handleToggleMaximize()}
        >
          {maximized ? (
            <svg
              id="icon-restore"
              width="10"
              height="10"
              viewBox="0 0 10 10"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.1"
              shapeRendering="crispEdges"
            >
              <polyline points="2.5,2.5 2.5,0.5 9.5,0.5 9.5,7.5 7.5,7.5" />
              <rect x="0.5" y="2.5" width="7" height="7" />
            </svg>
          ) : (
            <svg
              id="icon-maximize"
              width="10"
              height="10"
              viewBox="0 0 10 10"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.1"
              shapeRendering="crispEdges"
            >
              <rect x="0.5" y="0.5" width="9" height="9" />
            </svg>
          )}
        </button>

        <button
          id="btn-close"
          className="win-btn close"
          type="button"
          aria-label="Close"
          title={lang === "en" ? "Close" : "Zatvori"}
          onClick={() => void handleClose()}
        >
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinecap="round"
          >
            <line x1="1" y1="1" x2="9" y2="9" />
            <line x1="9" y1="1" x2="1" y2="9" />
          </svg>
        </button>
      </div>
    </header>
  );
}
