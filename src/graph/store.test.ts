import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GraphDoc } from "./doc";
import { useGraph } from "./store";

const doc = (patch: Partial<GraphDoc> = {}): GraphDoc => ({
  version: 1,
  id: "g",
  name: "Music",
  createdAt: 1,
  updatedAt: 1,
  viewport: null,
  nodes: [
    {
      id: "o",
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
      place: null,
      spaceView: null,
      pinned: [],
      position: { x: 0, y: 0 },
    },
  ],
  edges: [],
  delayLimit: 0.5,
  volumeLimit: 2,
  distanceLimit: 1,
  startOnLaunch: false,
  ...patch,
});

const graph = () => useGraph.getState();
const node = (id: string) => graph().nodes.find((n) => n.id === id)!;

beforeEach(() => {
  vi.useFakeTimers();
  graph().loadDoc(doc());
});
afterEach(() => vi.useRealTimers());

describe("graph editing", () => {
  it("undoes and redoes an added node", () => {
    graph().addNode("input", { x: 5, y: 5 });
    expect(graph().nodes).toHaveLength(2);
    graph().undo();
    expect(graph().nodes).toHaveLength(1);
    graph().redo();
    expect(graph().nodes).toHaveLength(2);
    expect(graph().nodes[1].data).toMatchObject({ kind: "input", volume: 1, sink: null });
  });

  it("undoes a slider movement in one step, and two movements apart in two", () => {
    for (const v of [0.9, 0.8, 0.7]) {
      graph().setVolume("o", v);
      vi.advanceTimersByTime(100);
    }
    vi.advanceTimersByTime(2000);
    graph().setVolume("o", 0.2);
    graph().undo();
    expect(node("o").data.volume).toBe(0.7);
    graph().undo();
    expect(node("o").data.volume).toBe(1);
  });

  it("starts the redo list over after a new edit", () => {
    graph().renameNode("o", "Hall");
    graph().undo();
    graph().renameNode("o", "Kitchen");
    graph().redo();
    expect(node("o").data.label).toBe("Kitchen");
  });

  it("doesn't count a rename to the same name", () => {
    graph().renameNode("o", "Speakers");
    graph().undo();
    expect(graph().nodes).toHaveLength(1);
    graph().addNode("cable", { x: 0, y: 0 });
    graph().renameNode("o", "Speakers");
    graph().undo();
    expect(graph().nodes).toHaveLength(1);
  });

  it("brings an undone state back unselected", () => {
    useGraph.setState({ nodes: graph().nodes.map((n) => ({ ...n, selected: true })) });
    graph().renameNode("o", "Hall");
    graph().undo();
    expect(node("o").selected).toBe(false);
  });

  it("forgets the history of another graph", () => {
    graph().renameNode("o", "Hall");
    graph().loadDoc(doc({ id: "other" }));
    graph().undo();
    expect(node("o").data.label).toBe("Speakers");
  });

  it("keeps the last hundred steps", () => {
    for (let i = 0; i < 120; i++) graph().renameNode("o", `Name ${i}`);
    for (let i = 0; i < 150; i++) graph().undo();
    expect(node("o").data.label).toBe("Name 19");
  });

  it("names an app node after its app unless renamed by hand", () => {
    graph().addNode("app", { x: 0, y: 0 });
    const app = graph().nodes[1].id;
    graph().setApp(app, { exe: "player.exe", path: "", name: "Player" });
    expect(node(app).data.label).toBe("Player");
    graph().setApp(app, { exe: "radio.exe", path: "", name: "Radio" });
    expect(node(app).data.label).toBe("Radio");
    graph().renameNode(app, "Music");
    graph().setApp(app, { exe: "player.exe", path: "", name: "Player" });
    expect(node(app).data.label).toBe("Music");
  });

  it("pins a slider to its node as an undo step, nothing is pinned by default", () => {
    expect(node("o").data.pinned).toEqual([]);
    graph().togglePinned("o", "delay");
    graph().togglePinned("o", "volume");
    expect(node("o").data.pinned).toEqual(["delay", "volume"]);
    graph().togglePinned("o", "delay");
    expect(node("o").data.pinned).toEqual(["volume"]);
    graph().undo();
    expect(node("o").data.pinned).toEqual(["delay", "volume"]);
  });

  it("sets several delays as one step", () => {
    graph().addNode("output", { x: 0, y: 100 });
    const other = graph().nodes[1].id;
    graph().setDelays({ o: -0.2, [other]: 0 });
    expect([node("o").data.delay, node(other).data.delay]).toEqual([-0.2, 0]);
    graph().undo();
    expect(graph().nodes).toHaveLength(2);
    expect(node("o").data.delay).toBe(0);
  });

  it("reads the options of old files as they were", () => {
    graph().loadDoc({ ...doc(), delayLimit: undefined, volumeLimit: undefined, distanceLimit: undefined, startOnLaunch: undefined } as unknown as GraphDoc);
    expect(graph().meta).toMatchObject({ delayLimit: 0.5, volumeLimit: 2, distanceLimit: 1, startOnLaunch: false });
  });

  describe("a place in space", () => {
    const at = (x: number) => ({ x, y: 0, z: 0 });
    const place = () => graph().nodes[0].data.place;
    const drag = (...xs: number[]) => xs.forEach((x, i) => graph().setPlace("o", at(x), i > 0));

    it("a drag is one undo step, however long it takes", () => {
      graph().loadDoc(doc());
      drag(1, 2, 3, 4);
      expect(place()).toEqual(at(4));
      graph().undo();
      expect(place()).toBeNull();
      graph().redo();
      expect(place()).toEqual(at(4));
    });

    it("two drags in a row are two steps", () => {
      graph().loadDoc(doc());
      drag(1, 2);
      drag(3, 4);
      graph().undo();
      expect(place()).toEqual(at(2));
      graph().undo();
      expect(place()).toBeNull();
    });

    it("turning it off is a step of its own", () => {
      graph().loadDoc(doc());
      drag(1, 2);
      graph().setPlace("o", null);
      expect(place()).toBeNull();
      graph().undo();
      expect(place()).toEqual(at(2));
    });

    it("undo leaves how it is looked at alone", () => {
      graph().loadDoc(doc());
      drag(1);
      const view = { solid: true, yaw: 1, pitch: 0.5 };
      graph().setSpaceView("o", view);
      graph().undo();
      expect(place()).toBeNull();
      expect(graph().nodes[0].data.spaceView).toEqual(view);
      graph().redo();
      expect(place()).toEqual(at(1));
      expect(graph().nodes[0].data.spaceView).toEqual(view);
    });

    it("looking at it isn't a step", () => {
      graph().loadDoc(doc());
      drag(1);
      graph().setSpaceView("o", { solid: true, yaw: 1, pitch: 0.5 });
      graph().setSpaceView("o", { solid: false, yaw: 1, pitch: 0.5 });
      graph().undo();
      expect(place()).toBeNull();
    });
  });
});
