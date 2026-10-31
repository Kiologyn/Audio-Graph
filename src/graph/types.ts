import type { Edge, Node } from "@xyflow/react";

/**
 * Виды нод графа.
 * - input  — устройство ввода (микрофон, запись кабеля): только выход
 * - output — устройство вывода (наушники, колонки): только вход
 * - app    — приложение: выход = его звук, вход = его микрофон
 * - cable  — кабель: внутренний микшер или кабель из общего списка (пара системных устройств)
 * - system — устройства по умолчанию в Windows: выход = звук приложений, играющих в устройство
 *            вывода по умолчанию, вход = то, что они слышат как микрофон по умолчанию
 */
export type AudioNodeKind = "input" | "output" | "app" | "cable" | "system";

/** Привязка порта к системному устройству. Совпадает с DeviceRef в graphs.rs. */
export type DeviceRef =
  | { type: "default" }
  // имя запоминается, чтобы показывать его, пока устройство отключено
  | { type: "device"; id: string; name: string };

/** Приложение ноды приложения. Совпадает с AppRef в graphs.rs. */
export type AppRef = {
  /** имя исполняемого файла: по нему приложение узнаётся, даже если сменилась папка */
  exe: string;
  /** полный путь, под которым приложение было выбрано */
  path: string;
  /** название, чтобы показывать его, пока приложение не запущено */
  name: string;
};

/** sink — устройство вывода за входным портом, source — устройство записи за выходным */
export type DeviceSlotName = "sink" | "source";

export type AudioNodeData = {
  kind: AudioNodeKind;
  label: string;
  sink: DeviceRef | null;
  source: DeviceRef | null;
  /** id кабеля из общего списка кабелей (cables.ts); null — внутренний кабель */
  cable: string | null;
  app: AppRef | null;
  /** Громкость: 1 — без изменений, 0 — тишина, больше 1 — усиление */
  volume: number;
};

export type AudioNode = Node<AudioNodeData, "audio">;
export type AudioEdge = Edge;

export type DeviceSlot = {
  /** Текст, пока устройство не выбрано */
  placeholder: string;
  /** Можно ли выбрать «по умолчанию» (следовать за системным устройством) */
  allowDefault: boolean;
};

/** Тексты — ключи перевода (i18n.ts) */
type KindMeta = {
  title: string;
  defaultLabel: string;
  hasInput: boolean;
  hasOutput: boolean;
  sink?: DeviceSlot;
  source?: DeviceSlot;
  /** Нода выбирает кабель из общего списка кабелей */
  pickCable?: boolean;
  /** Нода выбирает запущенное приложение */
  pickApp?: boolean;
  /** Нода показывает текущие устройства по умолчанию */
  showDefaults?: boolean;
  /**
   * Наибольшая громкость. Усиление больше 1 делает только движок; у приложения громкость —
   * это громкость в микшере Windows, она не выше 100%. Без поля громкости у ноды нет
   */
  maxVolume?: number;
};

export const NODE_KINDS: Record<AudioNodeKind, KindMeta> = {
  input: {
    title: "Ввод",
    defaultLabel: "Микрофон",
    hasInput: false,
    hasOutput: true,
    source: { placeholder: "Устройство не выбрано", allowDefault: true },
    maxVolume: 2,
  },
  output: {
    title: "Вывод",
    defaultLabel: "Наушники",
    hasInput: true,
    hasOutput: false,
    sink: { placeholder: "Устройство не выбрано", allowDefault: true },
    maxVolume: 2,
  },
  app: {
    title: "Приложение",
    defaultLabel: "Приложение",
    hasInput: true,
    hasOutput: true,
    pickApp: true,
    maxVolume: 1,
  },
  cable: {
    title: "Кабель",
    defaultLabel: "Кабель",
    hasInput: true,
    hasOutput: true,
    // пара устройств задаётся в списке кабелей, нода только ссылается на кабель
    pickCable: true,
    maxVolume: 2,
  },
  system: {
    title: "Система",
    defaultLabel: "Устройства по умолчанию",
    hasInput: true,
    hasOutput: true,
    showDefaults: true,
  },
};

export const KIND_ORDER: AudioNodeKind[] = ["input", "app", "system", "cable", "output"];
