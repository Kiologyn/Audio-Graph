/** Where a sound is around the listener, in metres: x to the right, y to the front, z up. Matches Place in graphs.rs */
export type Place = { x: number; y: number; z: number };

/** How a place is looked at on the node. Matches SpaceView in graphs.rs */
export type SpaceView = {
  /** A sphere that turns, otherwise a circle seen from above */
  solid: boolean;
  /** Radians around the vertical axis */
  yaw: number;
  /** Radians of looking down on it: 0 is from behind the listener, a quarter turn from straight above */
  pitch: number;
};

/** The graph's default for how far sound can be; matches graphs.rs */
export const DEFAULT_DISTANCE_LIMIT = 1;
/** The widest distance limit of a graph; matches DISTANCE_LIMITS in graphs.rs */
export const MAX_DISTANCE_LIMIT = 1000;

/** A first look at the sphere: from behind the listener and a bit above */
export const FIRST_VIEW: SpaceView = { solid: false, yaw: 0, pitch: 0.45 };

/** The circle is the sphere seen from straight above */
export const FROM_ABOVE: SpaceView = { solid: false, yaw: 0, pitch: Math.PI / 2 };

/** Up to this distance a source plays at its full volume; matches NEAR in spatial.rs */
const NEAR = 1;

/** Sound in the very middle has no direction: dragging stops this share of the limit short of it */
const NEAREST = 0.01;

/** The handles of a source: its distance, and along its parallel and meridian from where it is */
export type Handle = "distance" | "parallel" | "meridian";

/** A spot of the node, from the listener, in metres */
export type OnNode = { right: number; up: number };

/** The circle a handle takes a source around, as it shows on the node */
export type Ring = {
  middle: Place;
  radius: number;
  /** The way of the circle that lies along the node, so it shows at its full length */
  wide: Place;
  /** The way of the circle that turns towards whoever looks, so only a part of it shows */
  deep: Place;
  /** How open the circle shows: 1 straight on, 0 edge-on */
  open: number;
};

/** A handle being dragged */
export type Grab = {
  handle: Handle;
  /** The source when the handle was pressed */
  from: Place;
  /** The source now */
  place: Place;
  press: OnNode;
  /** The circle of a parallel or a meridian; the distance goes along a line */
  ring: Ring | null;
  /** The half of a ring seen edge-on that the source stays on */
  side: number;
};

/** A ring open less than this is nearly a line: almost any ray from its middle crosses it in the middle */
const EDGE_ON = 0.2;

/** A handle seen end-on moves nothing on the node: what it can take from a drag ends here, or it would race */
const LEAST_SEEN = 0.25;

const RING_POINTS = 72;

const SHINE = { right: -0.45, up: 0.6, towards: 0.66 };
const CONE_SIDES = 12;

const TURN = 2 * Math.PI;
const DEGREES = 180 / Math.PI;

export const distanceOf = ({ x, y, z }: Place) => Math.hypot(x, y, z);

/** A place within the graph's distance: a limit lowered later acts on what was set before, in the engine too */
export function withinLimit(place: Place, limit: number): Place {
  const distance = distanceOf(place);
  if (distance <= limit) return place;
  const share = limit / distance;
  return { x: place.x * share, y: place.y * share, z: place.z * share };
}

/** How a place's distance changes the volume: half as loud for twice as far */
export function distanceGain(place: Place | null, limit: number): number {
  if (!place || distanceOf(place) === 0) return 1;
  return NEAR / Math.max(NEAR, Math.min(distanceOf(place), limit));
}

/** The directions on the node in space: to the right of it, up it and out of it, towards whoever looks */
function axes({ yaw, pitch }: Pick<SpaceView, "yaw" | "pitch">) {
  const [ys, yc, ps, pc] = [Math.sin(yaw), Math.cos(yaw), Math.sin(pitch), Math.cos(pitch)];
  return {
    right: { x: yc, y: -ys, z: 0 },
    up: { x: ys * ps, y: yc * ps, z: pc },
    towards: { x: -ys * pc, y: -yc * pc, z: ps },
  };
}

const dot = (a: Place, b: Place) => a.x * b.x + a.y * b.y + a.z * b.z;
const times = ({ x, y, z }: Place, share: number) => ({ x: x * share, y: y * share, z: z * share });
const cross = (a: Place, b: Place) => ({ x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x });

/** Where a place is on the node, in the same units. Nothing turns the vertical axis sideways: it has no right */
export function project(place: Place, view: Pick<SpaceView, "yaw" | "pitch">) {
  const { right, up, towards } = axes(view);
  return { right: dot(place, right), up: dot(place, up), towards: dot(place, towards) };
}

/**
 * A place moved along the node's plane by this much, in metres. It stays in that plane: at the limit it goes
 * along the edge of the slice of the sphere, and short of the middle, which is only reached by turning it off
 */
export function moveOnNode(from: Place, right: number, up: number, view: Pick<SpaceView, "yaw" | "pitch">, limit: number): Place {
  const start = project(from, view);
  const inside = Math.sqrt(Math.max(0, limit * limit - start.towards * start.towards));
  const [wantedRight, wantedUp] = [start.right + right, start.up + up];
  const across = Math.hypot(wantedRight, wantedUp);
  const share = across > inside ? inside / across : 1;
  const [r, u] = [wantedRight * share, wantedUp * share];
  const axis = axes(view);
  const along = (name: "x" | "y" | "z") => axis.right[name] * r + axis.up[name] * u + axis.towards[name] * start.towards;
  const moved = { x: along("x"), y: along("y"), z: along("z") };

  const distance = distanceOf(moved);
  const nearest = limit * NEAREST;
  if (distance >= nearest) return moved;
  if (distance === 0) return distanceOf(from) > 0 ? from : { x: 0, y: nearest, z: 0 };
  const grown = nearest / distance;
  return { x: moved.x * grown, y: moved.y * grown, z: moved.z * grown };
}

/**
 * The directions at a place: out of the listener, along its parallel to the right as seen from above, and up its
 * meridian. Sound in the middle goes out to the front
 */
export function frameAt(place: Place | null) {
  const distance = place ? distanceOf(place) : 0;
  const out = place && distance > 0 ? times(place, 1 / distance) : { x: 0, y: 1, z: 0 };
  const angle = Math.atan2(out.x, out.y);
  const along = { x: Math.cos(angle), y: -Math.sin(angle), z: 0 };
  return { out, along, up: cross(along, out) };
}

/** The parallel or meridian through a place, seen from a view */
function ringOf(handle: Handle, place: Place, view: Pick<SpaceView, "yaw" | "pitch">): Ring {
  const frame = frameAt(place);
  const parallel = handle === "parallel";
  const square = parallel ? { x: 0, y: 0, z: 1 } : frame.along;
  const { towards } = axes(view);
  const level = cross(square, towards);
  const length = distanceOf(level);
  // seen straight on, every way along it shows in full
  const wide = length > 1e-9 ? times(level, 1 / length) : parallel ? frame.along : frame.out;
  const deep = cross(square, wide);
  return {
    middle: { x: 0, y: 0, z: parallel ? place.z : 0 },
    radius: parallel ? Math.hypot(place.x, place.y) : distanceOf(place),
    wide,
    deep: dot(deep, towards) < 0 ? times(deep, -1) : deep,
    open: Math.abs(dot(square, towards)),
  };
}

function onRing({ middle, radius, wide, deep }: Ring, angle: number): Place {
  const [across, depth] = [radius * Math.cos(angle), radius * Math.sin(angle)];
  return {
    x: middle.x + wide.x * across + deep.x * depth,
    y: middle.y + wide.y * across + deep.y * depth,
    z: middle.z + wide.z * across + deep.z * depth,
  };
}

/** The places along a ring, to draw it */
export const ringPlaces = (ring: Ring) => Array.from({ length: RING_POINTS + 1 }, (_, i) => onRing(ring, (i / RING_POINTS) * TURN));

/**
 * The angle on a ring that a spot of the node points to: where the ray from the middle of the ring as it shows through
 * the spot crosses it. Nearly edge-on a ring is almost a line, which such a ray crosses only in the middle: there the
 * source goes level with the spot, on its own half. In the very middle a ray points nowhere
 */
function aimed(ring: Ring, side: number, spot: OnNode, view: Pick<SpaceView, "yaw" | "pitch">, limit: number): number | null {
  const middle = project(ring.middle, view);
  const [right, up] = [spot.right - middle.right, spot.up - middle.up];
  const wide = project(ring.wide, view);
  const across = right * wide.right + up * wide.up;
  if (ring.open < EDGE_ON) return side * Math.acos(Math.max(-1, Math.min(1, across / ring.radius)));
  if (Math.hypot(right, up) < limit * NEAREST) return null;
  const deep = project(ring.deep, view);
  // the ring shows squeezed along its deep way: undone, the ray's angle is the angle on the ring
  return Math.atan2((right * deep.right + up * deep.up) / ring.open ** 2, across);
}

/** A drag of a handle starts here, with a press on this spot of the node. From the middle only the distance has a way to go */
export function grab(handle: Handle, from: Place | null, press: OnNode, view: Pick<SpaceView, "yaw" | "pitch">): Grab {
  const place = from ?? { x: 0, y: 0, z: 0 };
  const held = { handle, from: place, place, press };
  if (handle === "distance") return { ...held, ring: null, side: 1 };
  const ring = ringOf(handle, place, view);
  const offset = { x: place.x - ring.middle.x, y: place.y - ring.middle.y, z: place.z - ring.middle.z };
  return { ...held, ring, side: dot(offset, ring.deep) < 0 ? -1 : 1 };
}

/**
 * A handle dragged to this spot of the node. It all comes from where the cursor is, never from the way it went:
 * circling it somewhere gets the source nowhere, and it comes back as the cursor comes back
 */
export function dragged(held: Grab, spot: OnNode, view: Pick<SpaceView, "yaw" | "pitch">, limit: number): Grab {
  if (!held.ring) {
    // the distance keeps the cursor on the arrow it took
    const [right, up] = [spot.right - held.press.right, spot.up - held.press.up];
    const { out } = frameAt(held.from);
    const seen = project(out, view);
    const moved = (right * seen.right + up * seen.up) / Math.max(seen.right ** 2 + seen.up ** 2, LEAST_SEEN ** 2);
    return { ...held, place: times(out, Math.max(limit * NEAREST, Math.min(limit, distanceOf(held.from) + moved))) };
  }
  // at a pole a parallel has no length, there is nothing to go along
  if (held.ring.radius < limit * 1e-6) return held;
  const angle = aimed(held.ring, held.side, spot, view, limit);
  return angle === null ? held : { ...held, place: onRing(held.ring, angle) };
}

/** A cone's side as it shows on the drawing, y going down. Light goes from 0 to 1 */
export type Face = { points: [number, number][]; light: number };

/**
 * The faces of a cone standing on a point of the drawing and pointing along an axis of space: only those
 * turned to whoever looks, lit from the top left
 */
export function coneFaces(at: { x: number; y: number }, axis: Place, height: number, radius: number, view: Pick<SpaceView, "yaw" | "pitch">): Face[] {
  const seen = project(axis, view);
  const u = { x: seen.right, y: seen.up, z: seen.towards };
  const side = cross(u, Math.abs(u.z) < 0.9 ? { x: 0, y: 0, z: 1 } : { x: 1, y: 0, z: 0 });
  const b = times(side, 1 / distanceOf(side));
  const c = cross(u, b);
  const around = (angle: number) => ({
    x: b.x * Math.cos(angle) + c.x * Math.sin(angle),
    y: b.y * Math.cos(angle) + c.y * Math.sin(angle),
    z: b.z * Math.cos(angle) + c.z * Math.sin(angle),
  });
  const onDrawing = ({ x, y }: Place): [number, number] => [at.x + x, at.y - y];
  const rim = (i: number) => onDrawing(times(around((i / CONE_SIDES) * TURN), radius));
  const tip = onDrawing(times(u, height));
  const slant = Math.hypot(height, radius);
  const lit = (normal: Place) => 0.35 + 0.65 * Math.max(0, normal.x * SHINE.right + normal.y * SHINE.up + normal.z * SHINE.towards);

  const faces: Face[] = [];
  for (let i = 0; i < CONE_SIDES; i++) {
    const out = around(((i + 0.5) / CONE_SIDES) * TURN);
    const normal = {
      x: (out.x * height + u.x * radius) / slant,
      y: (out.y * height + u.y * radius) / slant,
      z: (out.z * height + u.z * radius) / slant,
    };
    if (normal.z > 0) faces.push({ points: [tip, rim(i), rim(i + 1)], light: lit(normal) });
  }
  if (u.z < 0) faces.push({ points: Array.from({ length: CONE_SIDES }, (_, i) => rim(i)), light: lit(times(u, -1)) });
  return faces;
}

type Spot = { x: number; y: number };

/** How far a spot is from a box */
function clearance(spot: Spot, box: Spot & { width: number; height: number }) {
  const across = Math.max(box.x - spot.x, 0, spot.x - box.x - box.width);
  const down = Math.max(box.y - spot.y, 0, spot.y - box.y - box.height);
  return Math.hypot(across, down);
}

/**
 * Where a label goes by a point of a square drawing: inside it, on the side away from the middle if it can,
 * clear of what it mustn't cover, or as clear as it gets
 */
export function labelSpot(point: Spot, size: { width: number; height: number }, drawing: number, avoid: Spot[]): Spot {
  const gap = 11;
  const enough = 8;
  const { width, height } = size;
  const [right, left, middle] = [point.x + gap, point.x - gap - width, point.x - width / 2];
  const [above, below] = [point.y - gap - height, point.y + gap];
  const [near, far] = point.x >= drawing / 2 ? [right, left] : [left, right];
  const [first, second] = point.y < drawing / 2 ? [above, below] : [below, above];
  const inside = (value: number, room: number) => Math.max(0, Math.min(drawing - room, value));
  const spots = [
    [near, first],
    [near, second],
    [far, first],
    [far, second],
    [middle, first],
    [middle, second],
  ].map(([x, y]) => ({ x: inside(x, width), y: inside(y, height), width, height }));
  const clear = (box: (typeof spots)[number]) => Math.min(...[point, ...avoid].map((spot) => clearance(spot, box)));
  const free = spots.find((box) => clear(box) >= enough);
  const { x, y } = free ?? spots.reduce((best, box) => (clear(box) > clear(best) ? box : best));
  return { x, y };
}

/** The range of turning: past a half turn is the other way round */
const wrapped = (angle: number) => angle - TURN * Math.round(angle / TURN);

/** Upside down the same drag has to turn the sphere the other way for its surface to follow the cursor */
export const upsideDown = ({ pitch }: Pick<SpaceView, "pitch">) => Math.cos(pitch) < 0;

/** A view turned by a drag of this many radians across and down: the surface in front follows the cursor */
export function turned(view: SpaceView, across: number, down: number): SpaceView {
  return { ...view, yaw: wrapped(view.yaw + (upsideDown(view) ? -across : across)), pitch: wrapped(view.pitch + down) };
}

/** Degrees around the listener: 0 in front, plus to the right, 180 behind */
export function azimuth({ x, y }: Place): number {
  return Math.atan2(x, y) * DEGREES;
}

/** Degrees above the listener's level, from -90 to 90 */
export function elevation({ x, y, z }: Place): number {
  return Math.atan2(z, Math.hypot(x, y)) * DEGREES;
}

/** The same place turned to another azimuth: its distance and height stay */
export function atAzimuth(place: Place, degrees: number): Place {
  const across = Math.hypot(place.x, place.y);
  const angle = degrees / DEGREES;
  return { x: across * Math.sin(angle), y: across * Math.cos(angle), z: place.z };
}

/** The same place at another distance: its direction stays, and it stays out of the middle */
export function atDistance(place: Place, metres: number, limit: number): Place {
  const wanted = Math.max(limit * NEAREST, Math.min(limit, metres));
  return times(place, wanted / distanceOf(place));
}

/** The same place lifted to another elevation: its distance and azimuth stay */
export function atElevation(place: Place, degrees: number): Place {
  const distance = distanceOf(place);
  const angle = Math.max(-90, Math.min(90, degrees)) / DEGREES;
  const toward = Math.atan2(place.x, place.y);
  const across = distance * Math.cos(angle);
  return { x: across * Math.sin(toward), y: across * Math.cos(toward), z: distance * Math.sin(angle) };
}
