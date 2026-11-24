import sr from "./sr.json";
import en from "./en.json";
import { normalizeUiLanguage, type UiLanguage } from "../lib/i18nUtils";

export { normalizeUiLanguage, type UiLanguage };
export type TranslationSchema = typeof sr;

export const dictionaries: Record<UiLanguage, TranslationSchema> = {
  sr,
  en,
};

export function getDictionary(lang?: string | null): TranslationSchema {
  return dictionaries[normalizeUiLanguage(lang)];
}
