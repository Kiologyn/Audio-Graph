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
import { edgesFromDoc, nodesFromDoc, type GraphDoc, type GraphPrefs } from "./doc";
import {
  NODE_KINDS,
  type AudioEdge,
  type AudioNode,
  type AudioNodeKind,
  type AppRef,
  type DeviceRef,
  type DeviceSlotName,
} from "./types";

/** The graph open on the canvas */
type GraphState = {
  /** null until the graph is loaded */
  meta: ({ id: string; name: string; createdAt: number } & GraphPrefs) | null;
  viewport: Viewport | null;
  nodes: AudioNode[];
  edges: AudioEdge[];
  loadDoc: (doc: GraphDoc) => void;
  unload: () => void;
  setName: (name: string) => void;
  setPrefs: (patch: Partial<GraphPrefs>) => void;
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
  setDelay: (id: string, delay: number) => void;
  /** Delays of several outputs as one undo step (calibration) */
  setDelays: (delays: Record<string, number>) => void;
  setBalance: (id: string, balance: number) => void;
  undo: () => void;
  redo: () => void;
};

/** The graph state before an edit, for undo */
type Snapshot = { nodes: AudioNode[]; edges: AudioEdge[] };

const HISTORY_LIMIT = 100;

// History lives outside the state: its changes must not re-render the canvas.
// The store changes nodes and edges only by replacing arrays, so a snapshot is just references
let past: Snapshot[] = [];
let future: Snapshot[] = [];
// the last entry: a series of similar edits in a row (a slider, deleting a node with its edges)
// is undone in one step
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
    data: { kind, label: label ?? t(NODE_KINDS[kind].defaultLabel), sink: null, source: null, cable: null, app: null, volume: 1, delay: 0, balance: 0 },
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
 * First-start graphs. Cables and devices in them aren't chosen: every PC has its own
 */
export function starterGraphs(): { name: string; nodes: AudioNode[]; edges: AudioEdge[] }[] {
  // everything playing to the default goes into a cable, and from it to two output devices
  const system = makeNode("system", { x: 0, y: 100 });
  const cable = makeNode("cable", { x: 320, y: 100 });
  const out1 = makeNode("output", { x: 640, y: 0 }, t("label_headphones"));
  const out2 = makeNode("output", { x: 640, y: 200 }, t("label_speakers"));

  // an app's sound goes through a cable to the headphones and, with the microphone, into a cable for calls
  const app = makeNode("app", { x: 0, y: 0 });
  const appCable = makeNode("cable", { x: 370, y: 58 });
  const mic = makeNode("input", { x: 82, y: 330 }, t("label_microphone"));
  const phones = makeNode("output", { x: 928, y: 60 }, t("label_headphones"));
  const mixed = makeNode("cable", { x: 697, y: 284 }, t("example_microphone_with_browser"));
  mic.data.source = { type: "default" };
  phones.data.sink = { type: "default" };

  return [
    {
      name: t("example_system_to_cable"),
      nodes: [system, cable, out1, out2],
      edges: [makeEdge(system, cable), makeEdge(cable, out1), makeEdge(cable, out2)],
    },
    {
      name: t("example_microphone_with_app"),
      nodes: [app, appCable, mic, phones, mixed],
      edges: [makeEdge(app, appCable), makeEdge(appCable, phones), makeEdge(appCable, mixed), makeEdge(mic, mixed)],
    },
  ];
}

// In development an HMR edit of a store would recreate it empty without reloading,
// so for store modules Vite reloads the whole page
import.meta.hot?.accept(() => location.reload());

export const useGraph = create<GraphState>()((set, get) => {
  /**
   * Remember the graph before an edit. key merges a series of edits into one undo step
   * if less than window ms passed between them
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

  // an undone state comes back without selection: otherwise Delete right after undo
  // would delete what was selected long ago
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
        meta: {
          id: doc.id,
          name: doc.name,
          createdAt: doc.createdAt,
          showVolume: doc.showVolume ?? true,
          showDelay: doc.showDelay ?? false,
          delayLimit: doc.delayLimit ?? 0.5,
          startOnLaunch: doc.startOnLaunch ?? false,
          showBalance: doc.showBalance ?? false,
        },
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
    setPrefs: (patch) => {
      const meta = get().meta;
      if (meta) set({ meta: { ...meta, ...patch } });
    },
    setViewport: (viewport) => set({ viewport }),

    onNodesChange: (changes) => {
      // selection and sizes are not graph edits; a drag is one step from start to end
      if (changes.some((c) => c.type === "remove" || c.type === "add" || c.type === "replace")) record("delete", 50);
      for (const c of changes) {
        if (c.type !== "position") continue;
        if (c.dragging) {
          if (!dragging) record();
          dragging = true;
        } else if (dragging) dragging = false;
        // moving with the arrow keys
        else if (c.position) record("nudge");
      }
      set({ nodes: applyNodeChanges(changes, get().nodes) });
    },
    onEdgesChange: (changes) => {
      // a node's edges are deleted in a separate call right after the node: the same step
      if (changes.some((c) => c.type === "remove" || c.type === "add" || c.type === "replace")) record("delete", 50);
      set({ edges: applyEdgeChanges(changes, get().edges) });
    },
    onConnect: (connection) => {
      const edges = addEdge(connection, get().edges);
      if (edges.length === get().edges.length) return;
      record();
      set({ edges });
    },
    // the canvas returns an edge the way it showed it, with the red problem mark;
    // the new edge is a copy of the old one, and the store doesn't need the mark
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
          // a label not changed by hand (in any of the languages) follows the chosen app
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

    setDelay: (id, delay) => {
      record(`delay:${id}`);
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, delay } } : n)),
      });
    },

    setDelays: (delays) => {
      record();
      set({
        nodes: get().nodes.map((n) => (n.id in delays ? { ...n, data: { ...n.data, delay: delays[n.id] } } : n)),
      });
    },

    setBalance: (id, balance) => {
      record(`balance:${id}`);
      set({
        nodes: get().nodes.map((n) => (n.id === id ? { ...n, data: { ...n.data, balance } } : n)),
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
