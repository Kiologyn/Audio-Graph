import { invoke } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { create } from "zustand";
import { useEngine } from "./engine";
import { useGraph } from "./store";
import { NODE_KINDS, type AudioNodeData } from "./types";

/** Matches Target in system_volume.rs; a device without an id is the system's default one */
export type VolumeTarget = { type: "device"; flow: "render" | "capture"; id: string | null } | { type: "app"; exe: string };

/** How often the system's sliders are read */
const POLL_MS = 500;
/** A slider just moved here ignores what the system said a moment before the move */
const HOLD_MS = 1500;

/** The system slider behind a node; null if it has none or nothing is chosen yet */
export function volumeTarget(data: AudioNodeData): VolumeTarget | null {
  if (!NODE_KINDS[data.kind].systemVolume) return null;
  if (data.kind === "app") return data.app ? { type: "app", exe: data.app.exe.toLowerCase() } : null;
  const flow = data.kind === "input" ? "capture" : "render";
  const device = data.kind === "input" ? data.source : data.sink;
  if (!device) return null;
  return { type: "device", flow, id: device.type === "device" ? device.id : null };
}

/**
 * The system's slider left the graph's value by itself. A graph just started hasn't reached its value
 * yet: until the slider stood at it, what it reads isn't a change to follow
 */
export function movedAwayFrom(set: number | null, before: number | null, now: number | null): boolean {
  const same = (a: number, b: number) => Math.abs(a - b) < 0.005;
  return set !== null && before !== null && now !== null && same(before, set) && !same(now, set);
}

type SystemVolumes = {
  /** Node id → the system slider's position; null if there is nothing to read */
  volumes: Record<string, number | null>;
  held: Record<string, number>;
  /** A slider moved here: shows the value at once */
  moved: (id: string, volume: number) => void;
  received: (ids: string[], values: (number | null)[]) => void;
};

export const useSystemVolumes = create<SystemVolumes>()((set, get) => ({
  volumes: {},
  held: {},
  moved: (id, volume) =>
    set((s) => ({ volumes: { ...s.volumes, [id]: volume }, held: { ...s.held, [id]: Date.now() + HOLD_MS } })),
  received: (ids, values) => {
    const { volumes, held } = get();
    const now = Date.now();
    const next = Object.fromEntries(ids.map((id, i) => [id, (held[id] ?? 0) > now ? (volumes[id] ?? null) : values[i]]));
    const same = ids.length === Object.keys(volumes).length && ids.every((id) => volumes[id] === next[id]);
    if (!same) set({ volumes: next });
  },
}));

/** Keeps the shown sliders up to date with the system. With no volume slider on the canvas it reads nothing */
export function useSystemVolumePolling() {
  useEffect(() => {
    let alive = true;
    let timer: number | undefined;
    const tick = async () => {
      const nodes = useGraph.getState().nodes;
      const shown = nodes.some((n) => n.data.pinned.includes("volume"));
      const found = nodes.flatMap((n) => {
        const target = volumeTarget(n.data);
        return target ? [{ id: n.id, target }] : [];
      });
      if (shown && found.length > 0 && !document.hidden) {
        try {
          const values = await invoke<(number | null)[]>("system_volumes", { targets: found.map((f) => f.target) });
          if (alive) {
            const { volumes, held, received } = useSystemVolumes.getState();
            const { meta, nodes: now, followSystemVolume } = useGraph.getState();
            const running = !!meta && useEngine.getState().status.running.includes(meta.id);
            found.forEach(({ id }, i) => {
              const set = now.find((n) => n.id === id)?.data.systemVolume ?? null;
              if (running && (held[id] ?? 0) <= Date.now() && movedAwayFrom(set, volumes[id] ?? null, values[i])) followSystemVolume(id, values[i]!);
            });
            received(found.map((f) => f.id), values);
          }
        } catch {
          // the next round tries again
        }
      }
      if (alive) timer = window.setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, []);
}
