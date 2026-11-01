import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { t } from "../i18n";
import type { DeviceRef } from "./types";

/** Matches AudioDevice in src-tauri/src/devices/mod.rs */
export type AudioDevice = {
  id: string;
  name: string;
  flow: "render" | "capture";
  /** disabled: turned off in the sound settings, it can be turned on */
  state: "active" | "disabled";
  isDefault: boolean;
  /** A cable device the graph created itself (Linux): nodes don't choose it */
  isVirtual: boolean;
  /** The other side of the virtual cable this device belongs to, found by driver properties (Windows) */
  pair: string | null;
  /** The name of that cable: "Line 1" */
  pairName: string | null;
};

/**
 * Shared buffer of the system's device list. All dropdowns read it
 * and ask to refresh it when opened: the old list shows at once, the new one replaces it when ready.
 */
type DevicesState = {
  devices: AudioDevice[];
  /** The list was received at least once; before that a device can't be judged gone */
  loaded: boolean;
  loading: boolean;
  error: string | null;
  /** Devices being turned on or off right now */
  pending: string[];
  /** The error of the last turning on or off */
  toggleError: string | null;
  refresh: () => Promise<void>;
  setEnabled: (ids: string[], enabled: boolean) => Promise<void>;
};

// concurrent refresh requests are merged into one
let inflight: Promise<void> | null = null;

// see store.ts: editing this module in development reloads the whole page
import.meta.hot?.accept(() => location.reload());

export const useDevices = create<DevicesState>()((set, get) => ({
  devices: [],
  loaded: false,
  loading: false,
  error: null,
  pending: [],
  toggleError: null,

  refresh: () =>
    (inflight ??= (async () => {
      set({ loading: true });
      try {
        const devices = await invoke<AudioDevice[]>("audio_devices");
        // if nothing changed, keep the array so nodes don't re-render
        const same = JSON.stringify(devices) === JSON.stringify(get().devices);
        set({ ...(same ? {} : { devices }), loaded: true, error: null });
      } catch (e) {
        set({ error: String(e) });
      } finally {
        set({ loading: false });
        inflight = null;
      }
    })()),

  setEnabled: async (ids, enabled) => {
    if (ids.length === 0) return;
    const state = enabled ? "active" : "disabled";
    // the switch responds at once, the real state comes with the next list
    set((s) => ({
      pending: [...s.pending, ...ids],
      devices: s.devices.map((d) => (ids.includes(d.id) ? { ...d, state } : d)),
    }));
    try {
      await invoke("set_devices_enabled", { ids, enabled });
      set({ toggleError: null });
    } catch (e) {
      set({ toggleError: String(e) });
    } finally {
      set((s) => ({ pending: s.pending.filter((id) => !ids.includes(id)) }));
      // a list requested before the switch may arrive with old states
      if (inflight) await inflight;
      await get().refresh();
    }
  },
}));

/**
 * The label of the chosen device from the list buffer.
 * missing: the device is chosen but the system doesn't have it (unplugged or removed);
 * disabled: present but turned off.
 */
export function describeDevice(
  ref: DeviceRef | null,
  flow: AudioDevice["flow"],
  devices: AudioDevice[],
  loaded: boolean,
): { text: string | null; missing: boolean; disabled: boolean } {
  if (!ref) return { text: null, missing: false, disabled: false };
  if (ref.type === "default") return { text: t("device_default"), missing: false, disabled: false };
  const device = devices.find((d) => d.flow === flow && d.id === ref.id);
  // while the device exists in the system, show its current name
  return { text: device?.name ?? ref.name, missing: loaded && !device, disabled: device?.state === "disabled" };
}
