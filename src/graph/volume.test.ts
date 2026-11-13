import { describe, expect, it } from "vitest";
import type { AudioEdge, AudioNode, AudioNodeKind } from "./types";
import { totalVolume } from "./volume";

const node = (id: string, kind: AudioNodeKind, volume = 1): AudioNode => ({
  id,
  type: "audio",
  position: { x: 0, y: 0 },
  data: { kind, label: id, sink: null, source: null, cable: null, app: null, volume, systemVolume: null, delay: 0, balance: 0, gate: -100, place: null, spaceView: null, pinned: [] },
});
const links = (...pairs: [string, string][]): AudioEdge[] => pairs.map(([source, target]) => ({ id: `${source}-${target}`, source, target }));

describe("the real volume of an output", () => {
  it("isn't shown if nothing on the way changes it", () => {
    const nodes = [node("mic", "input"), node("c", "cable"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, { o: 0.6 })).toBeNull();
    expect(totalVolume("o", [node("o", "output")], [], 2, { o: 0.6 })).toBeNull();
  });

  it("multiplies the system's sliders and the cables on the way", () => {
    const nodes = [node("mic", "input"), node("c", "cable", 1.5), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, { mic: 0.5, o: 0.8 })).toBe("60%");
  });

  it("counts how far a cable is in space, up to the graph's distance", () => {
    const far = node("c", "cable");
    far.data.place = { x: 0, y: 4, z: 0 };
    const nodes = [node("mic", "input"), far, node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, { o: 1 }, 10)).toBe("25%");
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, { o: 1 }, 2)).toBe("50%");
  });

  it("gives a range for paths of different volumes", () => {
    const nodes = [node("a", "input"), node("b", "input"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["a", "o"], ["b", "o"]), 2, { a: 0.4, b: 1, o: 0.5 })).toBe("20–50%");
  });

  it("starts at apps and the System node: what they listen to isn't what they play", () => {
    const nodes = [node("mic", "input"), node("app", "app"), node("c", "cable"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "app"], ["app", "c"], ["c", "o"]), 2, { mic: 0.1, app: 0.5 })).toBe("50%");
  });

  it("applies an output's slider to a straight link too", () => {
    expect(totalVolume("o", [node("s", "system"), node("o", "output")], links(["s", "o"]), 2, { o: 0.5 })).toBeNull();
    expect(totalVolume("o", [node("a", "app"), node("o", "output")], links(["a", "o"]), 2, { a: 0.5, o: 0.5 })).toBe("25%");
  });

  it("counts a cable beyond the graph's limit as the limit", () => {
    const nodes = [node("mic", "input"), node("c", "cable", 3), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, {})).toBe("200%");
  });

  it("takes 100% for a system slider it couldn't read", () => {
    const nodes = [node("mic", "input"), node("c", "cable", 0.5), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2, { mic: null, o: null })).toBe("50%");
  });

  it("follows the sound an output passes on, without that output's slider", () => {
    const nodes = [node("mic", "input"), node("a", "output"), node("b", "output")];
    expect(totalVolume("b", nodes, links(["mic", "a"], ["a", "b"]), 2, { mic: 0.5, a: 0.1, b: 0.8 })).toBe("40%");
    expect(totalVolume("b", nodes.slice(1), links(["a", "b"]), 2, { a: 0.1, b: 0.8 })).toBeNull();
  });

  it("prefers what the graph sets to what the system has now", () => {
    const o = node("o", "output");
    o.data.systemVolume = 0.5;
    const nodes = [node("c", "cable", 0.5), o];
    expect(totalVolume("o", nodes, links(["c", "o"]), 2, { o: 0.9 })).toBe("25%");
  });

  it("survives a loop", () => {
    const nodes = [node("a", "cable", 0.5), node("b", "cable"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["a", "b"], ["b", "a"], ["b", "o"]), 2, {})).toBe("50%");
  });
});
