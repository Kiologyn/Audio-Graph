import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import { LANGUAGES } from "./i18n";

export type Theme = "light" | "system" | "dark";
/** Коды языков интерфейса; список с названиями — LANGUAGES в i18n.ts */
export type Lang = "ru" | "en" | "es" | "zh" | "ja";

/** Совпадает с Settings в src-tauri/src/settings.rs */
export type Settings = {
  theme: Theme;
  language: Lang;
  /** Выключать кабели запущенного графа после его остановки и при выходе; при запуске — включать */
  disableIdleCables: boolean;
  /** Убрать карту графа из правого нижнего угла холста */
  hideMinimap: boolean;
  /** Галочка громкости у нового графа: показывать ли на его нодах регуляторы */
  newGraphVolume: boolean;
};

type SettingsState = Settings & {
  error: string | null;
  load: () => Promise<void>;
  set: (patch: Partial<Settings>) => void;
};

const DEFAULTS: Settings = {
  theme: "system",
  // настоящий язык по умолчанию выбирает Rust по языку Windows; это — если настройки не прочитались
  language: (["ru", "es", "zh", "ja"] as const).find((l) => navigator.language.startsWith(l)) ?? "en",
  disableIdleCables: false,
  hideMinimap: false,
  newGraphVolume: true,
};

// запись на диск по очереди: последний запрос несёт самые свежие настройки
let queue: Promise<void> = Promise.resolve();

// см. graph/store.ts: при правке модуля в разработке — полная перезагрузка страницы
import.meta.hot?.accept(() => location.reload());

export const useSettings = create<SettingsState>()((set, get) => ({
  ...DEFAULTS,
  error: null,

  load: async () => {
    try {
      set(await invoke<Settings>("settings_get"));
    } catch (e) {
      set({ error: String(e) });
    }
  },

  set: (patch) => {
    set(patch);
    const { theme, language, disableIdleCables, hideMinimap, newGraphVolume } = get();
    const settings: Settings = { theme, language, disableIdleCables, hideMinimap, newGraphVolume };
    queue = queue.then(
      () => invoke("settings_set", { settings }).then(() => set({ error: null })),
    ).catch((e) => set({ error: String(e) }));
  },
}));

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

/** Светлая или тёмная тема сейчас: «системная» — по теме Windows */
export function resolvedTheme(theme: Theme): "light" | "dark" {
  return theme === "system" ? (darkQuery.matches ? "dark" : "light") : theme;
}

function applyTheme() {
  const { theme, language } = useSettings.getState();
  document.documentElement.dataset.theme = resolvedTheme(theme);
  document.documentElement.lang = LANGUAGES.find((l) => l.code === language)?.html ?? language;
}

/** Тема и язык страницы следуют за настройками; тема окна (заголовок) — тоже */
export function watchSettings() {
  applyTheme();
  let windowTheme: Theme | null = null;
  const syncWindow = () => {
    const { theme } = useSettings.getState();
    if (theme === windowTheme) return;
    windowTheme = theme;
    // заголовок окна — дополнение: если окна Tauri нет (страница открыта в браузере), интерфейс всё равно рисуется
    try {
      getCurrentWindow()
        .setTheme(theme === "system" ? null : theme)
        .catch(() => {});
    } catch {
      // нет окна Tauri
    }
  };
  syncWindow();
  useSettings.subscribe(() => {
    applyTheme();
    syncWindow();
  });
  darkQuery.addEventListener("change", applyTheme);
}
