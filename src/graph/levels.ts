import { invoke } from "@tauri-apps/api/core";

/** How often meters are refreshed: a meter needs no more, and the engine only stores the loudest value meanwhile */
export const LEVEL_TICK_MS = 40;

type Listener = (peak: number) => void;

const listeners = new Map<string, Set<Listener>>();
let timer: ReturnType<typeof setTimeout> | null = null;

/** One request for all meters, and none at all with no meter on screen or the window hidden */
async function poll() {
  if (!document.hidden) {
    const levels = await invoke<Record<string, number>>("engine_levels").catch(() => ({}) as Record<string, number>);
    for (const [node, each] of listeners) {
      for (const listener of each) listener(levels[node] ?? 0);
    }
  }
  timer = listeners.size > 0 ? setTimeout(poll, LEVEL_TICK_MS) : null;
}

/** Hands a node's loudest sample value (0 to 1 and beyond) to the listener on every tick; returns how to stop */
export function watchLevel(node: string, listener: Listener): () => void {
  const each = listeners.get(node) ?? new Set();
  listeners.set(node, each.add(listener));
  if (timer === null) timer = setTimeout(poll, LEVEL_TICK_MS);
  return () => {
    each.delete(listener);
    if (each.size === 0) listeners.delete(node);
  };
}

/** A sample value in dB, within what a noise gate's slider shows */
export function toDb(peak: number, floor: number): number {
  return peak > 0 ? Math.min(0, Math.max(floor, 20 * Math.log10(peak))) : floor;
}

/** How fast a meter falls after a peak, in dB per tick: it rises at once, or short sounds would never show */
const FALL_DB = 1.6;

/** The hold of the engine's gate (GATE_HOLD in mixer.rs), so the knob shows what the gate does */
const HOLD_MS = 250;

export type Meter = {
  /** The level to show, in dB */
  db: number;
  /** Until when the gate stays open */
  openUntil: number;
};

/** A meter's next state after a tick; the gate opens at the threshold, like the engine's */
export function nextMeter(meter: Meter, peakDb: number, threshold: number, now: number, floor: number): Meter {
  return {
    db: Math.max(peakDb, meter.db - FALL_DB, floor),
    openUntil: peakDb >= threshold ? now + HOLD_MS : meter.openUntil,
  };
}
