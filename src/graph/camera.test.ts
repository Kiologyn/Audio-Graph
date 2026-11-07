import { describe, expect, it } from "vitest";
import { graphFits, MAX_ZOOM, MIN_ZOOM, openingView, sameView, usableCamera, viewFor } from "./camera";

const SMALL = { width: 1280, height: 720 };
const BIG = { width: 1920, height: 1000 };

/** The point of the graph in the middle of a view */
const middle = (view: { x: number; y: number; zoom: number }, size: { width: number; height: number }) => ({
  x: (size.width / 2 - view.x) / view.zoom,
  y: (size.height / 2 - view.y) / view.zoom,
});

describe("viewFor", () => {
  it("gives back what was left on the same size, exactly", () => {
    const camera = { x: 484.09838, y: -50.7528, zoom: 1.3608, ...BIG };
    expect(viewFor(camera, BIG)).toEqual({ x: 484.09838, y: -50.7528, zoom: 1.3608 });
  });

  it("keeps the same point in the middle", () => {
    const camera = { x: 300, y: -40, zoom: 1.2, ...BIG };
    const view = viewFor(camera, SMALL);
    expect(middle(view, SMALL).x).toBeCloseTo(middle(camera, BIG).x);
    expect(middle(view, SMALL).y).toBeCloseTo(middle(camera, BIG).y);
  });

  it("keeps what was visible visible: the zoom goes by the side that allows less", () => {
    const camera = { x: 0, y: 0, zoom: 1, ...BIG };
    expect(viewFor(camera, SMALL).zoom).toBeCloseTo(1280 / 1920);
    // a narrower height decides here
    expect(viewFor(camera, { width: 1920, height: 500 }).zoom).toBeCloseTo(0.5);
    // a graph left on a big screen fits the smallest window
    expect(viewFor({ x: 0, y: 0, zoom: 0.5, width: 3840, height: 2000 }, { width: 800, height: 440 }).zoom).toBeCloseTo(0.104, 3);
  });

  it("depends only on the camera and the size: any way there and back ends the same", () => {
    const camera = { x: 120, y: 80, zoom: 1.7, ...SMALL };
    for (const size of [BIG, { width: 801, height: 503 }, { width: 3000, height: 2000 }]) viewFor(camera, size);
    expect(viewFor(camera, SMALL)).toEqual({ x: 120, y: 80, zoom: 1.7 });
  });

  it("stays within the zoom range", () => {
    expect(viewFor({ x: 0, y: 0, zoom: 1.9, ...SMALL }, { width: 3840, height: 2160 }).zoom).toBe(MAX_ZOOM);
    expect(viewFor({ x: 0, y: 0, zoom: 0.11, ...BIG }, { width: 800, height: 400 }).zoom).toBe(MIN_ZOOM);
  });
});

describe("usableCamera", () => {
  it("leaves out what would break the canvas", () => {
    expect(usableCamera(null)).toBeNull();
    expect(usableCamera({ x: 0, y: 0, zoom: 0 })).toBeNull();
    expect(usableCamera({ x: Number.NaN, y: 0, zoom: 1 })).toBeNull();
    expect(usableCamera({ x: 0, y: 0, zoom: 1 })).toEqual({ x: 0, y: 0, zoom: 1 });
  });
});

describe("graphFits", () => {
  const node = (x: number, y: number) => ({ position: { x, y } });

  it("sees a node past any edge", () => {
    const view = { x: 0, y: 0, zoom: 1 };
    expect(graphFits([node(10, 10), node(1000, 500)], view, SMALL)).toBe(true);
    expect(graphFits([node(1100, 10)], view, SMALL)).toBe(false);
    expect(graphFits([node(10, 600)], view, SMALL)).toBe(false);
    expect(graphFits([node(-10, 10)], view, SMALL)).toBe(false);
  });

  it("counts the view's zoom and place", () => {
    expect(graphFits([node(1100, 10)], { x: 0, y: 0, zoom: 0.5 }, SMALL)).toBe(true);
    expect(graphFits([node(10, 10)], { x: 1100, y: 0, zoom: 1 }, SMALL)).toBe(false);
  });
});

describe("openingView", () => {
  const nodes = [{ position: { x: 100, y: 100 } }, { position: { x: 1500, y: 600 } }];

  it("fits a graph with no camera", () => {
    expect(openingView(null, nodes, SMALL)).toEqual({ camera: null, view: null });
  });

  it("opens a camera with a size on any canvas, and goes on from the camera as it was left", () => {
    const camera = { x: 10, y: 20, zoom: 1, ...BIG };
    const opened = openingView(camera, nodes, SMALL);
    expect(opened.camera).toEqual(camera);
    expect(opened.view).toEqual(viewFor(camera, SMALL));
    expect(openingView(camera, nodes, BIG).view).toEqual({ x: 10, y: 20, zoom: 1 });
  });

  it("takes a camera of an earlier version as it is if the graph shows whole", () => {
    const opened = openingView({ x: 0, y: 0, zoom: 0.5 }, nodes, SMALL);
    expect(opened.view).toEqual({ x: 0, y: 0, zoom: 0.5 });
    expect(opened.camera).toEqual({ x: 0, y: 0, zoom: 0.5, ...SMALL });
  });

  it("fits a graph that a camera of an earlier version cuts off", () => {
    expect(openingView({ x: 484, y: -50, zoom: 1.36 }, nodes, SMALL)).toEqual({ camera: null, view: null });
  });

  it("doesn't trust a damaged camera", () => {
    expect(openingView({ x: 0, y: 0, zoom: 0, ...BIG }, nodes, SMALL)).toEqual({ camera: null, view: null });
  });
});

describe("sameView", () => {
  it("doesn't mind rounding", () => {
    expect(sameView({ x: 1, y: 2, zoom: 1 }, { x: 1.0001, y: 2, zoom: 1 })).toBe(true);
    expect(sameView({ x: 1, y: 2, zoom: 1 }, { x: 1.1, y: 2, zoom: 1 })).toBe(false);
  });
});
