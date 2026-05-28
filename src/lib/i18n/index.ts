import ja from "./ja.json";
import en from "./en.json";
import { persisted } from "../persisted";

type Locale = "ja" | "en";
type TranslationKey = keyof typeof ja;

const translations: Record<Locale, Record<string, string>> = { ja, en };

const [locale, setLocale] = persisted<Locale>("locale", "ja");

export function t(key: TranslationKey): string {
  return translations[locale()][key] ?? key;
}

export { locale, setLocale };
export type { Locale, TranslationKey };
