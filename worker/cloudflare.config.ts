import { bindings, defineConfig } from "cf/config";

// Deployed live at https://easysrt-telemetry.ig-systems.rs (zone: ig-systems.rs)
export default defineConfig({
  worker: {
    name: "easysrt-telemetry",
    entrypoint: "./src/index.ts",
    compatibilityDate: "2026-11-23",
    observability: {
      enabled: true,
      logs: {
        enabled: true,
      },
      traces: {
        enabled: true,
      },
    },
    env: {
      GITHUB_REPO: bindings.text("Gruya13/EasySRT-Studio"),
      GITHUB_ISSUES_TOKEN: bindings.secret(),
    },
  },
});
