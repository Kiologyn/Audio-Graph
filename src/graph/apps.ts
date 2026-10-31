import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { AppRef } from "./types";

/** Совпадает с AudioApp в src-tauri/src/apps/mod.rs */
export type AudioApp = {
  exe: string;
  path: string;
  name: string;
  /** есть звуковой сеанс: приложение видно в микшере громкости */
  hasAudio: boolean;
  /** сейчас воспроизводит или записывает звук */
  playing: boolean;
};

/** Запущенные приложения. Как и устройства, обновляются при каждом открытии списка. */
type AppsState = {
  apps: AudioApp[];
  loaded: boolean;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
};

// одновременные запросы обновления склеиваются в один
let inflight: Promise<void> | null = null;

// см. store.ts: при правке модуля в разработке — полная перезагрузка страницы
import.meta.hot?.accept(() => location.reload());

export const useApps = create<AppsState>()((set, get) => ({
  apps: [],
  loaded: false,
  loading: false,
  error: null,

  refresh: () =>
    (inflight ??= (async () => {
      set({ loading: true });
      try {
        const apps = await invoke<AudioApp[]>("audio_apps");
        const same = JSON.stringify(apps) === JSON.stringify(get().apps);
        set({ ...(same ? {} : { apps }), loaded: true, error: null });
      } catch (e) {
        set({ error: String(e) });
      } finally {
        set({ loading: false });
        inflight = null;
      }
    })()),
}));

/**
 * Запущенное приложение для выбранного в ноде: сначала по полному пути, потом по имени файла
 * (после обновления папка может смениться, как у Discord: app-1.0.9266).
 */
export function findApp(ref: AppRef | null, apps: AudioApp[]): AudioApp | undefined {
  if (!ref) return undefined;
  const path = ref.path.toLowerCase();
  const exe = ref.exe.toLowerCase();
  return (
    (path ? apps.find((a) => a.path.toLowerCase() === path) : undefined) ??
    apps.find((a) => a.exe.toLowerCase() === exe)
  );
}

export function toAppRef(app: AudioApp): AppRef {
  return { exe: app.exe, path: app.path, name: app.name };
}
