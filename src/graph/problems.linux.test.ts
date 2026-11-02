import { describe, expect, it, vi } from "vitest";

// on Linux every cable node is a pair of virtual devices the graph creates
vi.mock("../platform", async (original) => ({
  ...(await original<typeof import("../platform")>()),
  dynamicCables: () => true,
}));

const { nodeNotes } = await import("./problems");
const { nodesFromDoc } = await import("./doc");

const [mix] = nodesFromDoc([
  {
    id: "3f2a-9b",
    kind: "cable",
    label: "Mix",
    sink: null,
    source: null,
    cable: null,
    app: null,
    volume: 1,
    delay: 0,
    balance: 0,
    position: { x: 0, y: 0 },
  },
]);

const status = (device: string) => ({
  running: ["g"],
  graphId: "g",
  issues: [],
  streams: [{ sink: device, sources: [], errors: [{ device, message: "failed to create" }] }],
  apps: [],
  troubled: [],
});

describe("node notes on Linux", () => {
  it("find errors of a cable node's own virtual devices", () => {
    for (const device of ["audiograph.3f2a-9b", "audiograph.3f2a-9b.out"]) {
      expect(nodeNotes(mix.id, mix.data, status(device), "g", [], []).notes).toEqual([{ text: "Failed to create", level: "error" }]);
    }
    expect(nodeNotes(mix.id, mix.data, status("audiograph.other"), "g", [], []).notes).toEqual([]);
  });
});
