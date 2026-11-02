import { describe, expect, it } from "vitest";
import { describeDevice, findDevice, type AudioDevice } from "./devices";

const device = (id: string, name: string, patch: Partial<AudioDevice> = {}): AudioDevice => ({
  id,
  name,
  flow: "render",
  state: "active",
  isDefault: false,
  isVirtual: false,
  pair: null,
  pairName: null,
  ...patch,
});

describe("findDevice", () => {
  // the same cases as devices::tests::finds_a_device_under_a_new_id in Rust
  const list = [
    device("usb-port-2", "Headphones (USB)"),
    device("bt", "Speakers"),
    device("bt-2", "Speakers"),
    device("off", "Mic", { state: "disabled" }),
    device("ag", "Calls (Audio Graph)", { isVirtual: true }),
  ];
  const id = (id: string, name: string) => findDevice({ id, name }, "render", list)?.id;

  it("finds by id whatever the name", () => expect(id("bt", "Old name")).toBe("bt"));
  it("finds the same headphones in another USB port by name", () =>
    expect(id("usb-port-1", "Headphones (USB)")).toBe("usb-port-2"));
  it("doesn't guess between two devices with the name", () => expect(id("gone", "Speakers")).toBeUndefined());
  it("doesn't take a turned off device", () => expect(id("gone", "Mic")).toBeUndefined());
  it("doesn't take a graph's own cable device", () => expect(id("gone", "Calls (Audio Graph)")).toBeUndefined());
  it("looks in one direction only", () =>
    expect(findDevice({ id: "usb-port-1", name: "Headphones (USB)" }, "capture", list)).toBeUndefined());
});

describe("describeDevice", () => {
  const list = [device("spk", "Speakers"), device("off", "Old speakers", { state: "disabled" })];

  it("shows nothing for no choice", () =>
    expect(describeDevice(null, "render", list, true)).toEqual({ text: null, missing: false, disabled: false }));
  it("shows the current name of a device that's there", () =>
    expect(describeDevice({ type: "device", id: "spk", name: "Renamed" }, "render", list, true)).toEqual({
      text: "Speakers",
      missing: false,
      disabled: false,
    }));
  it("says a turned off device is off", () =>
    expect(describeDevice({ type: "device", id: "off", name: "Old speakers" }, "render", list, true).disabled).toBe(true));
  it("calls a device missing only once the list has come", () => {
    const gone = { type: "device", id: "gone", name: "USB speakers" } as const;
    expect(describeDevice(gone, "render", [], false)).toEqual({ text: "USB speakers", missing: false, disabled: false });
    expect(describeDevice(gone, "render", list, true)).toEqual({ text: "USB speakers", missing: true, disabled: false });
  });
  it("never calls the default device missing", () =>
    expect(describeDevice({ type: "default" }, "capture", [], true).missing).toBe(false));
});
