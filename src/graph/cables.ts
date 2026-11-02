import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { findDevice, type AudioDevice } from "./devices";
import { t } from "../i18n";
import { uniqueName } from "./names";
import { inOrder } from "./sortable";
import type { DeviceRef } from "./types";

/** A cable is a pair of system devices shared by all graphs. Matches Cable in cables.rs. */
export type Cable = {
  id: string;
  name: string;
  /** The output device: apps play into it, it is the cable's input */
  render: DeviceRef | null;
  /** The input device: sound is taken from it, it is the cable's output */
  capture: DeviceRef | null;
  createdAt: number;
};

type CablesState = {
  cables: Cable[];
  /** The list was received at least once; before that a cable can't be judged removed */
  loaded: boolean;
  error: string | null;
  load: () => Promise<void>;
  /** A new cable: by default named "Cable" without devices */
  create: (name?: string, sides?: Pick<Cable, "render" | "capture">) => Promise<string>;
  update: (id: string, patch: Partial<Pick<Cable, "name" | "render" | "capture">>) => void;
  remove: (id: string) => void;
  /** Puts the list in this order of cable ids */
  reorder: (ids: string[]) => void;
};

// Changes apply in the interface at once and go to disk strictly in order:
// every request carries the cable's full state, so the last one written is the latest
let queue: Promise<void> = Promise.resolve();
function enqueue(job: () => Promise<unknown>): Promise<void> {
  queue = queue.then(job).then(
    () => useCables.setState({ error: null }),
    (e) => useCables.setState({ error: String(e) }),
  );
  return queue;
}

// see store.ts: editing this module in development reloads the whole page
import.meta.hot?.accept(() => location.reload());

export const useCables = create<CablesState>()((set, get) => ({
  cables: [],
  loaded: false,
  error: null,

  load: () =>
    enqueue(async () => {
      const cables = await invoke<Cable[]>("cables_list");
      set({ cables, loaded: true });
    }),

  create: async (base = t("node_cable"), sides) => {
    const name = uniqueName(base, get().cables.map((c) => c.name));
    let id = "";
    await enqueue(async () => {
      const cable = await invoke<Cable>("cable_create", { name });
      set((s) => ({ cables: [...s.cables, cable] }));
      id = cable.id;
    });
    if (id && sides) get().update(id, sides);
    return id;
  },

  update: (id, patch) => {
    const cable = get().cables.find((c) => c.id === id);
    if (!cable) return;
    const next = { ...cable, ...patch };
    set((s) => ({ cables: s.cables.map((c) => (c.id === id ? next : c)) }));
    enqueue(() => invoke("cable_update", { cable: next }));
  },

  remove: (id) => {
    set((s) => ({ cables: s.cables.filter((c) => c.id !== id) }));
    enqueue(() => invoke("cable_delete", { id }));
  },

  reorder: (ids) => {
    set({ cables: inOrder(get().cables, ids) });
    enqueue(() => invoke("cables_reorder", { ids }));
  },
}));

/** The cable panel in the header: the cable choice on a node opens it too ("New cable…") */
type CablePanelState = {
  open: boolean;
  /** The cable expanded for editing */
  editing: string | null;
  show: (editing?: string | null) => void;
  hide: () => void;
  edit: (id: string | null) => void;
};

export const useCablePanel = create<CablePanelState>()((set) => ({
  open: false,
  editing: null,
  show: (editing = null) => set({ open: true, editing }),
  hide: () => set({ open: false, editing: null }),
  edit: (editing) => set({ editing }),
}));

/**
 * The cable's system devices. With the device list, as the engine finds them now: a device back
 * under a new id is found by its name (findDevice)
 */
export function cableDeviceIds(cable: Cable, devices?: AudioDevice[]): string[] {
  const sides = [
    [cable.render, "render"],
    [cable.capture, "capture"],
  ] as const;
  return sides.flatMap(([d, flow]) =>
    d?.type === "device" ? [(devices && findDevice(d, flow, devices)?.id) || d.id] : [],
  );
}

/** Device → name of the cable it belongs to */
export function cableDeviceOwners(cables: Cable[]): Map<string, string> {
  return new Map(cables.flatMap((c) => cableDeviceIds(c).map((id) => [id, c.name] as const)));
}

/**
 * Virtual cables found by driver properties that aren't in the list yet: an output device
 * and the recording device its driver pairs with it. A cable with exactly these two devices counts as present.
 */
export function newFoundCables(cables: Cable[], devices: AudioDevice[]): { name: string; sides: Pick<Cable, "render" | "capture"> }[] {
  const present = new Set(cables.map((c) => `${c.render?.type === "device" ? c.render.id : ""}|${c.capture?.type === "device" ? c.capture.id : ""}`));
  return devices.flatMap((render) => {
    const capture = render.flow === "render" && devices.find((d) => d.flow === "capture" && d.id === render.pair);
    if (!capture || present.has(`${render.id}|${capture.id}`)) return [];
    return [
      {
        name: render.pairName ?? render.name,
        sides: {
          render: { type: "device", id: render.id, name: render.name },
          capture: { type: "device", id: capture.id, name: capture.name },
        },
      },
    ];
  });
}

/**
 * The cable's sides are from different virtual cables: its chosen device is paired by the driver
 * with another one. An empty side isn't a mismatch. Returns the warning: a string code and its values.
 */
export function cableMismatch(cable: Cable, devices: AudioDevice[]): { code: string; vars: Record<string, string> } | null {
  const [renderId, captureId] = [cable.render, cable.capture].map((d) => (d?.type === "device" ? d.id : null));
  if (!renderId || !captureId) return null;
  const render = devices.find((d) => d.flow === "render" && d.id === renderId);
  const capture = devices.find((d) => d.flow === "capture" && d.id === captureId);
  for (const [device, other, chosen, code] of [
    [render, "capture", captureId, "cable_output_mismatch"],
    [capture, "render", renderId, "cable_input_mismatch"],
  ] as const) {
    if (!device?.pair || device.pair === chosen) continue;
    const pair = devices.find((d) => d.flow === other && d.id === device.pair);
    if (pair) return { code, vars: { device: device.name, pair: pair.name } };
  }
  return null;
}

/** The cable is on, off or partly on; none means the system has none of its devices, nothing to switch */
export type CablePower = "on" | "off" | "mixed" | "none";

export function cablePower(cable: Cable, devices: AudioDevice[]): CablePower {
  const ids = new Set(cableDeviceIds(cable, devices));
  const states = devices.filter((d) => ids.has(d.id)).map((d) => d.state);
  if (states.length === 0) return "none";
  if (states.every((s) => s === "active")) return "on";
  if (states.every((s) => s === "disabled")) return "off";
  return "mixed";
}

/** Cable devices to switch so that all of them become on (or off) */
export function idsToToggle(cables: Cable[], devices: AudioDevice[], enabled: boolean): string[] {
  const ids = new Set(cables.flatMap((c) => cableDeviceIds(c, devices)));
  const from = enabled ? "disabled" : "active";
  return devices.filter((d) => ids.has(d.id) && d.state === from).map((d) => d.id);
}
