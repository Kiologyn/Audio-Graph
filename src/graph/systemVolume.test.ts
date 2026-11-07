import { describe, expect, it } from "vitest";
import { movedAwayFrom } from "./systemVolume";

describe("following the system's slider", () => {
  it("follows it once it leaves the graph's value", () => {
    expect(movedAwayFrom(0.5, 0.5, 0.7)).toBe(true);
  });

  it("waits while a graph just started hasn't reached its value", () => {
    expect(movedAwayFrom(0.5, 0.9, 0.9)).toBe(false);
    expect(movedAwayFrom(0.5, 0.9, 0.5)).toBe(false);
  });

  it("leaves alone what the graph doesn't set or the system can't read", () => {
    expect(movedAwayFrom(null, 0.5, 0.7)).toBe(false);
    expect(movedAwayFrom(0.5, null, 0.7)).toBe(false);
    expect(movedAwayFrom(0.5, 0.5, null)).toBe(false);
  });
});
