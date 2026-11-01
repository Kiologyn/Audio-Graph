import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { useDevices } from "./devices";
import { flushSave } from "./library";

/** Matches engine/mod.rs and engine/plan.rs */
export type Issue = { nodes: string[]; edges: string[]; message: string };
export type DeviceError = { device: string; message: string };
export type StreamStatus = { sink: string; sources: string[]; errors: DeviceError[] };
export type AppStatus = {
  node: string;
  /**
   * not_running: the app is closed, no_audio: it hasn't opened audio yet;
   * in both cases devices are assigned by themselves as soon as possible
   */
  state: "routed" | "not_running" | "no_audio" | "failed";
  error: string | null;
};
export type EngineStatus = {
  /** ids of running graphs in start order */
  running: string[];
  /** the graph the issues belong to: the one open in the window, or else the first running one */
  graphId: string | null;
  issues: Issue[];
  /** devices of all running graphs */
  streams: StreamStatus[];
  /** apps of all running graphs */
  apps: AppStatus[];
  /** running graphs where something doesn't work */
  troubled: string[];
};

const EMPTY: EngineStatus = { running: [], graphId: null, issues: [], streams: [], apps: [], troubled: [] };

type EngineState = {
  status: EngineStatus;
  error: string | null;
  init: () => void;
  start: (graphId: string) => Promise<void>;
  stop: (graphId: string) => Promise<void>;
  /** Node volume on the fly while the slider moves: the engine doesn't wait for autosave */
  volume: (graphId: string, node: string, volume: number) => void;
  /** Node balance on the fly, the same way */
  balance: (graphId: string, node: string, balance: number) => void;
};

let initOnce = false;

// see store.ts: editing this module in development reloads the whole page
import.meta.hot?.accept(() => location.reload());

/** Engine state; Rust is the source of truth, it arrives here with the engine-status event */
export const useEngine = create<EngineState>()((set, get) => ({
  status: EMPTY,
  error: null,

  init: () => {
    if (initOnce) return;
    initOnce = true;
    listen<EngineStatus>("engine-status", (e) => {
      set({ status: e.payload });
      // the engine may have changed the default devices: "System" and "default" nodes show them
      useDevices.getState().refresh();
    });
    invoke<EngineStatus>("engine_status").then((status) => set({ status }));
    // a device was plugged in or out, turned on or off: nodes show it at once
    listen("devices-changed", () => useDevices.getState().refresh());
  },

  start: async (graphId) => {
    // the engine reads the graph from disk: write unsaved edits first
    await flushSave();
    // the button responds at once, details come with the status
    const without = (s: EngineStatus) => s.running.filter((id) => id !== graphId);
    set({ status: { ...get().status, running: [...without(get().status), graphId] }, error: null });
    try {
      await invoke("engine_start", { id: graphId });
    } catch (e) {
      set({ status: { ...get().status, running: without(get().status) }, error: String(e) });
    }
  },

  volume: (graphId, node, volume) => {
    invoke("engine_volume", { graph: graphId, node, volume }).catch(() => {});
  },

  balance: (graphId, node, balance) => {
    invoke("engine_balance", { graph: graphId, node, balance }).catch(() => {});
  },

  stop: async (graphId) => {
    const status = get().status;
    set({ status: { ...status, running: status.running.filter((id) => id !== graphId) }, error: null });
    try {
      await invoke("engine_stop", { id: graphId });
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));
