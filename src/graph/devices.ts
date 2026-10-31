import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { t } from "../i18n";
import type { DeviceRef } from "./types";

/** Совпадает с AudioDevice в src-tauri/src/devices/mod.rs */
export type AudioDevice = {
  id: string;
  name: string;
  flow: "render" | "capture";
  /** disabled — выключено в настройках звука, его можно включить */
  state: "active" | "disabled";
  isDefault: boolean;
};

/**
 * Общий буфер списка устройств системы. Все выпадающие списки читают его,
 * а при открытии просят обновить: старый список виден сразу, новый подменяет его по готовности.
 */
type DevicesState = {
  devices: AudioDevice[];
  /** Список хотя бы раз получен; до этого нельзя судить, что устройство пропало */
  loaded: boolean;
  loading: boolean;
  error: string | null;
  /** Устройства, которые сейчас включаются или выключаются */
  pending: string[];
  /** Ошибка последнего включения или выключения */
  toggleError: string | null;
  refresh: () => Promise<void>;
  setEnabled: (ids: string[], enabled: boolean) => Promise<void>;
};

// одновременные запросы обновления склеиваются в один
let inflight: Promise<void> | null = null;

// см. store.ts: при правке модуля в разработке — полная перезагрузка страницы
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
        // если ничего не поменялось, не трогаем массив, чтобы не перерисовывать ноды
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
    // переключатель отзывается сразу, а настоящее состояние приходит со следующим списком
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
      // список, запрошенный до переключения, мог прийти со старыми состояниями
      if (inflight) await inflight;
      await get().refresh();
    }
  },
}));

/**
 * Подпись выбранного устройства по буферу списка.
 * missing — устройство выбрано, но его нет в системе (отсоединено или удалено);
 * disabled — есть, но выключено.
 */
export function describeDevice(
  ref: DeviceRef | null,
  flow: AudioDevice["flow"],
  devices: AudioDevice[],
  loaded: boolean,
): { text: string | null; missing: boolean; disabled: boolean } {
  if (!ref) return { text: null, missing: false, disabled: false };
  if (ref.type === "default") return { text: t("По умолчанию"), missing: false, disabled: false };
  const device = devices.find((d) => d.flow === flow && d.id === ref.id);
  // пока устройство есть в системе, показываем его актуальное имя
  return { text: device?.name ?? ref.name, missing: loaded && !device, disabled: device?.state === "disabled" };
}
