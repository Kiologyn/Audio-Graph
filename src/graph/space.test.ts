import { describe, expect, it } from "vitest";
import {
  DEFAULT_DISTANCE_LIMIT,
  FIRST_VIEW,
  FROM_ABOVE,
  atAzimuth,
  atDistance,
  atElevation,
  azimuth,
  coneFaces,
  distanceGain,
  distanceOf,
  dragged,
  elevation,
  frameAt,
  grab,
  labelSpot,
  moveOnNode,
  project,
  ringPlaces,
  turned,
  upsideDown,
  withinLimit,
  type Handle,
  type OnNode,
  type Place,
  type SpaceView,
} from "./space";

const view = (yaw: number, pitch: number): SpaceView => ({ solid: true, yaw, pitch });
const near = (a: number, b: number) => expect(a).toBeCloseTo(b, 6);

describe("looking at a place", () => {
  it("from above the circle has the front up and the right on the right", () => {
    near(project({ x: 0, y: 1, z: 0 }, FROM_ABOVE).up, 1);
    near(project({ x: 1, y: 0, z: 0 }, FROM_ABOVE).right, 1);
    near(project({ x: 0, y: 0, z: 1 }, FROM_ABOVE).towards, 1);
  });

  it("first shows the sphere from behind the listener", () => {
    expect(project({ x: 0, y: 1, z: 0 }, FIRST_VIEW).towards).toBeLessThan(0);
    expect(project({ x: 0, y: 0, z: 1 }, FIRST_VIEW).up).toBeGreaterThan(0);
  });

  it("keeps the vertical axis vertical however the sphere is turned", () => {
    for (const yaw of [0, 0.7, 2, -3]) {
      for (const pitch of [-3, -1.5, -0.4, 0, 0.9, 1.57, 2.5]) {
        near(project({ x: 0, y: 0, z: 1 }, view(yaw, pitch)).right, 0);
      }
    }
  });

  it("keeps sizes: nothing stretches as it turns", () => {
    const place = { x: 1, y: 2, z: 3 };
    const { right, up, towards } = project(place, view(1.1, 0.6));
    near(Math.hypot(right, up, towards), distanceOf(place));
  });
});

describe("turning", () => {
  it("goes all the way round in both directions", () => {
    let v = view(0, 0.45);
    for (let i = 0; i < 40; i++) v = turned(v, 0.5, 0.5);
    expect(Math.abs(v.yaw)).toBeLessThanOrEqual(Math.PI);
    expect(Math.abs(v.pitch)).toBeLessThanOrEqual(Math.PI);
  });

  it("moves the surface in front with the cursor", () => {
    const before = view(0.4, 0.5);
    const surface = (v: SpaceView) => {
      // the point of the sphere facing whoever looks
      const front = { x: Math.sin(before.yaw) * -Math.cos(before.pitch), y: -Math.cos(before.yaw) * Math.cos(before.pitch), z: Math.sin(before.pitch) };
      return project(front, v);
    };
    expect(surface(turned(before, 0.01, 0)).right).toBeGreaterThan(0);
    expect(surface(turned(before, 0, 0.01)).up).toBeLessThan(0);
  });

  it("still follows the cursor upside down", () => {
    const before = view(0.4, 2.5);
    expect(upsideDown(before)).toBe(true);
    const front = { x: Math.sin(0.4) * -Math.cos(2.5), y: -Math.cos(0.4) * Math.cos(2.5), z: Math.sin(2.5) };
    expect(project(front, turned(before, 0.01, 0)).right).toBeGreaterThan(0);
  });

  it("doesn't change the kind of view", () => {
    expect(turned(view(0, 0), 1, 1).solid).toBe(true);
  });
});

describe("moving a place", () => {
  it("in the circle changes only where it is around the listener, not its height", () => {
    const moved = moveOnNode({ x: 1, y: 2, z: 3 }, 1, -1, FROM_ABOVE, 10);
    near(moved.x, 2);
    near(moved.y, 1);
    near(moved.z, 3);
  });

  it("on the sphere stays in the node's plane", () => {
    const v = view(0.8, 0.35);
    const from = { x: 2, y: -1, z: 1.5 };
    const moved = moveOnNode(from, 1.5, 0.75, v, 10);
    near(project(moved, v).towards, project(from, v).towards);
    near(project(moved, v).right, project(from, v).right + 1.5);
    near(project(moved, v).up, project(from, v).up + 0.75);
  });

  it("stops at the limit along the edge, still in the plane", () => {
    const v = view(0.8, 0.35);
    const from = { x: 2, y: 3, z: 1 };
    const moved = moveOnNode(from, 100, 40, v, 5);
    near(distanceOf(moved), 5);
    near(project(moved, v).towards, project(from, v).towards);
  });

  it("in the circle keeps its height at the edge", () => {
    const moved = moveOnNode({ x: 0, y: 1, z: 3 }, 100, 0, FROM_ABOVE, 5);
    near(moved.z, 3);
    near(distanceOf(moved), 5);
  });

  it("doesn't reach the middle by dragging", () => {
    const from = { x: 3, y: 0, z: 0 };
    expect(distanceOf(moveOnNode(from, -3, 0, FROM_ABOVE, 10))).toBeGreaterThan(0);
    const close = moveOnNode(from, -2.9999, 0, FROM_ABOVE, 10);
    near(distanceOf(close), 0.1);
    expect(close.x).toBeGreaterThan(0);
  });

  it("leaves the middle in any direction", () => {
    const moved = moveOnNode({ x: 0, y: 0, z: 0 }, -2, 1, FROM_ABOVE, 10);
    near(moved.x, -2);
    near(moved.y, 1);
    expect(distanceOf(moveOnNode({ x: 0, y: 0, z: 0 }, 0, 0, FROM_ABOVE, 10))).toBeGreaterThan(0);
  });
});

describe("angles", () => {
  it("count from the front, plus to the right", () => {
    near(azimuth({ x: 0, y: 1, z: 0 }), 0);
    near(azimuth({ x: 1, y: 0, z: 0 }), 90);
    near(azimuth({ x: -1, y: 0, z: 0 }), -90);
    near(Math.abs(azimuth({ x: 0, y: -1, z: 0 })), 180);
    near(elevation({ x: 1, y: 0, z: 1 }), 45);
    near(elevation({ x: 0, y: 0, z: -2 }), -90);
  });

  it("typing the azimuth keeps the distance and the height", () => {
    const place = atAzimuth({ x: 3, y: 4, z: 2 }, -120);
    near(azimuth(place), -120);
    near(place.z, 2);
    near(Math.hypot(place.x, place.y), 5);
  });

  it("typing the elevation keeps the distance and the azimuth", () => {
    const from = { x: 3, y: 4, z: 2 };
    const place = atElevation(from, 30);
    near(elevation(place), 30);
    near(azimuth(place), azimuth(from));
    near(distanceOf(place), distanceOf(from));
  });

  it("a source taken straight up comes back in its direction", () => {
    const from = { x: 3, y: 3, z: 0 };
    near(azimuth(atElevation(atElevation(from, 90), 0)), 45);
  });

  it("an elevation past the pole stops at it", () => {
    near(elevation(atElevation({ x: 1, y: 0, z: 0 }, 200)), 90);
    near(elevation(atElevation({ x: 1, y: 0, z: 0 }, -200)), -90);
  });
});

describe("distance", () => {
  it("a lowered limit pulls a place in", () => {
    const place = withinLimit({ x: 0, y: 6, z: 8 }, 5);
    near(place.y, 3);
    near(place.z, 4);
    const inside = { x: 1, y: 1, z: 1 };
    expect(withinLimit(inside, 5)).toBe(inside);
  });

  it("quiets beyond a metre, as in the engine", () => {
    expect(distanceGain(null, 10)).toBe(1);
    expect(distanceGain({ x: 0, y: 0.5, z: 0 }, 10)).toBe(1);
    expect(distanceGain({ x: 0, y: 2, z: 0 }, 10)).toBe(0.5);
    expect(distanceGain({ x: 0, y: 20, z: 0 }, 10)).toBe(0.1);
  });
});

describe("the handles of a source", () => {
  const side = view(0.5, 0.4);
  const from = { x: 1, y: 2, z: 2 };
  const LIMIT = 10;

  /** Where a place shows on the node */
  const onNode = (place: Place, v: SpaceView = side) => {
    const { right, up } = project(place, v);
    return { right, up };
  };
  /** A spot on the node, so far along a handle's way from a place */
  const along = (place: Place, way: Place, length: number, v: SpaceView = side) => {
    const [at, by] = [onNode(place, v), onNode(way, v)];
    return { right: at.right + by.right * length, up: at.up + by.up * length };
  };
  /** A press on the arrow of a handle, as far from the source as its head is */
  const pressed = (handle: Handle, place: Place, v: SpaceView) => {
    const frame = frameAt(place);
    return along(place, handle === "distance" ? frame.out : handle === "parallel" ? frame.along : frame.up, 0.3, v);
  };
  const held = (handle: Handle, place: Place | null, v: SpaceView = side) => grab(handle, place, pressed(handle, place ?? { x: 0, y: 0, z: 0 }, v), v);
  const drag = (handle: Handle, place: Place | null, to: OnNode, v: SpaceView = side, limit = LIMIT) => dragged(held(handle, place, v), to, v, limit).place;
  const listener = { right: 0, up: 0 };
  /** Whether a spot lies on the ray from a spot through another */
  const onRay = (from: OnNode, through: OnNode, spot: OnNode) => {
    const [a, b] = [
      { right: through.right - from.right, up: through.up - from.up },
      { right: spot.right - from.right, up: spot.up - from.up },
    ];
    near((a.right * b.up - a.up * b.right) / Math.hypot(a.right, a.up) / Math.hypot(b.right, b.up), 0);
    expect(a.right * b.right + a.up * b.up).toBeGreaterThan(0);
  };

  it("point out of the listener, along the parallel and up the meridian", () => {
    const { out, along, up } = frameAt({ x: 0, y: 3, z: 0 });
    expect(out).toEqual({ x: 0, y: 1, z: 0 });
    near(along.x, 1);
    near(up.z, 1);
    const right = frameAt({ x: 3, y: 0, z: 0 });
    near(right.along.y, -1);
    near(right.up.z, 1);
  });

  it("stand at right angles to each other wherever the source is", () => {
    for (const place of [from, { x: -3, y: -1, z: 0.5 }, { x: 0.1, y: 0, z: -5 }]) {
      const { out, along, up } = frameAt(place);
      near(distanceOf(out), 1);
      near(distanceOf(along), 1);
      near(distanceOf(up), 1);
      near(out.x * along.x + out.y * along.y + out.z * along.z, 0);
      near(out.x * up.x + out.y * up.y + out.z * up.z, 0);
      near(along.x * up.x + along.y * up.y + along.z * up.z, 0);
    }
  });

  it("start out to the front from the middle", () => {
    expect(frameAt(null).out).toEqual({ x: 0, y: 1, z: 0 });
  });

  describe("the distance", () => {
    const { out } = frameAt(from);
    const start = pressed("distance", from, side);
    const pull = (metres: number) => along(from, out, 0.3 + metres);

    it("moves the source along its line and nothing else", () => {
      const moved = drag("distance", from, pull(2));
      near(distanceOf(moved), 5);
      near(azimuth(moved), azimuth(from));
      near(elevation(moved), elevation(from));
    });

    it("ignores a drag across it", () => {
      const way = onNode(out);
      near(distanceOf(drag("distance", from, { right: start.right - way.up, up: start.up + way.right })), distanceOf(from));
    });

    it("is a metre on the node for a metre of the way", () => {
      const v = view(0, Math.PI / 2);
      const place = { x: 0, y: 5, z: 0 };
      near(distanceOf(dragged(grab("distance", place, { right: 0, up: 5.3 }, v), { right: 0, up: 6.3 }, v, LIMIT).place), 6);
    });

    it("stops at the limit and short of the middle", () => {
      near(distanceOf(drag("distance", from, pull(100))), LIMIT);
      expect(distanceOf(drag("distance", from, pull(-100)))).toBeGreaterThan(0);
    });

    it("doesn't lose what went past the limit", () => {
      const past = dragged(held("distance", from), pull(100), side, LIMIT);
      near(distanceOf(dragged(past, pull(1), side, LIMIT).place), 4);
    });

    it("starts from the middle to the front", () => {
      const v = view(0, Math.PI / 2);
      near(dragged(grab("distance", null, { right: 0, up: 0.3 }, v), { right: 0, up: 2.3 }, v, LIMIT).place.y, 2);
    });
  });

  describe("the parallel and the meridian", () => {
    const FAR = 40;
    const rays = [0, 0.7, 1.6, 2.5, 3.1, 4, 5.2, 6];
    const ray = (angle: number, length = FAR) => ({ right: length * Math.cos(angle), up: length * Math.sin(angle) });
    const views = [view(0.5, 0.4), view(1.4, 0.8), view(-2, 1.2), view(0.6, -0.7), view(2.6, 2.3), view(-1.1, 0.3), view(2, -0.4), view(-1.4, 1)];
    const plus = (a: OnNode, b: OnNode) => ({ right: a.right + b.right, up: a.up + b.up });
    /** Views where a handle's ring shows open, not as a line */
    const openOn = (handle: Handle, place: Place) => {
      const shown = views.filter((v) => held(handle, place, v).ring!.open > 0.3);
      expect(shown.length).toBeGreaterThanOrEqual(3);
      return shown;
    };

    it("change only the azimuth, or only the elevation", () => {
      for (const v of views) {
        for (const angle of rays) {
          const round = drag("parallel", from, ray(angle), v);
          near(elevation(round), elevation(from));
          near(distanceOf(round), distanceOf(from));
          const over = drag("meridian", from, ray(angle), v);
          near(distanceOf(over), distanceOf(from));
          // over a pole the azimuth turns to the other side of the same circle
          const turn = Math.abs(azimuth(over) - azimuth(from));
          expect(Math.min(turn, Math.abs(turn - 180), Math.abs(turn - 360))).toBeLessThan(1e-6);
        }
      }
    });

    it("go where the arrow points", () => {
      const round = along(from, frameAt(from).along, 0.8);
      expect(azimuth(drag("parallel", from, round))).toBeGreaterThan(azimuth(from));
      const up = along(from, frameAt(from).up, 0.8);
      expect(elevation(drag("meridian", from, up))).toBeGreaterThan(elevation(from));
      const down = along(from, frameAt(from).up, -0.8);
      expect(elevation(drag("meridian", from, down))).toBeLessThan(elevation(from));
    });

    it("stand where the ray from the listener to the cursor crosses a meridian", () => {
      for (const v of openOn("meridian", from)) {
        for (const angle of rays) {
          const to = ray(angle);
          onRay(listener, onNode(drag("meridian", from, to, v), v), to);
        }
      }
    });

    it("stand where the ray from the listener to the cursor crosses the equator", () => {
      const level = { x: 3, y: 4, z: 0 };
      for (const v of views) {
        for (const angle of rays) {
          const to = ray(angle);
          onRay(listener, onNode(drag("parallel", level, to, v), v), to);
        }
      }
    });

    it("stand on the ray from the middle of a parallel off the equator, wherever the cursor is", () => {
      for (const [place, v] of [
        // seen from higher than it lies, the parallel shows around the listener
        [{ x: 2, y: 3, z: 1 }, view(1.4, 0.8)],
        [{ x: 2, y: 3, z: 1 }, view(-2, 1.2)],
        // and from lower, wholly away from it, so a ray from the listener could miss it
        [{ x: 1, y: 3, z: 6 }, view(0, 0.45)],
        [{ x: -2, y: 1, z: -4 }, view(2, 0.5)],
      ] as const) {
        const middle = onNode({ x: 0, y: 0, z: place.z }, v);
        for (const angle of rays) {
          for (const length of [0.5, 3, FAR]) {
            const to = plus(middle, ray(angle, length));
            const moved = drag("parallel", place, to, v);
            onRay(middle, onNode(moved, v), to);
            near(moved.z, place.z);
            near(distanceOf(moved), distanceOf(place));
          }
        }
      }
    });

    it("come right under a cursor that is on the circle", () => {
      const level = Math.hypot(from.x, from.y);
      const rings = {
        parallel: (a: number) => ({ x: level * Math.sin(a), y: level * Math.cos(a), z: from.z }),
        meridian: (a: number) => {
          const [across, up] = [3 * Math.cos(a), 3 * Math.sin(a)];
          return { x: (from.x / level) * across, y: (from.y / level) * across, z: up };
        },
      };
      for (const handle of ["parallel", "meridian"] as const) {
        for (const v of openOn(handle, from)) {
          const grabbed = held(handle, from, v);
          for (let degrees = 0; degrees < 360; degrees += 15) {
            const spot = onNode(rings[handle]((degrees * Math.PI) / 180), v);
            const on = onNode(dragged(grabbed, spot, v, LIMIT).place, v);
            near(on.right, spot.right);
            near(on.up, spot.up);
          }
        }
      }
    });

    it("go onto the ray from the first move, wherever the arrow was taken", () => {
      for (const v of openOn("meridian", from)) {
        const start = pressed("meridian", from, v);
        const to = { right: start.right + 0.01, up: start.up };
        onRay(listener, onNode(drag("meridian", from, to, v), v), to);
      }
    });

    it("get nowhere from a cursor circling to the side", () => {
      for (const handle of ["parallel", "meridian"] as const) {
        for (const v of views) {
          const aside = { right: 15, up: 4 };
          const first = dragged(held(handle, from, v), plus(aside, ray(0, 2)), v, LIMIT);
          let now = first;
          for (let turn = 0; turn < 5; turn++) {
            for (let degrees = 10; degrees <= 360; degrees += 10) now = dragged(now, plus(aside, ray((degrees * Math.PI) / 180, 2)), v, LIMIT);
            // after every circle the source is where it was before it
            near(now.place.x, first.place.x);
            near(now.place.y, first.place.y);
            near(now.place.z, first.place.z);
          }
        }
      }
    });

    it("come back as the cursor comes back, wherever it went", () => {
      for (const handle of ["parallel", "meridian"] as const) {
        const grabbed = held(handle, from);
        let now = dragged(grabbed, ray(1), side, LIMIT);
        const there = now.place;
        for (const spot of [ray(2), ray(4), ray(5.5), { right: 0.5, up: 0.5 }]) now = dragged(now, spot, side, LIMIT);
        now = dragged(now, ray(1), side, LIMIT);
        near(now.place.x, there.x);
        near(now.place.y, there.y);
        near(now.place.z, there.z);
      }
    });

    it("go over the pole along the meridian", () => {
      // seen from the side the meridian is a full circle on the node
      const v = view(Math.PI / 2, 0);
      const start = { x: 0, y: 3, z: 0 };
      let now = held("meridian", start, v);
      for (let degrees = 5; degrees <= 170; degrees += 5) {
        const angle = (degrees * Math.PI) / 180;
        now = dragged(now, { right: -3 * Math.cos(angle), up: 3 * Math.sin(angle) }, v, LIMIT);
      }
      near(distanceOf(now.place), 3);
      expect(Math.abs(azimuth(now.place))).toBeGreaterThan(170);
      expect(elevation(now.place)).toBeCloseTo(10, 0);
    });

    it("stay in the same circle over the pole, even from the pole itself", () => {
      const v = view(Math.PI / 2, 0);
      const moved = drag("meridian", { x: 0, y: 0, z: 3 }, { right: 2, up: 2 }, v);
      near(moved.x, 0);
      near(distanceOf(moved), 3);
    });

    it("hold still while the cursor is in the very middle of the ring", () => {
      for (const v of openOn("meridian", from)) {
        const out = dragged(held("meridian", from, v), ray(1), v, LIMIT);
        expect(dragged(out, { right: 0, up: 0 }, v, LIMIT).place).toEqual(out.place);
      }
    });

    it("show the ring they go along", () => {
      const { ring } = held("parallel", from);
      const places = ringPlaces(ring!);
      expect(places.length).toBeGreaterThan(30);
      for (const place of places) {
        near(place.z, from.z);
        near(distanceOf(place), distanceOf(from));
      }
      for (const place of ringPlaces(held("meridian", from).ring!)) near(distanceOf(place), distanceOf(from));
    });

    it("have nothing to do from the middle", () => {
      const v = view(0, 0.5);
      for (const handle of ["parallel", "meridian"] as const) {
        expect(dragged(grab(handle, null, { right: 1, up: 1 }, v), { right: 5, up: 2 }, v, LIMIT).place).toEqual({ x: 0, y: 0, z: 0 });
      }
    });

    it("survive a pole, where a parallel has no length", () => {
      const moved = drag("parallel", { x: 0, y: 0, z: 5 }, { right: 1, up: 1 });
      expect(Number.isFinite(moved.x) && Number.isFinite(moved.y)).toBe(true);
      near(distanceOf(moved), 5);
    });

    it("survive a ring seen straight on", () => {
      const above = view(0.3, Math.PI / 2);
      const moved = drag("parallel", { x: 3, y: 4, z: 0 }, { right: 0, up: 9 }, above);
      near(distanceOf(moved), 5);
      onRay(listener, onNode(moved, above), { right: 0, up: 9 });
    });
  });

  describe("a ring seen edge-on", () => {
    // from behind the listener the meridian of a source in front is a line going up the node
    const v = view(0, 0.45);
    const front = { x: 0, y: 3, z: 0 };

    it("is a line", () => {
      expect(held("meridian", front, v).ring!.open).toBeLessThan(0.01);
    });

    it("takes the source level with the cursor, however far to the side it is", () => {
      for (const aside of [0, 1, -2, 9, -30]) {
        const moved = drag("meridian", front, { right: aside, up: -1.5 }, v);
        near(onNode(moved, v).up, -1.5);
        near(moved.x, 0);
        near(distanceOf(moved), 3);
      }
    });

    it("keeps the source on the half it is on", () => {
      expect(drag("meridian", front, { right: 3, up: 2 }, v).y).toBeGreaterThan(0);
      expect(drag("meridian", { x: 0, y: -3, z: 0 }, { right: -3, up: 2 }, v).y).toBeLessThan(0);
    });

    it("stops the source at the end of the line", () => {
      const up = onNode({ x: 0, y: 0, z: 3 }, v);
      const moved = drag("meridian", front, { right: 0, up: 30 }, v);
      near(project(moved, v).right, 0);
      expect(project(moved, v).up).toBeGreaterThan(up.up);
      near(distanceOf(moved), 3);
    });

    it("doesn't break exactly along a parallel seen from the side", () => {
      const level = view(0.7, 0);
      const moved = drag("parallel", { x: 3, y: 4, z: 2 }, { right: -2, up: 30 }, level);
      near(onNode(moved, level).right, -2);
      near(moved.z, 2);
    });
  });
});

describe("typing a distance", () => {
  it("keeps the direction", () => {
    const place = atDistance({ x: 3, y: 4, z: 0 }, 2.5, 10);
    near(distanceOf(place), 2.5);
    near(azimuth(place), azimuth({ x: 3, y: 4, z: 0 }));
  });

  it("stays within the limit and out of the middle", () => {
    near(distanceOf(atDistance({ x: 0, y: 1, z: 0 }, 50, 10)), 10);
    expect(distanceOf(atDistance({ x: 0, y: 1, z: 0 }, 0, 10))).toBeGreaterThan(0);
  });
});

describe("a 3D arrow head", () => {
  const at = { x: 100, y: 100 };

  it("shows its side faces and no base when it points away from the base's side", () => {
    // seen from above, pointing straight out of the node
    const faces = coneFaces(at, { x: 0, y: 0, z: 1 }, 10, 4, FROM_ABOVE);
    expect(faces.length).toBe(12);
    for (const face of faces) expect(face.points.length).toBe(3);
  });

  it("shows only its base when it points straight away", () => {
    const faces = coneFaces(at, { x: 0, y: 0, z: -1 }, 10, 4, FROM_ABOVE);
    expect(faces.length).toBe(1);
    expect(faces[0].points.length).toBe(12);
  });

  it("lying along the node shows half of itself, tip where it points", () => {
    const faces = coneFaces(at, { x: 1, y: 0, z: 0 }, 10, 4, FROM_ABOVE);
    expect(faces.length).toBe(6);
    near(faces[0].points[0][0], 110);
    near(faces[0].points[0][1], 100);
  });

  it("points up the drawing when it points up the node", () => {
    const faces = coneFaces(at, { x: 0, y: 1, z: 0 }, 10, 4, FROM_ABOVE);
    near(faces[0].points[0][1], 90);
  });

  it("is lit unevenly, so it reads as round", () => {
    const light = coneFaces(at, { x: 1, y: 0, z: 0 }, 10, 4, FROM_ABOVE).map((f) => f.light);
    expect(Math.max(...light) - Math.min(...light)).toBeGreaterThan(0.2);
  });

  it("keeps its shape however it is turned", () => {
    for (const v of [view(0.3, 0.2), view(2, -1), view(-1, 2.5)]) {
      const faces = coneFaces(at, { x: 0.6, y: 0.8, z: 0 }, 10, 4, v);
      expect(faces.length).toBeGreaterThan(0);
      for (const f of faces) for (const [x, y] of f.points) expect(Math.hypot(x - at.x, y - at.y)).toBeLessThanOrEqual(Math.hypot(10, 4));
    }
  });
});

describe("the values by the point", () => {
  const size = { width: 60, height: 14 };
  const inside = (s: { x: number; y: number }) => {
    expect(s.x).toBeGreaterThanOrEqual(0);
    expect(s.y).toBeGreaterThanOrEqual(0);
    expect(s.x + size.width).toBeLessThanOrEqual(200);
    expect(s.y + size.height).toBeLessThanOrEqual(200);
  };

  it("stand on the side away from the middle", () => {
    const spot = labelSpot({ x: 130, y: 70 }, size, 200, [{ x: 100, y: 100 }]);
    expect(spot.x).toBeGreaterThan(130);
    expect(spot.y).toBeLessThan(70);
    inside(spot);
  });

  it("stay inside the drawing at its edge", () => {
    for (const point of [{ x: 190, y: 100 }, { x: 10, y: 100 }, { x: 100, y: 5 }, { x: 100, y: 195 }, { x: 195, y: 195 }]) {
      inside(labelSpot(point, size, 200, []));
    }
  });

  it("don't cover an arrow if there is room", () => {
    const point = { x: 130, y: 70 };
    const tip = { x: 160, y: 60 };
    const spot = labelSpot(point, size, 200, [tip]);
    expect(spot.x + size.width < tip.x || spot.x > tip.x || spot.y > tip.y || spot.y + size.height < tip.y).toBe(true);
  });

  it("don't cover the listener", () => {
    const listener = { x: 100, y: 100 };
    const spot = labelSpot({ x: 104, y: 98 }, size, 200, [listener]);
    const covers = listener.x >= spot.x && listener.x <= spot.x + size.width && listener.y >= spot.y && listener.y <= spot.y + size.height;
    expect(covers).toBe(false);
  });
});

it("a graph reaches a metre by default", () => {
  expect(DEFAULT_DISTANCE_LIMIT).toBe(1);
});
