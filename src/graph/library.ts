import { create } from "zustand";
import { t } from "../i18n";
import { useSettings } from "../settings";
import { graphsApi } from "./api";
import { edgesToDoc, nodesToDoc, type GraphDoc, type GraphMeta, type GraphSliders } from "./doc";
import { uniqueName } from "./names";
import { starterGraphs, useGraph } from "./store";
import type { AudioEdge, AudioNode } from "./types";

export type SaveStatus = "saved" | "pending" | "saving" | "error";

/** The graph list and operations on it. The open graph lives in useGraph. */
type LibraryState = {
  ready: boolean;
  graphs: GraphMeta[];
  status: SaveStatus;
  error: string | null;
  init: () => Promise<void>;
  open: (id: string) => Promise<void>;
  create: () => Promise<string>;
  rename: (id: string, name: string) => Promise<void>;
  /** The graph's volume and delay sliders and the delay range */
  setSliders: (id: string, patch: Partial<GraphSliders>) => Promise<void>;
  remove: (id: string) => Promise<void>;
  /** Puts the list in this order of graph ids */
  reorder: (ids: string[]) => Promise<void>;
};

/** Pause after the last change before writing to disk */
const SAVE_DELAY_MS = 300;

// ---------- autosave ----------

function snapshot(): GraphDoc | null {
  const { meta, viewport, nodes, edges } = useGraph.getState();
  if (!meta) return null;
  return {
    version: 1,
    id: meta.id,
    name: meta.name,
    createdAt: meta.createdAt,
    updatedAt: 0,
    viewport,
    nodes: nodesToDoc(nodes),
    edges: edgesToDoc(edges),
    showVolume: meta.showVolume,
    showDelay: meta.showDelay,
    delayLimit: meta.delayLimit,
  };
}

// Content key: selection, node sizes and other transient state are not part of it,
// so their changes don't trigger a save
const contentKey = (doc: GraphDoc | null) =>
  doc
    ? JSON.stringify([doc.id, doc.name, doc.viewport, doc.nodes, doc.edges, doc.showVolume, doc.showDelay, doc.delayLimit])
    : "";

let savedKey = "";
let timer: ReturnType<typeof setTimeout> | undefined;
// saves run strictly in order
let queue: Promise<void> = Promise.resolve();

function markLoaded() {
  savedKey = contentKey(snapshot());
  useLibrary.setState({ status: "saved", error: null });
}

async function saveNow() {
  const doc = snapshot();
  const key = contentKey(doc);
  if (!doc || key === savedKey) return;
  useLibrary.setState({ status: "saving" });
  try {
    const meta = await graphsApi.save(doc);
    savedKey = key;
    useLibrary.setState((s) => ({
      graphs: s.graphs.map((g) => (g.id === meta.id ? meta : g)),
      status: timer ? "pending" : "saved",
      error: null,
    }));
  } catch (e) {
    useLibrary.setState({ status: "error", error: String(e) });
  }
}

/** Write unsaved changes now (before switching graphs, when the window closes) */
export function flushSave(): Promise<void> {
  clearTimeout(timer);
  timer = undefined;
  queue = queue.then(saveNow);
  return queue;
}

useGraph.subscribe((state, prev) => {
  // loading another graph is not a change
  if (!state.meta || state.meta.id !== prev.meta?.id) return;
  if (contentKey(snapshot()) === savedKey) return;
  useLibrary.setState({ status: "pending" });
  clearTimeout(timer);
  timer = setTimeout(() => {
    timer = undefined;
    queue = queue.then(saveNow);
  }, SAVE_DELAY_MS);
});

// ---------- library ----------

const metaOf = ({ id, name, createdAt, updatedAt, showVolume, showDelay, delayLimit }: GraphDoc): GraphMeta => ({
  id,
  name,
  createdAt,
  updatedAt,
  showVolume,
  showDelay,
  delayLimit,
});

/** A graph from the old autosave in localStorage (before files existed) */
const LEGACY_KEY = "audio-graph:canvas";
function legacyGraph(): { nodes: AudioNode[]; edges: AudioEdge[] } | null {
  try {
    const raw = localStorage.getItem(LEGACY_KEY);
    if (!raw) return null;
    const { state } = JSON.parse(raw) as { state: { nodes: AudioNode[]; edges: AudioEdge[] } };
    for (const n of state.nodes) if ((n.data.kind as string) === "bus") n.data.kind = "cable";
    return state;
  } catch {
    return null;
  }
}

let initOnce: Promise<void> | null = null;

// see store.ts: editing this module in development reloads the whole page
import.meta.hot?.accept(() => location.reload());

export const useLibrary = create<LibraryState>()((set, get) => {
  async function createFrom(name: string, nodes: AudioNode[], edges: AudioEdge[]): Promise<string> {
    await flushSave();
    // a new graph's volume checkbox is as set in the settings
    const showVolume = useSettings.getState().newGraphVolume;
    const doc = await graphsApi.create(name, nodesToDoc(nodes), edgesToDoc(edges), showVolume);
    set((s) => ({ graphs: [...s.graphs, metaOf(doc)] }));
    useGraph.getState().loadDoc(doc);
    markLoaded();
    return doc.id;
  }

  async function guard(action: () => Promise<void>) {
    try {
      await action();
    } catch (e) {
      set({ status: "error", error: String(e) });
    }
  }

  return {
    ready: false,
    graphs: [],
    status: "saved",
    error: null,

    // StrictMode runs effects twice in development, so init is idempotent
    init: () =>
      (initOnce ??= guard(async () => {
        const { graphs, lastOpened } = await graphsApi.init();
        set({ graphs });
        if (graphs.length === 0) {
          const legacy = legacyGraph();
          if (legacy) {
            await createFrom(t("graph_default_name"), legacy.nodes, legacy.edges);
            try {
              localStorage.removeItem(LEGACY_KEY);
            } catch {
              // not a problem: on the next start the graphs will already be in files
            }
          } else {
            // first start: ready-made graphs, the first one open
            const ids = [];
            for (const g of starterGraphs()) ids.push(await createFrom(g.name, g.nodes, g.edges));
            await get().open(ids[0]);
          }
        } else {
          await get().open(graphs.some((g) => g.id === lastOpened) ? lastOpened! : graphs[0].id);
        }
        set({ ready: true });
      })),

    open: (id) =>
      guard(async () => {
        if (useGraph.getState().meta?.id === id) return;
        await flushSave();
        const doc = await graphsApi.load(id);
        useGraph.getState().loadDoc(doc);
        markLoaded();
      }),

    create: async () => {
      try {
        return await createFrom(uniqueName(t("new_graph"), get().graphs.map((g) => g.name)), [], []);
      } catch (e) {
        set({ status: "error", error: String(e) });
        return "";
      }
    },

    rename: (id, name) =>
      guard(async () => {
        const trimmed = name.trim();
        if (!trimmed) return;
        set((s) => ({ graphs: s.graphs.map((g) => (g.id === id ? { ...g, name: trimmed } : g)) }));
        // the open graph is renamed through the canvas, autosave saves it
        if (useGraph.getState().meta?.id === id) useGraph.getState().setName(trimmed);
        else await graphsApi.rename(id, trimmed);
      }),

    setSliders: (id, patch) =>
      guard(async () => {
        const graph = get().graphs.find((g) => g.id === id);
        if (!graph) return;
        const next = { ...graph, ...patch };
        set((s) => ({ graphs: s.graphs.map((g) => (g.id === id ? next : g)) }));
        // autosave saves the open graph, the others go through a separate command
        if (useGraph.getState().meta?.id === id) useGraph.getState().setSliders(patch);
        else await graphsApi.setSliders(id, next);
      }),

    reorder: (ids) =>
      guard(async () => {
        const byId = new Map(get().graphs.map((g) => [g.id, g]));
        set({ graphs: ids.flatMap((id) => byId.get(id) ?? []) });
        await graphsApi.reorder(ids);
      }),

    remove: (id) =>
      guard(async () => {
        const isOpen = useGraph.getState().meta?.id === id;
        if (isOpen) {
          // a pending save of the graph being deleted is no longer needed
          clearTimeout(timer);
          timer = undefined;
          await queue;
        }
        await graphsApi.remove(id);
        const graphs = get().graphs.filter((g) => g.id !== id);
        set({ graphs });
        if (isOpen) {
          useGraph.getState().unload();
          savedKey = "";
          if (graphs.length > 0) await get().open(graphs[0].id);
          else await createFrom(t("new_graph"), [], []);
        }
      }),
  };
});
