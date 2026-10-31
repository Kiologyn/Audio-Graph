import {
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  reconnectEdge,
  type Connection,
  type EdgeChange,
  type NodeChange,
  type Viewport,
  type XYPosition,
} from "@xyflow/react";
import { create } from "zustand";
import { isAnyTranslation, t } from "../i18n";
import { edgesFromDoc, nodesFromDoc, type GraphDoc } from "./doc";
import {
  NODE_KINDS,
  type AudioEdge,
  type AudioNode,
  type AudioNodeKind,
  type AppRef,
  type DeviceRef,
  type DeviceSlotName,
} from "./types";

/** Открытый на холсте граф */
type GraphState = {
  /** null, пока граф не загружен */
  meta: { id: string; name: string; createdAt: number; showVolume: boolean } | null;
  viewport: Viewport | null;
  nodes: AudioNode[];
  edges: AudioEdge[];
  loadDoc: (doc: GraphDoc) => void;
  unload: () => void;
  setName: (name: string) => void;
  setShowVolume: (show: boolean) => void;
  setViewport: (viewport: Viewport) => void;
  onNodesChange: (changes: NodeChange<AudioNode>[]) => void;
  onEdgesChange: (changes: EdgeChange<AudioEdge>[]) => void;
  onConnect: (connection: Connection) => void;
  onReconnect: (oldEdge: AudioEdge, connection: Connection) => void;
  removeEdge: (id: string) => void;
  addNode: (kind: AudioNodeKind, position: XYPosition) => void;
  renameNode: (id: string, label: string) => void;
  setDevice: (id: string, slot: DeviceSlotName, device: DeviceRef | null) => void;
  setCable: (id: string, cable: string | null) => void;
  setApp: (id: string, app: AppRef | null) => void;
  setVolume: (id: string, volume: number) => void;
  undo: () => void;
  redo: () => void;
};

/** Состояние графа до правки, для отмены */
type Snapshot = { nodes: AudioNode[]; edges: AudioEdge[] };

const HISTORY_LIMIT = 100;

// История живёт вне состояния: её изменения не должны перерисовывать холст.
// Хранилище меняет ноды и связи только заменой массивов, поэтому снимок — это просто ссылки
let past: Snapshot[] = [];
let future: Snapshot[] = [];
// последняя запись: серия однотипных правок подряд (ползунок, удаление ноды вместе со связями)
// отменяется одним шагом
let lastKey: string | null = null;
let lastAt = 0;
let dragging = false;

function resetHistory() {
  past = [];
  future = [];
  lastKey = null;
  dragging = false;
}

function makeNode(kind: AudioNodeKind, position: XYPosition, label?: string): AudioNode {
  return {
    id: crypto.randomUUID(),
    type: "audio",
    position,
    data: { kind, label: label ?? t(NODE_KINDS[kind].defaultLabel), sink: null, source: null, cable: null, app: null, volume: 1 },
  };
}

function makeEdge(source: AudioNode, target: AudioNode): AudioEdge {
  return {
    id: `${source.id}->${target.id}`,
    source: source.id,
    sourceHandle: "out",
    target: target.id,
    targetHandle: "in",
  };
}

/**
 * Графы первого запуска. Кабели и устройства в них не выбраны: на каждом ПК они свои
 */
export function starterGraphs(): { name: string; nodes: AudioNode[]; edges: AudioEdge[] }[] {
  // всё, что играет по умолчанию, — в кабель, а из него в два устройства вывода
  const system = makeNode("system", { x: 0, y: 100 });
  const cable = makeNode("cable", { x: 320, y: 100 });
  const out1 = makeNode("output", { x: 640, y: 0 }, t("Наушники"));
  const out2 = makeNode("output", { x: 640, y: 200 }, t("Колонки"));

  // звук приложения через кабель в наушники, и он же вместе с микрофоном — в кабель для звонков
  const app = makeNode("app", { x: 0, y: 0 });
  const appCable = makeNode("cable", { x: 370, y: 58 });
  const mic = makeNode("input", { x: 82, y: 330 }, t("Микрофон"));
  const phones = makeNode("output", { x: 928, y: 60 }, t("Наушники"));
  const mixed = makeNode("cable", { x: 697, y: 284 }, t("Микрофон с браузером"));
  mic.data.source = { type: "default" };
  phones.data.sink = { type: "default" };

  return [
    {
      name: t("Система в кабель"),
      nodes: [system, cable, out1, out2],
      edges: [makeEdge(system, cable), makeEdge(cable, out1), makeEdge(cable, out2)],
    },
    {
      name: t("Микрофон с приложением"),
      nodes: [app, appCable, mic, phones, mixed],
      edges: [makeEdge(app, appCable), makeEdge(appCable, phones), makeEdge(appCable, mixed), makeEdge(mic, mixed)],
    },
  ];
}

// В разработке правка хранилища через HMR пересоздала бы его пустым без повторной загрузки,
// поэтому для модулей-хранилищ Vite перезагружает страницу целиком
import.meta.hot?.accept(() => location.reload());

export const useGraph = create<GraphState>()((set, get) => {
  /**
   * Запомнить граф перед правкой. key склеивает серию правок в один шаг отмены,
   * если между ними прошло меньше window мс
   */
  const record = (key: string | null = null, window = 1000) => {
    const now = Date.now();
    const series = key !== null && key === lastKey && now - lastAt < window;
    lastKey = key;
    lastAt = now;
    if (series) return;
    const { nodes, edges } = get();
    past.push({ nodes, edges });
    if (past.length > HISTORY_LIMIT) past.shift();
    future = [];
  };

  // отменённое состояние возвращается без выделения: иначе Delete сразу после отмены
  // удалил бы то, что было выделено когда-то давно
  const restore = (snap: Snapshot) => {
    lastKey = null;
    dragging = false;
    set({
      nodes: snap.nodes.map((n) => (n.selected ? { ...n, selected: false } : n)),
      edges: snap.edges.map((e) => (e.selected ? { ...e, selected: false } : e)),
    });
  };

  return {
    meta: null,
    viewport: null,
    nodes: [],
    edges: [],

    loadDoc: (doc) => {
      resetHistory();
      set({
        meta: { id: doc.id, name: doc.name, createdAt: doc.createdAt, showVolume: doc.showVolume ?? true },
        viewport: doc.viewport,
        nodes: nodesFromDoc(doc.nodes),
        edges: edgesFromDoc(doc.edges),
      });
    },
    unload: () => {
      resetHistory();
      set({ meta: null, viewport: null, nodes: [], edges: [] });
    },

    setName: (name) => {
      const meta = get().meta;
      if (meta) set({ meta: { ...meta, name } });
    },
    setShowVolume: (showVolume) => {
      const meta = get().meta;
      if (meta) set({ meta: { ...meta, showVolume } });
    },
    setViewport: (viewport) => set({ viewport }),

    onNodesChange: (changes) => {
      // выделение и размеры — не правки графа; перетаскивание — один шаг от начала до конца
      if (changes.some((c) => c.type === "remove" || c.type === "add" || c.type === "replace")) record("delete", 50);
      for (const c of changes) {
        if (c.type !== "position") continue;
        if (c.dragging) {
          if (!dragging) record();
          dragging = true;
        } else if (dragging) dragging = false;
        // сдвиг стрелками клавиатуры
        else if (c.position) record("nudge");
      }
      set({ nodes: applyNodeChanges(changes, get().nodes) });
    },
    onEdgesChange: (changes) => {
      // связи ноды удаляются отдельным вызовом сразу после неё самой — это тот же шаг
      if (changes.some((c) => c.type === "remove" || c.type === "add" || c.type === "replace")) record("delete", 50);
      set({ edges: applyEdgeChanges(changes, get().edges) });
    },
    onConnect: (connection) => {
      const edges = addEdge(connection, get().edges);
      if (edges.length === get().edges.length) return;
      record();
      set({ edges });
    },
    // холст отдаёт связь в том виде, в каком показывал её, с красной пометкой проблемы;
    // новая связь создаётся копией старой, и пометка в хранилище не нужна
    onReconnect: (oldEdge, connection) => {
      const { className: _, ...edge } = oldEdge;
      record();
      set({ edges: reconnectEdge(edge, connection, get().edges) });
    },
    removeEdge: (id) => {
      record("delete", 50);
      set({ edges: get().edges.filter((e) => e.id !== id) });
    },

    addNode: (kind, position) => {
      record();
      set({ nodes: [...get().nodes, makeNode(kind, position)] });
    },

    renameNode: (id, label) => {
      if (get().nodes.find((n) => n.id === id)?.data.label === label) return;
      record();
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, label } } : n)),
      });
    },

    setDevice: (id, slot, device) => {
      record();
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, [slot]: device } } : n)),
      });
    },

    setCable: (id, cable) => {
      record();
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, cable } } : n)),
      });
    },

    setApp: (id, app) => {
      record();
      set({
        nodes: get().nodes.map((n) => {
          if (n.id !== id) return n;
          // подпись, которую не меняли вручную (на любом из языков), следует за выбранным приложением
          const auto = isAnyTranslation(n.data.label, NODE_KINDS.app.defaultLabel) || n.data.label === n.data.app?.name;
          const label = auto ? (app?.name ?? t(NODE_KINDS.app.defaultLabel)) : n.data.label;
          return { ...n, data: { ...n.data, app, label } };
        }),
      });
    },

    setVolume: (id, volume) => {
      record(`volume:${id}`);
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, volume } } : n)),
      });
    },

    undo: () => {
      const snap = past.pop();
      if (!snap) return;
      const { nodes, edges } = get();
      future.push({ nodes, edges });
      restore(snap);
    },
    redo: () => {
      const snap = future.pop();
      if (!snap) return;
      const { nodes, edges } = get();
      past.push({ nodes, edges });
      restore(snap);
    },
  };
});
