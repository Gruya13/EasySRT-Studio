/**
 * Single source of truth for the frontend fallback version.
 * Injected at build/dev time by Vite from root `package.json`.
 */
export const APP_VERSION: string =
  typeof __APP_VERSION__ !== "undefined" && __APP_VERSION__.length > 0
    ? __APP_VERSION__
    : "0.0.0-dev";
