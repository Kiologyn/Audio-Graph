import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphDoc } from "./doc";

// the Rust side: graphs on "disk" and every save
const api = vi.hoisted(() => ({
  saved: [] as GraphDoc[],
  graphs: new Map<string, GraphDoc>(),
}));

/** The list entry of a graph file, as graphs.rs returns it */
const metaOf = ({ version: _v, viewport: _vp, nodes: _n, edges: _e, ...meta }: GraphDoc) => meta;

vi.mock("./api", () => ({
  graphsApi: {
    init: async () => ({ graphs: [...api.graphs.values()], lastOpened: null }),
    load: async (id: string) => structuredClone(api.graphs.get(id)!),
    save: async (graph: GraphDoc) => {
      api.saved.push(structuredClone(graph));
      return { ...metaOf(graph), updatedAt: 2 };
    },
    setPrefs: async () => {},
    remove: async (id: string) => void api.graphs.delete(id),
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: async () => null, save: async () => null }));

const { fileName, flushSave, useLibrary } = await import("./library");
const { useGraph } = await import("./store");

const doc = (id: string): GraphDoc => ({
  version: 1,
  id,
  name: id,
  createdAt: 1,
  updatedAt: 1,
  viewport: null,
  nodes: [
    {
      id: `${id}-out`,
      kind: "output",
      label: "Speakers",
      sink: null,
      source: null,
      cable: null,
      app: null,
      volume: 1,
      systemVolume: null,
      delay: 0,
      balance: 0,
      gate: -100,
      pinned: [],
      position: { x: 0, y: 0 },
    },
  ],
  edges: [],
  delayLimit: 0.5,
  volumeLimit: 2,
  startOnLaunch: false,
});

beforeEach(async () => {
  vi.useFakeTimers();
  api.graphs = new Map([
    ["a", doc("a")],
    ["b", doc("b")],
  ]);
  useLibrary.setState({ graphs: [...api.graphs.values()].map(metaOf) });
  await useLibrary.getState().open("a");
  api.saved = [];
});
afterEach(() => {
  useGraph.getState().unload();
  vi.useRealTimers();
});

const settle = async () => {
  await vi.advanceTimersByTimeAsync(400);
  await flushSave();
};

describe("autosave", () => {
  it("doesn't save what isn't in the file: selection", async () => {
    useGraph.setState({ nodes: useGraph.getState().nodes.map((n) => ({ ...n, selected: true })) });
    await settle();
    expect(api.saved).toHaveLength(0);
  });

  it("saves a burst of edits once, after a pause", async () => {
    for (const v of [0.9, 0.8, 0.7]) {
      useGraph.getState().setVolume("a-out", v);
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(api.saved).toHaveLength(0);
    await settle();
    expect(api.saved).toHaveLength(1);
    expect(api.saved[0].nodes[0].volume).toBe(0.7);
  });

  it("saves the graph's options with it", async () => {
    useGraph.getState().setPrefs({ delayLimit: 0.8 });
    await settle();
    expect(api.saved).toHaveLength(1);
    expect(api.saved[0]).toMatchObject({ id: "a", delayLimit: 0.8, startOnLaunch: false });
  });

  it("writes pending changes before another graph opens", async () => {
    useGraph.getState().renameNode("a-out", "Hall");
    await useLibrary.getState().open("b");
    expect(api.saved.map((g) => [g.id, g.nodes[0].label])).toEqual([["a", "Hall"]]);
    expect(useGraph.getState().meta?.id).toBe("b");
    // opening a graph isn't an edit of it
    await settle();
    expect(api.saved).toHaveLength(1);
  });

  it("doesn't bring back a deleted graph with a pending save", async () => {
    useGraph.getState().renameNode("a-out", "Hall");
    await useLibrary.getState().remove("a");
    await settle();
    expect(api.saved.filter((g) => g.id === "a")).toHaveLength(0);
    expect(useGraph.getState().meta?.id).toBe("b");
  });
});

describe("export file name", () => {
  it("is the graph's name without what file names can't have", () => {
    expect(fileName("Music")).toBe("Music");
    expect(fileName('Music/Games: "best"?')).toBe("Music_Games_ _best__");
    expect(fileName(String.raw`a\b<c>|d*`)).toBe("a_b_c__d_");
    expect(fileName("tab\there")).toBe("tab_here");
    expect(fileName("Night mix...")).toBe("Night mix");
    expect(fileName(" ")).toBe("graph");
    expect(fileName("Музыка")).toBe("Музыка");
  });
});
