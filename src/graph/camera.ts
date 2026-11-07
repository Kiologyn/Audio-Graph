import type { Viewport } from "@xyflow/react";

// the canvas's zoom range. A big window may hold a view at a low zoom: what it shows has to fit a small one too
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 2;

export type Size = { width: number; height: number };

/**
 * A view as it was left by hand, with the size of the canvas it was left on: the view for any other size follows
 * from it, so a window that changes size and comes back changes nothing. Files of earlier versions have no size.
 */
export type Camera = Viewport & Partial<Size>;

/** What a node takes on the canvas, about: the file doesn't hold their sizes */
const NODE_SIZE = { width: 240, height: 140 };

/** A saved camera the canvas can use: a damaged file is as good as none */
export function usableCamera(camera: Camera | null | undefined): Camera | null {
  if (!camera) return null;
  const { x, y, zoom } = camera;
  return Number.isFinite(x) && Number.isFinite(y) && Number.isFinite(zoom) && zoom > 0 ? camera : null;
}

const hasSize = (camera: Camera): camera is Viewport & Size => (camera.width ?? 0) > 0 && (camera.height ?? 0) > 0;

/**
 * The view of a camera on a canvas of this size: the same point stays in the middle and what was visible stays
 * visible, so the zoom goes by the side that allows less.
 */
export function viewFor(camera: Camera, size: Size): Viewport {
  const { x, y, zoom } = camera;
  if (!hasSize(camera) || (camera.width === size.width && camera.height === size.height)) return { x, y, zoom };
  const fit = Math.min(size.width / camera.width, size.height / camera.height);
  const fitted = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom * fit));
  const middle = { x: (camera.width / 2 - x) / zoom, y: (camera.height / 2 - y) / zoom };
  return { x: size.width / 2 - middle.x * fitted, y: size.height / 2 - middle.y * fitted, zoom: fitted };
}

/** The camera didn't move, whatever rounding it went through */
export function sameView(a: Viewport, b: Viewport): boolean {
  return (["x", "y", "zoom"] as const).every((k) => Math.abs(a[k] - b[k]) < 0.001);
}

/** Every node is inside the view */
export function graphFits(nodes: { position: { x: number; y: number } }[], view: Viewport, size: Size): boolean {
  return nodes.every(({ position }) => {
    const [left, top] = [position.x * view.zoom + view.x, position.y * view.zoom + view.y];
    return left >= 0 && top >= 0 && left + NODE_SIZE.width * view.zoom <= size.width && top + NODE_SIZE.height * view.zoom <= size.height;
  });
}

/** How a graph opens: a camera to go on from and the view to start with; without a view the graph is fit to the canvas */
export function openingView(
  saved: Camera | null | undefined,
  nodes: { position: { x: number; y: number } }[],
  size: Size,
): { camera: Camera | null; view: Viewport | null } {
  const camera = usableCamera(saved);
  if (!camera) return { camera: null, view: null };
  if (hasSize(camera)) return { camera, view: viewFor(camera, size) };
  // nobody knows what size this camera was left on: what shows the whole graph is taken as it is, the rest is fit
  const { x, y, zoom } = camera;
  const view = { x, y, zoom };
  return graphFits(nodes, view, size) ? { camera: { ...view, ...size }, view } : { camera: null, view: null };
}
