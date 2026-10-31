import { useCallback } from "react";
import en from "./locales/en.json";
import es from "./locales/es.json";
import ja from "./locales/ja.json";
import zh from "./locales/zh.json";
import { useSettings, type Lang } from "./settings";

/**
 * Перевод интерфейса. Ключ — русский текст, как он написан в коде; словари других языков —
 * src/locales/<код>.json, общие с Rust (src-tauri/src/i18n.rs). {name} в тексте — подстановка.
 * Каждую новую строку сразу добавлять во все словари.
 *
 * Новый язык: строка в LANGUAGES, словарь здесь в DICTIONARIES, вариант Language в settings.rs
 * и словарь в i18n.rs.
 */
export const LANGUAGES: { code: Lang; name: string; english: string; html: string }[] = [
  { code: "ru", name: "Русский", english: "Russian", html: "ru" },
  { code: "en", name: "English", english: "English", html: "en" },
  { code: "es", name: "Español", english: "Spanish", html: "es" },
  { code: "zh", name: "中文", english: "Chinese (Simplified)", html: "zh-CN" },
  { code: "ja", name: "日本語", english: "Japanese", html: "ja" },
];

const DICTIONARIES: Record<Exclude<Lang, "ru">, Record<string, string>> = { en, es, zh, ja };

function fill(text: string, vars?: Record<string, string | number>): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (all, key: string) => (key in vars ? String(vars[key]) : all));
}

const missing = new Set<string>();

export function translate(lang: Lang, text: string, vars?: Record<string, string | number>): string {
  if (lang === "ru") return fill(text, vars);
  const found = DICTIONARIES[lang][text];
  if (found === undefined && import.meta.env.DEV && !missing.has(`${lang}:${text}`)) {
    missing.add(`${lang}:${text}`);
    console.warn(`нет перевода (${lang}): ${text}`);
  }
  // нет в словаре языка — по-английски, нет и там — по-русски
  return fill(found ?? DICTIONARIES.en[text] ?? text, vars);
}

/** Перевод вне компонентов: при смене языка компонент, который это показывает, перерисуется сам */
export function t(text: string, vars?: Record<string, string | number>): string {
  return translate(useSettings.getState().language, text, vars);
}

/** Перевод в компоненте: подписывает компонент на смену языка */
export function useT() {
  const lang = useSettings((s) => s.language);
  return useCallback((text: string, vars?: Record<string, string | number>) => translate(lang, text, vars), [lang]);
}

/** Текст на любом из языков: подпись по умолчанию, которую пользователь не менял */
export function isAnyTranslation(value: string, text: string): boolean {
  return value === text || Object.values(DICTIONARIES).some((d) => d[text] === value);
}
