import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { AppRef } from "./types";

/** Matches AudioApp in src-tauri/src/apps/mod.rs */
export type AudioApp = {
  exe: string;
  path: string;
  name: string;
  /** has an audio session: the app is shown in the volume mixer */
  hasAudio: boolean;
  /** is playing or recording sound right now */
  playing: boolean;
};

/** Running apps. Like devices, refreshed every time the list opens. */
type AppsState = {
  apps: AudioApp[];
  loaded: boolean;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
};

// concurrent refresh requests are merged into one
let inflight: Promise<void> | null = null;

// see store.ts: editing this module in development reloads the whole page
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
 * The running app for the one chosen in a node: by full path first, then by file name
 * (an update can change the folder, as with Discord: app-1.0.9266).
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
