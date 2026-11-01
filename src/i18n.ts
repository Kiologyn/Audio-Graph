import { useCallback } from "react";
import en from "./locales/en.json";
import es from "./locales/es.json";
import ja from "./locales/ja.json";
import ru from "./locales/ru.json";
import zh from "./locales/zh.json";
import { useSettings, type Lang } from "./settings";

/**
 * Interface translation, like Android string resources: the code uses string codes
 * (t("start")), and every language has a file src/locales/<language>.json mapping
 * codes to its strings. English (en.json) is the base: a string missing in a language is shown
 * in English. The files are shared with Rust (src-tauri/src/i18n.rs). {name} in a string is a
 * placeholder. Add every new code to all language files right away.
 *
 * A new language: a row in LANGUAGES, its dictionary in DICTIONARIES here, a Language variant
 * in settings.rs and its dictionary in i18n.rs.
 */
export const LANGUAGES: { code: Lang; name: string; english: string; html: string }[] = [
  { code: "en", name: "English", english: "English", html: "en" },
  { code: "ru", name: "Русский", english: "Russian", html: "ru" },
  { code: "es", name: "Español", english: "Spanish", html: "es" },
  { code: "zh", name: "中文", english: "Chinese (Simplified)", html: "zh-CN" },
  { code: "ja", name: "日本語", english: "Japanese", html: "ja" },
];

const DICTIONARIES: Record<Lang, Record<string, string>> = { en, ru, es, zh, ja };

function fill(text: string, vars?: Record<string, string | number>): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (all, key: string) => (key in vars ? String(vars[key]) : all));
}

const missing = new Set<string>();

export function translate(lang: Lang, code: string, vars?: Record<string, string | number>): string {
  const found = DICTIONARIES[lang][code];
  if (found === undefined && import.meta.env.DEV && !missing.has(`${lang}:${code}`)) {
    missing.add(`${lang}:${code}`);
    console.warn(`missing string (${lang}): ${code}`);
  }
  // missing in the language: English; missing in English too: the code itself, so it's noticed
  return fill(found ?? DICTIONARIES.en[code] ?? code, vars);
}

/** Translation outside components: a component that shows it re-renders on a language change by itself */
export function t(code: string, vars?: Record<string, string | number>): string {
  return translate(useSettings.getState().language, code, vars);
}

/** Translation in a component: subscribes the component to language changes */
export function useT() {
  const lang = useSettings((s) => s.language);
  return useCallback((code: string, vars?: Record<string, string | number>) => translate(lang, code, vars), [lang]);
}

/** The string of a code in any of the languages: a default label the user hasn't changed */
export function isAnyTranslation(value: string, code: string): boolean {
  return Object.values(DICTIONARIES).some((d) => d[code] === value);
}
