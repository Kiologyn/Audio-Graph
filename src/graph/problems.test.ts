import { describe, expect, it } from "vitest";
import type { Cable } from "./cables";
import { nodesFromDoc, type DocNode } from "./doc";
import type { AudioDevice } from "./devices";
import type { EngineStatus } from "./engine";
import { allProblems, nodeNotes } from "./problems";

const docNode = (id: string, patch: Partial<DocNode>): DocNode => ({
  id,
  kind: "output",
  label: id,
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
  ...patch,
});

const nodes = nodesFromDoc([
  docNode("player", { kind: "app", app: { exe: "player.exe", path: "", name: "Player" } }),
  docNode("phones", { sink: { type: "device", id: "usb-1", name: "Headphones" } }),
  docNode("mic", { kind: "input", source: { type: "default" } }),
  docNode("line", { kind: "cable", cable: "c1" }),
]);
const data = (id: string) => nodes.find((n) => n.id === id)!.data;

const devices: AudioDevice[] = [
  // the headphones came back in another USB port
  { id: "usb-2", name: "Headphones", flow: "render", state: "active", isDefault: false, isVirtual: false, pair: null, pairName: null },
  { id: "mic-1", name: "Mic", flow: "capture", state: "active", isDefault: true, isVirtual: false, pair: null, pairName: null },
  { id: "r1", name: "Line 1", flow: "render", state: "active", isDefault: false, isVirtual: false, pair: "c1x", pairName: "Line 1" },
  { id: "c1x", name: "Line 1", flow: "capture", state: "active", isDefault: false, isVirtual: false, pair: "r1", pairName: "Line 1" },
  { id: "c2x", name: "Line 2", flow: "capture", state: "active", isDefault: false, isVirtual: false, pair: "r2", pairName: "Line 2" },
  { id: "r2", name: "Line 2", flow: "render", state: "active", isDefault: false, isVirtual: false, pair: "c2x", pairName: "Line 2" },
];
const cables: Cable[] = [
  {
    id: "c1",
    name: "Line 1",
    render: { type: "device", id: "r1", name: "Line 1" },
    capture: { type: "device", id: "c2x", name: "Line 2" },
    createdAt: 0,
  },
];

const status = (patch: Partial<EngineStatus>): EngineStatus => ({
  running: ["g"],
  graphId: "g",
  issues: [],
  streams: [],
  apps: [],
  troubled: [],
  ...patch,
});

describe("node notes", () => {
  it("put a problem of several nodes on the first, and only mark the others", () => {
    const s = status({ issues: [{ nodes: ["player", "line"], edges: [], message: "A loop" }] });
    expect(nodeNotes("player", data("player"), s, "g", [], devices)).toEqual({
      notes: [{ text: "A loop", level: "error" }],
      flagged: true,
    });
    expect(nodeNotes("line", data("line"), s, "g", [], devices)).toEqual({ notes: [], flagged: true });
  });

  it("find device errors of a device that changed its id", () => {
    const s = status({ streams: [{ sink: "usb-2", sources: [], errors: [{ device: "usb-2", message: "device disconnected" }] }] });
    expect(nodeNotes("phones", data("phones"), s, "g", [], devices).notes).toEqual([
      { text: "Device disconnected", level: "error" },
    ]);
  });

  it("find errors of the default device on a node that follows it", () => {
    const s = status({ streams: [{ sink: "x", sources: ["mic-1"], errors: [{ device: "mic-1", message: "busy" }] }] });
    expect(nodeNotes("mic", data("mic"), s, "g", [], devices).notes).toEqual([{ text: "Busy", level: "error" }]);
  });

  it("show device errors only while the graph runs, once each", () => {
    const error = { device: "usb-2", message: "gone" };
    const streams = [
      { sink: "usb-2", sources: [], errors: [error] },
      { sink: "usb-2", sources: [], errors: [error] },
    ];
    expect(nodeNotes("phones", data("phones"), status({ streams }), "g", [], devices).notes).toHaveLength(1);
    expect(nodeNotes("phones", data("phones"), status({ streams, running: [] }), "g", [], devices).notes).toEqual([]);
  });

  it("say an app waits without calling it an error", () => {
    const s = status({ apps: [{ node: "player", state: "not_running", error: null }] });
    const { notes, flagged } = nodeNotes("player", data("player"), s, "g", [], devices);
    expect(notes.map((n) => n.level)).toEqual(["info"]);
    expect(flagged).toBe(false);
  });

  it("warn about a cable of two lines even when nothing runs", () => {
    const { notes, flagged } = nodeNotes("line", data("line"), status({ running: [], graphId: null }), "g", cables, devices);
    expect(notes.map((n) => n.level)).toEqual(["warning"]);
    expect(flagged).toBe(false);
  });

  it("ignore another graph's problems", () => {
    const s = status({ graphId: "other", issues: [{ nodes: ["player"], edges: [], message: "A loop" }] });
    expect(nodeNotes("player", data("player"), s, "g", [], devices)).toEqual({ notes: [], flagged: false });
  });
});

describe("problem list", () => {
  it("names the nodes of the open graph", () => {
    const s = status({ issues: [{ nodes: ["player", "phones"], edges: [], message: "Twice" }] });
    expect(allProblems(s, "g", nodes, [], devices)).toEqual([{ key: "issue-0", who: "player, phones", text: "Twice", level: "error" }]);
    // another graph's issues come without names
    expect(allProblems({ ...s, graphId: "other" }, "g", nodes, [], devices)[0].who).toBeNull();
  });

  it("lists errors of this graph's devices only, with the device name", () => {
    const s = status({
      streams: [
        { sink: "usb-2", sources: [], errors: [{ device: "usb-2", message: "gone" }, { device: "usb-2", message: "busy" }] },
        { sink: "else", sources: [], errors: [{ device: "else", message: "another graph's" }] },
      ],
    });
    const list = allProblems(s, "g", nodes, [], devices);
    expect(list.map((p) => [p.who, p.text])).toEqual([
      ["Headphones", "Gone"],
      ["Headphones", "Busy"],
    ]);
    // every row has its own key
    expect(new Set(list.map((p) => p.key)).size).toBe(list.length);
  });
});
