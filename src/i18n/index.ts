import sr from "./sr.json";
import en from "./en.json";

export type UiLanguage = "sr" | "en";
export type TranslationSchema = typeof sr;

export const dictionaries: Record<UiLanguage, TranslationSchema> = {
  sr,
  en,
};

export function normalizeUiLanguage(lang?: string | null): UiLanguage {
  if (lang && lang.toLowerCase().startsWith("en")) {
    return "en";
  }
  return "sr";
}

export function getDictionary(lang?: string | null): TranslationSchema {
  return dictionaries[normalizeUiLanguage(lang)];
}
