import { describe, expect, it } from "vitest";
import { nextMeter, toDb } from "./levels";

const FLOOR = -100;

describe("toDb", () => {
  it("turns sample values into dB within the slider", () => {
    expect(toDb(1, FLOOR)).toBe(0);
    expect(toDb(0.1, FLOOR)).toBeCloseTo(-20);
    expect(toDb(0, FLOOR)).toBe(FLOOR);
    expect(toDb(1e-9, FLOOR)).toBe(FLOOR);
    expect(toDb(2, FLOOR)).toBe(0);
  });
});

describe("nextMeter", () => {
  const quiet = { db: FLOOR, openUntil: 0 };

  it("rises at once and falls slowly", () => {
    const peak = nextMeter(quiet, -20, -100, 0, FLOOR);
    expect(peak.db).toBe(-20);
    const after = nextMeter(peak, FLOOR, -100, 40, FLOOR);
    expect(after.db).toBeLessThan(-20);
    expect(after.db).toBeGreaterThan(-25);
  });

  it("comes to rest at the bottom", () => {
    let meter = { db: -99, openUntil: 0 };
    for (let i = 0; i < 3; i++) meter = nextMeter(meter, FLOOR, -40, i, FLOOR);
    expect(meter.db).toBe(FLOOR);
  });

  it("stays open for the gate's hold after the sound falls below the threshold", () => {
    const open = nextMeter(quiet, -20, -40, 1000, FLOOR);
    expect(open.openUntil).toBe(1250);
    expect(nextMeter(open, -60, -40, 1100, FLOOR).openUntil).toBe(1250);
  });

  it("doesn't open for what is quieter than the threshold", () => {
    expect(nextMeter(quiet, -60, -40, 1000, FLOOR).openUntil).toBe(0);
  });
});
