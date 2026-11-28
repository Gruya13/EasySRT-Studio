import { bindings, defineConfig, triggers } from "cf/config";

// Deployed live at https://easysrt-telemetry.ig-systems.rs (zone: ig-systems.rs)
export default defineConfig({
  worker: {
    name: "easysrt-telemetry",
    entrypoint: "./src/index.ts",
    compatibilityDate: "2026-11-23",
    workersDev: false,
    domains: ["easysrt-telemetry.ig-systems.rs"],
    // Zero-IP / PRIVACY.md alignment: disable automatic HTTP invocation logs and traces
    // (which can capture client metadata/headers) and redact URL query strings.
    // Only explicit PII-free JSON console events emitted by src/index.ts are recorded.
    observability: {
      enabled: true,
      redactQueryString: true,
      logs: {
        enabled: true,
        invocationLogs: false,
      },
      traces: {
        enabled: false,
      },
    },
    triggers: [triggers.scheduled({ schedule: "0 3 * * *" })],
    env: {
      GITHUB_REPO: bindings.text("Gruya13/EasySRT-Studio"),
      GITHUB_ISSUES_TOKEN: bindings.secret(),
      TELEMETRY_ADMIN_TOKEN: bindings.secret(),
      SUBNET_HMAC_SECRET: bindings.secret(),
      DB: bindings.d1({ name: "easysrt-telemetry-db" }),
    },
  },
});
