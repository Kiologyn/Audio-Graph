import { describe, expect, it } from "vitest";
import { cableDeviceIds, cableMismatch, cablePower, idsToToggle, newFoundCables, type Cable } from "./cables";
import type { AudioDevice } from "./devices";
import { uniqueName } from "./names";
import { inOrder } from "./sortable";

const device = (id: string, flow: AudioDevice["flow"], patch: Partial<AudioDevice> = {}): AudioDevice => ({
  id,
  name: id,
  flow,
  state: "active",
  isDefault: false,
  isVirtual: false,
  pair: null,
  pairName: null,
  ...patch,
});

const cable = (id: string, render: string | null, capture: string | null): Cable => ({
  id,
  name: id,
  render: render ? { type: "device", id: render, name: render } : null,
  capture: capture ? { type: "device", id: capture, name: capture } : null,
  createdAt: 0,
});

// two lines of a virtual cable driver: each output device is paired with its recording device
const lines = [
  device("r1", "render", { pair: "c1", pairName: "Line 1" }),
  device("c1", "capture", { pair: "r1", pairName: "Line 1" }),
  device("r2", "render", { pair: "c2", pairName: "Line 2", state: "disabled" }),
  device("c2", "capture", { pair: "r2", pairName: "Line 2", state: "disabled" }),
  device("spk", "render"),
];

describe("uniqueName", () => {
  // as unique_name in storage.rs
  it("counts up from 2 and takes a gap", () => {
    expect(uniqueName("Cable", ["Mixer"])).toBe("Cable");
    expect(uniqueName("Cable", ["Cable", "Cable 2"])).toBe("Cable 3");
    expect(uniqueName("Cable", ["Cable", "Cable 3"])).toBe("Cable 2");
  });
});

describe("cables", () => {
  it("have devices only where chosen", () => {
    expect(cableDeviceIds(cable("a", "r1", null))).toEqual(["r1"]);
    expect(cableDeviceIds({ ...cable("a", null, null), render: { type: "default" } })).toEqual([]);
  });

  it("found by the driver are offered once", () => {
    expect(newFoundCables([], lines).map((f) => f.name)).toEqual(["Line 1", "Line 2"]);
    // Line 1 is already in the list
    const found = newFoundCables([cable("mine", "r1", "c1")], lines);
    expect(found).toEqual([
      {
        name: "Line 2",
        sides: { render: { type: "device", id: "r2", name: "r2" }, capture: { type: "device", id: "c2", name: "c2" } },
      },
    ]);
    // a cable with only one of the devices doesn't count as that cable
    expect(newFoundCables([cable("half", "r1", null)], lines).map((f) => f.name)).toEqual(["Line 1", "Line 2"]);
  });

  it("warn when their sides belong to different lines", () => {
    expect(cableMismatch(cable("ok", "r1", "c1"), lines)).toBeNull();
    expect(cableMismatch(cable("half", "r1", null), lines)).toBeNull();
    expect(cableMismatch(cable("mixed", "r1", "c2"), lines)).toEqual({
      code: "cable_output_mismatch",
      vars: { device: "r1", pair: "c1" },
    });
    // a device without a driver pair can't be judged
    expect(cableMismatch(cable("odd", "spk", "c1"), lines)).toEqual({
      code: "cable_input_mismatch",
      vars: { device: "c1", pair: "r1" },
    });
  });

  it("are on, off, partly on or not in the system", () => {
    expect(cablePower(cable("a", "r1", "c1"), lines)).toBe("on");
    expect(cablePower(cable("b", "r2", "c2"), lines)).toBe("off");
    expect(cablePower(cable("c", "r1", "c2"), lines)).toBe("mixed");
    expect(cablePower(cable("d", "gone", null), lines)).toBe("none");
  });

  it("switch only the devices that need it", () => {
    const cables = [cable("a", "r1", "c2"), cable("b", "r2", null)];
    expect(idsToToggle(cables, lines, true)).toEqual(["r2", "c2"]);
    expect(idsToToggle(cables, lines, false)).toEqual(["r1"]);
  });
});

describe("list order", () => {
  it("follows the dragged order and keeps rows added meanwhile at the end", () => {
    const list = ["a", "b", "c", "d"].map((id) => ({ id }));
    expect(inOrder(list, ["c", "gone", "a"]).map((x) => x.id)).toEqual(["c", "a", "b", "d"]);
    expect(inOrder(list, []).map((x) => x.id)).toEqual(["a", "b", "c", "d"]);
  });
});

describe("cable devices back under a new id", () => {
  // Line 1's recording side came back as another device with the same name
  const moved = [
    device("r1", "render"),
    { ...device("c1-new", "capture"), name: "c1" },
  ];
  const line = cable("line", "r1", "c1");

  it("are found by name, as the engine finds them", () => {
    expect(cableDeviceIds(line)).toEqual(["r1", "c1"]);
    expect(cableDeviceIds(line, moved)).toEqual(["r1", "c1-new"]);
  });

  it("can be switched", () => {
    expect(cablePower(line, moved)).toBe("on");
    expect(idsToToggle([line], moved, false)).toEqual(["r1", "c1-new"]);
  });
});
