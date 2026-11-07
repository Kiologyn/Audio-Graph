import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import { LANGUAGES } from "./i18n";

export type Theme = "light" | "system" | "dark";
/** Interface language codes; the list with names is LANGUAGES in i18n.ts */
export type Lang = "en" | "ru" | "es" | "zh" | "ja";

/** Matches Settings in src-tauri/src/settings.rs */
export type Settings = {
  theme: Theme;
  language: Lang;
  /** Turn a running graph's cables off after it stops and on exit; turn them on when it starts */
  disableIdleCables: boolean;
  /** Hide the graph minimap in the bottom right corner of the canvas */
  hideMinimap: boolean;
  /** Start with the system (only an installed app can) */
  autostart: boolean;
  /** An icon in the tray: the closed window hides there; without it closing the window quits */
  tray: boolean;
};

type SettingsState = Settings & {
  error: string | null;
  load: () => Promise<void>;
  set: (patch: Partial<Settings>) => void;
};

const DEFAULTS: Settings = {
  theme: "system",
  // the real default language is chosen by Rust from the system language; this one is for when settings fail to load
  language: (["ru", "es", "zh", "ja"] as const).find((l) => navigator.language.startsWith(l)) ?? "en",
  disableIdleCables: false,
  hideMinimap: false,
  autostart: false,
  tray: false,
};

// writes to disk go in order: the last request carries the latest settings
let queue: Promise<void> = Promise.resolve();

// see graph/store.ts: editing this module in development reloads the whole page
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
    const { error: _error, load: _load, set: _set, ...settings } = get();
    queue = queue.then(
      () => invoke("settings_set", { settings }).then(() => set({ error: null })),
    ).catch(async (e) => {
      // what really is set comes back (the autostart entry may fail to be written)
      await get().load();
      set({ error: String(e) });
    });
  },
}));

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

/** Light or dark theme right now: "system" follows the system theme */
export function resolvedTheme(theme: Theme): "light" | "dark" {
  return theme === "system" ? (darkQuery.matches ? "dark" : "light") : theme;
}

function applyTheme() {
  const { theme, language } = useSettings.getState();
  document.documentElement.dataset.theme = resolvedTheme(theme);
  document.documentElement.lang = LANGUAGES.find((l) => l.code === language)?.html ?? language;
}

/** The page theme and language follow the settings; so does the window theme (title bar) */
export function watchSettings() {
  applyTheme();
  let windowTheme: Theme | null = null;
  const syncWindow = () => {
    const { theme } = useSettings.getState();
    if (theme === windowTheme) return;
    windowTheme = theme;
    // the title bar is an extra: without a Tauri window (page opened in a browser) the interface still renders
    try {
      getCurrentWindow()
        .setTheme(theme === "system" ? null : theme)
        .catch(() => {});
    } catch {
      // no Tauri window
    }
  };
  syncWindow();
  useSettings.subscribe(() => {
    applyTheme();
    syncWindow();
  });
  darkQuery.addEventListener("change", applyTheme);
}
