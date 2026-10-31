import { create } from "zustand";
import { t } from "../i18n";
import { useSettings } from "../settings";
import { graphsApi } from "./api";
import { edgesToDoc, nodesToDoc, type GraphDoc, type GraphMeta } from "./doc";
import { uniqueName } from "./names";
import { starterGraphs, useGraph } from "./store";
import type { AudioEdge, AudioNode } from "./types";

export type SaveStatus = "saved" | "pending" | "saving" | "error";

/** Список графов и операции над ними. Открытый граф живёт в useGraph. */
type LibraryState = {
  ready: boolean;
  graphs: GraphMeta[];
  status: SaveStatus;
  error: string | null;
  init: () => Promise<void>;
  open: (id: string) => Promise<void>;
  create: () => Promise<string>;
  rename: (id: string, name: string) => Promise<void>;
  setShowVolume: (id: string, show: boolean) => Promise<void>;
  remove: (id: string) => Promise<void>;
};

/** Пауза после последнего изменения перед записью на диск */
const SAVE_DELAY_MS = 300;

// ---------- автосохранение ----------

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
  };
}

// Ключ содержимого: выделение, размеры нод и прочее временное в него не входят,
// поэтому их изменения не вызывают сохранения
const contentKey = (doc: GraphDoc | null) =>
  doc ? JSON.stringify([doc.id, doc.name, doc.viewport, doc.nodes, doc.edges, doc.showVolume]) : "";

let savedKey = "";
let timer: ReturnType<typeof setTimeout> | undefined;
// сохранения выполняются строго по очереди
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

/** Немедленно записать несохранённые изменения (перед сменой графа, при закрытии окна) */
export function flushSave(): Promise<void> {
  clearTimeout(timer);
  timer = undefined;
  queue = queue.then(saveNow);
  return queue;
}

useGraph.subscribe((state, prev) => {
  // загрузка другого графа — не изменение
  if (!state.meta || state.meta.id !== prev.meta?.id) return;
  if (contentKey(snapshot()) === savedKey) return;
  useLibrary.setState({ status: "pending" });
  clearTimeout(timer);
  timer = setTimeout(() => {
    timer = undefined;
    queue = queue.then(saveNow);
  }, SAVE_DELAY_MS);
});

// ---------- библиотека ----------

const metaOf = ({ id, name, createdAt, updatedAt, showVolume }: GraphDoc): GraphMeta => ({
  id,
  name,
  createdAt,
  updatedAt,
  showVolume,
});

/** Граф из старого автосохранения в localStorage (до появления файлов) */
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

// см. store.ts: при правке модуля в разработке — полная перезагрузка страницы
import.meta.hot?.accept(() => location.reload());

export const useLibrary = create<LibraryState>()((set, get) => {
  async function createFrom(name: string, nodes: AudioNode[], edges: AudioEdge[]): Promise<string> {
    await flushSave();
    // галочка громкости у нового графа — как задано в настройках
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

    // StrictMode в разработке вызывает эффекты дважды, поэтому init идемпотентен
    init: () =>
      (initOnce ??= guard(async () => {
        const { graphs, lastOpened } = await graphsApi.init();
        set({ graphs });
        if (graphs.length === 0) {
          const legacy = legacyGraph();
          if (legacy) {
            await createFrom(t("Мой граф"), legacy.nodes, legacy.edges);
            try {
              localStorage.removeItem(LEGACY_KEY);
            } catch {
              // не страшно: при следующем запуске графы уже будут в файлах
            }
          } else {
            // первый запуск: готовые графы, открыт первый
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
        return await createFrom(uniqueName(t("Новый граф"), get().graphs.map((g) => g.name)), [], []);
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
        // открытый граф переименовываем через холст, его сохранит автосохранение
        if (useGraph.getState().meta?.id === id) useGraph.getState().setName(trimmed);
        else await graphsApi.rename(id, trimmed);
      }),

    setShowVolume: (id, show) =>
      guard(async () => {
        set((s) => ({ graphs: s.graphs.map((g) => (g.id === id ? { ...g, showVolume: show } : g)) }));
        // открытый граф сохранит автосохранение, остальные — отдельной командой
        if (useGraph.getState().meta?.id === id) useGraph.getState().setShowVolume(show);
        else await graphsApi.setShowVolume(id, show);
      }),

    remove: (id) =>
      guard(async () => {
        const isOpen = useGraph.getState().meta?.id === id;
        if (isOpen) {
          // отложенное сохранение удаляемого графа больше не нужно
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
          else await createFrom(t("Новый граф"), [], []);
        }
      }),
  };
});
