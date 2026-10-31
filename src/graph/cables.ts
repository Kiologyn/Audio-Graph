import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { AudioDevice } from "./devices";
import { t } from "../i18n";
import { uniqueName } from "./names";
import type { DeviceRef } from "./types";

/** Кабель — пара системных устройств, общая для всех графов. Совпадает с Cable в cables.rs. */
export type Cable = {
  id: string;
  name: string;
  /** Устройство вывода: в него играют приложения, это вход кабеля */
  render: DeviceRef | null;
  /** Устройство ввода: с него забирают звук, это выход кабеля */
  capture: DeviceRef | null;
  createdAt: number;
};

type CablesState = {
  cables: Cable[];
  /** Список хотя бы раз получен; до этого нельзя судить, что кабель удалён */
  loaded: boolean;
  error: string | null;
  load: () => Promise<void>;
  create: () => Promise<string>;
  update: (id: string, patch: Partial<Pick<Cable, "name" | "render" | "capture">>) => void;
  remove: (id: string) => void;
};

// Изменения применяются в интерфейсе сразу, а на диск уходят строго по очереди:
// каждый запрос несёт полное состояние кабеля, поэтому последний записанный — самый свежий
let queue: Promise<void> = Promise.resolve();
function enqueue(job: () => Promise<unknown>): Promise<void> {
  queue = queue.then(job).then(
    () => useCables.setState({ error: null }),
    (e) => useCables.setState({ error: String(e) }),
  );
  return queue;
}

// см. store.ts: при правке модуля в разработке — полная перезагрузка страницы
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

  create: async () => {
    const name = uniqueName(t("Кабель"), get().cables.map((c) => c.name));
    let id = "";
    await enqueue(async () => {
      const cable = await invoke<Cable>("cable_create", { name });
      set((s) => ({ cables: [...s.cables, cable] }));
      id = cable.id;
    });
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
}));

/** Панель кабелей в шапке: её открывает и выбор кабеля на ноде («Новый кабель…») */
type CablePanelState = {
  open: boolean;
  /** Кабель, раскрытый для редактирования */
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

/** Системные устройства кабеля */
export function cableDeviceIds(cable: Cable): string[] {
  return [cable.render, cable.capture].flatMap((d) => (d?.type === "device" ? [d.id] : []));
}

/** Устройство → имя кабеля, в который оно входит */
export function cableDeviceOwners(cables: Cable[]): Map<string, string> {
  return new Map(cables.flatMap((c) => cableDeviceIds(c).map((id) => [id, c.name] as const)));
}

/** Кабель включён, выключен или частично; none — в системе нет его устройств, переключать нечего */
export type CablePower = "on" | "off" | "mixed" | "none";

export function cablePower(cable: Cable, devices: AudioDevice[]): CablePower {
  const ids = new Set(cableDeviceIds(cable));
  const states = devices.filter((d) => ids.has(d.id)).map((d) => d.state);
  if (states.length === 0) return "none";
  if (states.every((s) => s === "active")) return "on";
  if (states.every((s) => s === "disabled")) return "off";
  return "mixed";
}

/** Устройства кабелей, которые нужно переключить, чтобы все они стали включены (или выключены) */
export function idsToToggle(cables: Cable[], devices: AudioDevice[], enabled: boolean): string[] {
  const ids = new Set(cables.flatMap(cableDeviceIds));
  const from = enabled ? "disabled" : "active";
  return devices.filter((d) => ids.has(d.id) && d.state === from).map((d) => d.id);
}
