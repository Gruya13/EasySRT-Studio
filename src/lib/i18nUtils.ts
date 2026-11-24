export type UiLanguage = "sr" | "en";

/** Normalize any persisted or OS locale tag to `"en"` unless Serbian is explicitly chosen. */
export function normalizeUiLanguage(lang?: string | null): UiLanguage {
  if (lang && lang.toLowerCase().startsWith("sr")) {
    return "sr";
  }
  return "en";
}
