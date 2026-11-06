import { describe, expect, it } from "vitest";
import type { AudioEdge, AudioNode, AudioNodeKind } from "./types";
import { totalVolume } from "./volume";

const node = (id: string, kind: AudioNodeKind, volume = 1): AudioNode => ({
  id,
  type: "audio",
  position: { x: 0, y: 0 },
  data: { kind, label: id, sink: null, source: null, cable: null, app: null, volume, delay: 0, balance: 0, pinned: [] },
});
const links = (...pairs: [string, string][]): AudioEdge[] => pairs.map(([source, target]) => ({ id: `${source}-${target}`, source, target }));

describe("the real volume of an output", () => {
  it("isn't shown if nothing on the way changes it", () => {
    const nodes = [node("mic", "input"), node("c", "cable"), node("o", "output", 2)];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2)).toBeNull();
    expect(totalVolume("o", [node("o", "output", 1.5)], [], 2)).toBeNull();
  });

  it("multiplies the nodes on the way", () => {
    const nodes = [node("mic", "input", 0.5), node("c", "cable", 1.5), node("o", "output", 2)];
    expect(totalVolume("o", nodes, links(["mic", "c"], ["c", "o"]), 2)).toBe("150%");
  });

  it("gives a range for paths of different volumes", () => {
    const nodes = [node("a", "input", 0.4), node("b", "input"), node("o", "output", 2)];
    expect(totalVolume("o", nodes, links(["a", "o"], ["b", "o"]), 2)).toBe("80–200%");
  });

  it("starts at apps and the System node: what they listen to isn't what they play", () => {
    const nodes = [node("mic", "input", 0.1), node("app", "app", 0.5), node("c", "cable"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["mic", "app"], ["app", "c"], ["c", "o"]), 2)).toBe("50%");
  });

  it("knows what the system does to a straight link", () => {
    // "System" doesn't take the output's volume, an app takes it up to 100%
    expect(totalVolume("o", [node("s", "system"), node("o", "output", 1.5)], links(["s", "o"]), 2)).toBe("100%");
    expect(totalVolume("o", [node("a", "app", 0.5), node("o", "output", 3)], links(["a", "o"]), 4)).toBe("100%");
    expect(totalVolume("o", [node("a", "app", 0.5), node("o", "output", 1.5)], links(["a", "o"]), 2)).toBe("75%");
  });

  it("counts a volume beyond the graph's limit as the limit", () => {
    const nodes = [node("mic", "input", 3), node("o", "output", 1)];
    expect(totalVolume("o", nodes, links(["mic", "o"]), 2)).toBe("200%");
  });

  it("survives a loop", () => {
    const nodes = [node("a", "cable", 0.5), node("b", "cable"), node("o", "output")];
    expect(totalVolume("o", nodes, links(["a", "b"], ["b", "a"], ["b", "o"]), 2)).toBe("50%");
  });
});
