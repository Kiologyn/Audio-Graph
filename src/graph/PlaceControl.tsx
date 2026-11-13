import { useEffect, useId, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { useT } from "../i18n";
import { useSettings } from "../settings";
import { useEngine } from "./engine";
import { SliderValue, useLiveSend } from "./SliderValue";
import {
  FIRST_VIEW,
  FROM_ABOVE,
  atAzimuth,
  atDistance,
  atElevation,
  azimuth,
  coneFaces,
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
  withinLimit,
  type Grab,
  type Handle,
  type Place,
  type SpaceView,
} from "./space";
import { useGraph } from "./store";

/** The drawing's own units: the node scales it */
const SIZE = 200;
const MIDDLE = SIZE / 2;
/** Room stays around the sphere for the arrows of a source on its surface */
const RADIUS = 74;

/** An arrow on the drawing: where it starts from the point, where its head starts, how long and wide the head is */
const ARROW = { from: 7, head: 21, height: 4, width: 1.7 };
/** How far from an arrow's line and from the middle of its head the mouse still takes it */
const ARROW_REACH = 10;
const HEAD_REACH = 7;

/** A press that moved less than this many pixels is a click, not a drag */
const DRAG_FROM = 2;
/** A right-button drag ends with the system's context menu coming right after */
const MENU_AFTER_MS = 100;

const AXES: Place[] = [
  { x: 1, y: 0, z: 0 },
  { x: 0, y: 1, z: 0 },
  { x: 0, y: 0, z: 1 },
];
const FRONT = AXES[1];

type View = Pick<SpaceView, "yaw" | "pitch">;

/** A place on the drawing; depth goes from -1 at the back of the sphere to 1 in front of it */
function pointOf(place: Place, view: View, scale: number) {
  const { right, up, towards } = project(place, view);
  return { x: MIDDLE + right * scale, y: MIDDLE - up * scale, depth: (towards * scale) / RADIUS };
}

/** Half an axis, from the listener to the surface. What is further away is fainter */
function HalfAxis({ to, view }: { to: Place; view: View }) {
  const end = pointOf(to, view, RADIUS);
  return <line className="ag-space__axis" x1={MIDDLE} y1={MIDDLE} x2={end.x} y2={end.y} style={{ opacity: 0.2 + end.depth * 0.1 }} />;
}

/** A small mark where the listener faces, on the surface */
function FrontMark({ view }: { view: View }) {
  const tip = pointOf(FRONT, view, RADIUS - 1);
  const [dx, dy] = [tip.x - MIDDLE, tip.y - MIDDLE];
  const length = Math.hypot(dx, dy);
  const opacity = 0.42 + tip.depth * 0.14;
  // looking straight along it there is no way to point
  if (length < 6) return <circle className="ag-space__front" cx={tip.x} cy={tip.y} r={2} style={{ opacity }} />;
  const [ux, uy] = [dx / length, dy / length];
  const back = { x: tip.x - ux * 8, y: tip.y - uy * 8 };
  const corners = [
    [tip.x, tip.y],
    [back.x - uy * 4, back.y + ux * 4],
    [back.x + uy * 4, back.y - ux * 4],
  ];
  return <polygon className="ag-space__front" points={corners.map((c) => c.join(",")).join(" ")} style={{ opacity }} />;
}

/** A face's color: the arrow's own in the middle of the light, darker or lighter from it */
function shaded(light: number) {
  const towards = light < 0.7 ? "black" : "white";
  return `color-mix(in srgb, var(--axis), ${towards} ${Math.round(Math.abs(light - 0.7) * 100)}%)`;
}

/** Along an axis from a point of the drawing, as the view shows it */
function along(from: { x: number; y: number }, axis: Place, view: View, length: number) {
  const { right, up } = project(axis, view);
  return { x: from.x + right * length, y: from.y - up * length };
}

/** One of a source's arrows, with a head that is a cone in the same space as the sphere */
function Arrow({ handle, axis, at, view, held }: { handle: Handle; axis: Place; at: { x: number; y: number }; view: View; held: boolean }) {
  const start = along(at, axis, view, ARROW.from);
  const base = along(at, axis, view, ARROW.head);
  const tip = along(at, axis, view, ARROW.head + ARROW.height);
  return (
    <g className={`ag-space__arrow is-${handle}${held ? " is-held" : ""}`} data-handle={handle}>
      <line className="ag-space__reach" x1={start.x} y1={start.y} x2={tip.x} y2={tip.y} strokeWidth={ARROW_REACH} />
      <line className="ag-space__shaft" x1={start.x} y1={start.y} x2={base.x} y2={base.y} />
      {coneFaces(base, axis, ARROW.height, ARROW.width, view).map((face, i) => (
        <polygon key={i} points={face.points.map((p) => p.join(",")).join(" ")} style={{ fill: shaded(face.light), stroke: shaded(face.light) }} />
      ))}
    </g>
  );
}

/**
 * A cable's place in space: a circle seen from above, or a sphere that turns with any mouse button. The point is
 * dragged along the node's plane; on the sphere three arrows change its distance and move it along its parallel and
 * meridian, those two once it is out of the middle. The middle is only reached by a double click on the point or the
 * listener
 */
export function PlaceControl({ id, place, view, limit }: { id: string; place: Place | null; view: SpaceView | null; limit: number }) {
  const setPlace = useGraph((s) => s.setPlace);
  const setSpaceView = useGraph((s) => s.setSpaceView);
  const send = useLiveSend((graphId, value: Place | null) => useEngine.getState().place(graphId, id, value));
  const language = useSettings((s) => s.language);
  const t = useT();
  const shade = `ag-space-${useId().replace(/\W/g, "")}`;
  const drawing = useRef<HTMLDivElement>(null);
  const picture = useRef<SVGSVGElement>(null);
  const values = useRef<HTMLDivElement>(null);
  const saved = view ?? FIRST_VIEW;
  // a turn reaches the file when it ends
  const [turning, setTurning] = useState<SpaceView | null>(null);
  const [held, setHeld] = useState<Grab | null>(null);
  const [under, setUnder] = useState<Handle | null>(null);
  const looking = turning ?? saved;
  const solid = looking.solid;
  const camera = solid ? looking : FROM_ABOVE;
  const scale = RADIUS / limit;
  const at = place && withinLimit(place, limit);
  const point = pointOf(at ?? { x: 0, y: 0, z: 0 }, camera, scale);
  const foot = at && solid ? pointOf({ ...at, z: 0 }, camera, scale) : null;

  const frame = frameAt(at);
  // in the middle there is no parallel or meridian to go along
  const ways: [Handle, Place][] = at
    ? [
        ["distance", frame.out],
        ["parallel", frame.along],
        ["meridian", frame.up],
      ]
    : [["distance", frame.out]];
  // what comes towards whoever looks is drawn over what goes away
  const arrows = solid ? ways.sort(([, a], [, b]) => project(a, camera).towards - project(b, camera).towards) : [];
  const arrow = ([handle, axis]: [Handle, Place]) => <Arrow key={handle} handle={handle} axis={axis} at={point} view={camera} held={held?.handle === handle || under === handle} />;

  // the values go where they cover nothing, once their size is known
  useLayoutEffect(() => {
    const [box, label] = [drawing.current, values.current];
    if (!box || !label) return;
    const unit = SIZE / box.clientWidth;
    const size = { width: label.offsetWidth * unit, height: label.offsetHeight * unit };
    const reach = ARROW.head + ARROW.height;
    const avoid = arrows.flatMap(([, axis]) => [along(point, axis, camera, reach / 2), along(point, axis, camera, reach)]);
    const spot = labelSpot(point, size, SIZE, [{ x: MIDDLE, y: MIDDLE }, ...avoid]);
    label.style.left = `${(spot.x / SIZE) * 100}%`;
    label.style.top = `${(spot.y / SIZE) * 100}%`;
  });

  // on a node the wheel's press moves the canvas whatever the node's classes say: only stopping it on its way does
  useEffect(() => {
    const svg = picture.current;
    if (!svg || !solid) return;
    const keep = (e: MouseEvent) => {
      if (e.button !== 1) return;
      e.stopPropagation();
      // it would start the browser's scrolling too
      e.preventDefault();
    };
    svg.addEventListener("mousedown", keep);
    return () => svg.removeEventListener("mousedown", keep);
  }, [solid]);

  const change = (next: Place | null, continuing = false) => {
    setPlace(id, next, continuing);
    send(next);
  };

  const press = (e: ReactPointerEvent<SVGSVGElement>) => {
    const target = e.target as Element;
    const left = e.button === 0;
    const handle = left ? (target.closest("[data-handle]")?.getAttribute("data-handle") as Handle | null | undefined) : null;
    const moving = left && target.getAttribute("data-part") === "point";
    if (!moving && !handle && !solid) return;
    const button = e.button;
    const box = e.currentTarget.getBoundingClientRect();
    const unit = SIZE / box.width;
    const spot = ({ clientX, clientY }: { clientX: number; clientY: number }) => ({
      right: ((clientX - box.left) * unit - MIDDLE) / scale,
      up: (MIDDLE - (clientY - box.top) * unit) / scale,
    });
    const [fromX, fromY] = [e.clientX, e.clientY];
    const from = at ?? { x: 0, y: 0, z: 0 };
    let holding = handle ? grab(handle, at, spot(e), camera) : null;
    let moved = false;
    let view = looking;
    if (holding) {
      setHeld(holding);
      // the cursor is free to go far from the arrow: what it passes on the way stays out of it
      e.currentTarget.setPointerCapture(e.pointerId);
    }
    const move = (m: PointerEvent) => {
      const [x, y] = [m.clientX - fromX, m.clientY - fromY];
      if (!moved && Math.hypot(x, y) < DRAG_FROM) return;
      if (holding) {
        holding = dragged(holding, spot(m), camera, limit);
        change(holding.place, moved);
      } else if (moving) change(moveOnNode(from, (x * unit) / scale, (-y * unit) / scale, camera, limit), moved);
      else setTurning((view = turned(looking, (x * unit) / RADIUS, (y * unit) / RADIUS)));
      moved = true;
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      setHeld(null);
      if (moved && !moving && !holding) {
        setSpaceView(id, view);
        setTurning(null);
      }
      if (button === 2) {
        const noMenu = (c: MouseEvent) => c.preventDefault();
        window.addEventListener("contextmenu", noMenu, { capture: true });
        setTimeout(() => window.removeEventListener("contextmenu", noMenu, { capture: true }), MENU_AFTER_MS);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  const toMiddle = (e: React.MouseEvent<SVGSVGElement>) => {
    const part = (e.target as Element).getAttribute("data-part");
    if (at && (part === "point" || part === "listener")) change(null);
  };

  const show = (next: boolean) => {
    if (next !== solid) setSpaceView(id, { ...saved, solid: next });
  };

  const across = at ? Math.round(azimuth(at)) : 0;
  const high = at ? Math.round(elevation(at)) : 0;
  const metres = at ? distanceOf(at) : 0;
  const number = new Intl.NumberFormat(language, { maximumFractionDigits: metres < 10 ? 2 : metres < 100 ? 1 : 0, useGrouping: false }).format(metres);

  return (
    <div ref={drawing} className={`ag-space nodrag nopan${solid ? " is-solid" : ""}`} title={t("place_hint")}>
      <svg
        ref={picture}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        onPointerDown={press}
        onDoubleClick={toMiddle}
        onContextMenu={(e) => {
          if (!solid) return;
          e.preventDefault();
          e.stopPropagation();
        }}
      >
        <defs>
          <radialGradient id={shade} cx="38%" cy="32%" r="75%">
            <stop offset="0%" className="ag-space__light" />
            <stop offset="100%" className="ag-space__shade" />
          </radialGradient>
        </defs>
        <circle className="ag-space__ball" cx={MIDDLE} cy={MIDDLE} r={RADIUS} style={solid ? { fill: `url(#${shade})` } : undefined} />
        {solid && (
          <>
            {AXES.flatMap((axis, i) => [
              <HalfAxis key={i} to={axis} view={camera} />,
              <HalfAxis key={-i - 1} to={{ x: -axis.x, y: -axis.y, z: -axis.z }} view={camera} />,
            ])}
            <ellipse className="ag-space__axis" cx={MIDDLE} cy={MIDDLE} rx={RADIUS} ry={Math.abs(Math.sin(camera.pitch)) * RADIUS} style={{ opacity: 0.16 }} />
          </>
        )}
        <FrontMark view={camera} />
        <circle className="ag-space__listener" data-part="listener" cx={MIDDLE} cy={MIDDLE} r={4} />
        {foot && (
          <>
            <line className="ag-space__stem" x1={MIDDLE} y1={MIDDLE} x2={foot.x} y2={foot.y} />
            <line className="ag-space__stem" x1={foot.x} y1={foot.y} x2={point.x} y2={point.y} />
          </>
        )}
        {held?.ring && (
          <polyline
            className={`ag-space__ring is-${held.handle}`}
            points={ringPlaces(held.ring)
              .map((place) => pointOf(place, camera, scale))
              .map(({ x, y }) => `${x},${y}`)
              .join(" ")}
          />
        )}
        {arrows.filter(([, axis]) => project(axis, camera).towards < 0).map(arrow)}
        <circle
          className={`ag-space__source${at ? " is-on" : ""}`}
          cx={point.x}
          cy={point.y}
          r={at && solid ? 5.5 + point.depth * 1.5 : 5.5}
          style={at && solid ? { opacity: 0.8 + point.depth * 0.2 } : undefined}
        />
        {arrows.filter(([, axis]) => project(axis, camera).towards >= 0).map(arrow)}
        {/* over all the arrows: where they cross, the one whose head it is gets the press */}
        {arrows.map(([handle, axis]) => {
          const head = along(point, axis, camera, ARROW.head + ARROW.height / 2);
          return (
            <circle
              key={handle}
              className="ag-space__reach"
              data-handle={handle}
              cx={head.x}
              cy={head.y}
              r={HEAD_REACH}
              onPointerEnter={() => setUnder(handle)}
              onPointerLeave={() => setUnder(null)}
            />
          );
        })}
        <circle className="ag-space__grip" data-part="point" cx={point.x} cy={point.y} r={solid ? ARROW.from : 12} />
      </svg>

      <div className="ag-space__modes">
        <button className={solid ? "" : "is-on"} onClick={() => show(false)}>
          {t("place_view_2d")}
        </button>
        <button className={solid ? "is-on" : ""} onClick={() => show(true)}>
          {t("place_view_3d")}
        </button>
      </div>

      {at && (
        <div ref={values} className="ag-space__values">
          <SliderValue className="ag-space__value" draft={String(across)} onChange={(typed) => change(atAzimuth(at, typed))}>
            {across}°
          </SliderValue>
          {solid && (
            <SliderValue className="ag-space__value" draft={String(high)} onChange={(typed) => change(atElevation(at, typed))}>
              {high < 0 ? "▾" : "▴"}
              {Math.abs(high)}°
            </SliderValue>
          )}
          <SliderValue className="ag-space__value" draft={number} onChange={(typed) => change(atDistance(at, typed, limit))}>
            {t("place_distance", { value: number })}
          </SliderValue>
        </div>
      )}
    </div>
  );
}
