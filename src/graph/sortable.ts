import { useEffect, useRef, useState, type CSSProperties, type PointerEvent } from "react";

/**
 * Items in the order of the ids; items missing from the ids (added meanwhile) keep their order
 * at the end, as the order is kept on disk (cables_reorder in cables.rs, the graph list in graphs.rs)
 */
export function inOrder<T extends { id: string }>(items: T[], ids: string[]): T[] {
  const place = (id: string) => {
    const i = ids.indexOf(id);
    return i < 0 ? ids.length : i;
  };
  // the sort is stable: the missing ones stay in their order
  return [...items].sort((a, b) => place(a.id) - place(b.id));
}

/** A row being dragged: where it came from, where it would land, how far the pointer moved */
type Drag = { id: string; from: number; to: number; dy: number };

/**
 * Reordering a list by dragging a row by its handle. Pointer events, not HTML drag and drop:
 * Tauri takes drag and drop for dropping files into the window.
 * The dragged row follows the pointer, the rows it passes move aside; on release onMove gets the new order.
 */
export function useSortable(ids: string[], onMove: (ids: string[]) => void) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const rows = useRef(new Map<string, HTMLElement>());
  // the handlers below are created once per drag, they read the latest values from here
  const latest = useRef({ ids, onMove });
  latest.current = { ids, onMove };
  const stop = useRef<() => void>(() => {});

  // the list can disappear mid-drag (the panel closes)
  useEffect(() => () => stop.current(), []);

  /** The ref of a row's outer element */
  const row = (id: string) => (el: HTMLElement | null) => {
    if (el) rows.current.set(id, el);
    else rows.current.delete(id);
  };

  /** Props of a row's handle */
  const handle = (id: string) => ({
    onPointerDown: (e: PointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const order = latest.current.ids;
      const from = order.indexOf(id);
      const rects = order.map((i) => rows.current.get(i)?.getBoundingClientRect());
      if (from < 0 || rects.some((r) => !r)) return;
      const startY = e.clientY;
      const self = rects[from]!;
      let current: Drag = { id, from, to: from, dy: 0 };
      setDrag(current);

      const move = (ev: globalThis.PointerEvent) => {
        const dy = ev.clientY - startY;
        const center = self.top + self.height / 2 + dy;
        // the new place: how many other rows are above the dragged row's center
        const to = rects.filter((r, i) => i !== from && r!.top + r!.height / 2 < center).length;
        current = { id, from, to, dy };
        setDrag(current);
      };
      const up = () => {
        stop.current();
        if (current.to !== current.from) {
          const next = latest.current.ids.filter((i) => i !== id);
          next.splice(current.to, 0, id);
          latest.current.onMove(next);
        }
      };
      stop.current = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        window.removeEventListener("pointercancel", up);
        stop.current = () => {};
        setDrag(null);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", up);
    },
  });

  /** The row's shift while dragging: the dragged one follows the pointer, the passed ones make room for it */
  const style = (id: string): CSSProperties | undefined => {
    if (!drag) return undefined;
    if (id === drag.id) return { transform: `translateY(${drag.dy}px)` };
    const i = ids.indexOf(id);
    const el = rows.current.get(drag.id);
    const size = el ? el.offsetHeight : 0;
    if (drag.from < i && i <= drag.to) return { transform: `translateY(${-size}px)` };
    if (drag.to <= i && i < drag.from) return { transform: `translateY(${size}px)` };
    return undefined;
  };

  return { dragging: drag?.id ?? null, row, handle, style };
}
