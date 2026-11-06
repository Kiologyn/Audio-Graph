import { describe, expect, it } from "vitest";
import { virtualDeviceIds, virtualDeviceName } from "../platform";
import { edgesFromDoc, edgesToDoc, nodesFromDoc, nodesToDoc, type DocNode } from "./doc";
import type { AudioNode } from "./types";

const docNode = (patch: Partial<DocNode>): DocNode => ({
  id: "n",
  kind: "output",
  label: "Speakers",
  sink: null,
  source: null,
  cable: null,
  app: null,
  volume: 1,
  delay: 0,
  balance: 0,
  pinned: [],
  position: { x: 1, y: 2 },
  ...patch,
});

describe("graph file", () => {
  it("keeps what every kind of node chooses", () => {
    const nodes: DocNode[] = [
      docNode({ id: "o", sink: { type: "device", id: "spk", name: "Speakers" }, volume: 1.5, delay: -0.2, balance: 0.4 }),
      docNode({ id: "i", kind: "input", source: { type: "default" }, volume: 0.5, balance: -1 }),
      docNode({ id: "c", kind: "cable", cable: "line-1", volume: 2, delay: 0.3, balance: 0.1, pinned: ["volume", "delay"] }),
      docNode({ id: "a", kind: "app", app: { exe: "player.exe", path: "C:\\player.exe", name: "Player" }, volume: 0.3 }),
      docNode({ id: "s", kind: "system" }),
    ];
    expect(nodesToDoc(nodesFromDoc(nodes))).toEqual(nodes);
  });

  it("drops what a kind of node can't choose", () => {
    // an app node with leftovers: apps have no balance or delay, the system node has nothing, so no pins either
    const [app, system] = nodesToDoc(
      nodesFromDoc([
        docNode({ id: "a", kind: "app", sink: { type: "default" }, delay: 0.3, balance: 1, volume: 0.5, pinned: ["volume", "delay"] }),
        docNode({ id: "s", kind: "system", volume: 0.2, cable: "x", pinned: ["volume"] }),
      ]),
    );
    expect(app).toMatchObject({ sink: null, delay: 0, balance: 0, volume: 0.5, pinned: ["volume"] });
    expect(system).toMatchObject({ volume: 1, cable: null, pinned: [] });
  });

  it("reads nodes of old files without the newer fields", () => {
    const old = { id: "o", kind: "output", label: "Speakers", position: { x: 0, y: 0 } } as unknown as DocNode;
    const [node] = nodesFromDoc([old]);
    expect(node.data).toEqual({
      kind: "output",
      label: "Speakers",
      sink: null,
      source: null,
      cable: null,
      app: null,
      volume: 1,
      delay: 0,
      balance: 0,
      pinned: [],
    });
  });

  it("keeps edges and their ends", () => {
    const edges = [{ id: "a->b", source: "a", target: "b" }];
    const canvas = edgesFromDoc(edges);
    expect(canvas[0]).toMatchObject({ sourceHandle: "out", targetHandle: "in" });
    expect(edgesToDoc(canvas)).toEqual(edges);
  });

  it("saves a node's place, not the canvas's extras", () => {
    const [node] = nodesFromDoc([docNode({})]);
    const shown: AudioNode = { ...node, selected: true, measured: { width: 200, height: 100 } };
    expect(nodesToDoc([shown])[0].position).toEqual({ x: 1, y: 2 });
    expect(Object.keys(nodesToDoc([shown])[0])).not.toContain("selected");
  });
});

describe("virtual cable devices (Linux)", () => {
  // as virtual_device_names_are_safe in plan.rs
  it("have safe names made of the node id", () =>
    expect(virtualDeviceIds("3f2a-9b {x}/'\"")).toEqual(["audiograph.3f2a-9bx", "audiograph.3f2a-9bx.out"]));
  it("are shown with the node's label", () => {
    expect(virtualDeviceName(' Calls "1" ')).toBe("Calls 1 (Audio Graph)");
    expect(virtualDeviceName("'\\")).toBe("Audio Graph");
  });
});
