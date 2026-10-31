import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { useDevices } from "./devices";
import { flushSave } from "./library";

/** Совпадает с engine/mod.rs и engine/plan.rs */
export type Issue = { nodes: string[]; edges: string[]; message: string };
export type DeviceError = { device: string; message: string };
export type StreamStatus = { sink: string; sources: string[]; errors: DeviceError[] };
export type AppStatus = {
  node: string;
  /**
   * not_running — приложение закрыто, no_audio — ещё не открывало звук;
   * в обоих случаях устройства назначатся сами, как только станет можно
   */
  state: "routed" | "not_running" | "no_audio" | "failed";
  error: string | null;
};
export type EngineStatus = {
  /** id запущенных графов в порядке запуска */
  running: string[];
  /** граф, к которому относятся issues: открытый в окне, а без него — первый запущенный */
  graphId: string | null;
  issues: Issue[];
  /** устройства всех запущенных графов */
  streams: StreamStatus[];
  /** приложения всех запущенных графов */
  apps: AppStatus[];
  /** запущенные графы, у которых что-то не работает */
  troubled: string[];
};

const EMPTY: EngineStatus = { running: [], graphId: null, issues: [], streams: [], apps: [], troubled: [] };

type EngineState = {
  status: EngineStatus;
  error: string | null;
  init: () => void;
  start: (graphId: string) => Promise<void>;
  stop: (graphId: string) => Promise<void>;
  /** Громкость ноды на ходу, пока двигают ползунок: движку не ждать автосохранения */
  volume: (graphId: string, node: string, volume: number) => void;
};

let initOnce = false;

// см. store.ts: при правке модуля в разработке — полная перезагрузка страницы
import.meta.hot?.accept(() => location.reload());

/** Состояние движка; источник правды — Rust, сюда оно приходит событием engine-status */
export const useEngine = create<EngineState>()((set, get) => ({
  status: EMPTY,
  error: null,

  init: () => {
    if (initOnce) return;
    initOnce = true;
    listen<EngineStatus>("engine-status", (e) => {
      set({ status: e.payload });
      // движок мог поменять устройства по умолчанию: ноды «Система» и «по умолчанию» показывают их
      useDevices.getState().refresh();
    });
    invoke<EngineStatus>("engine_status").then((status) => set({ status }));
  },

  start: async (graphId) => {
    // движок читает граф с диска: сначала дописываем несохранённые правки
    await flushSave();
    // кнопка отзывается сразу, подробности приходят со статусом
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
