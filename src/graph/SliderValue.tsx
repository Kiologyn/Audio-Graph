import { useEffect, useRef, useState, type ReactNode } from "react";
import { useGraph } from "./store";

/** Sends a slider's value to the engine at most once per 50 ms: dragging fires far more events */
export function useLiveSend<T>(send: (graphId: string, value: T) => void) {
  const graphId = useGraph((s) => s.meta?.id);
  const last = useRef(0);
  const pending = useRef<number | null>(null);
  return (value: T) => {
    if (!graphId) return;
    const now = () => {
      last.current = Date.now();
      pending.current = null;
      send(graphId, value);
    };
    if (pending.current !== null) clearTimeout(pending.current);
    const wait = 50 - (Date.now() - last.current);
    if (wait <= 0) now();
    else pending.current = window.setTimeout(now, wait);
  };
}

/** Longer than this between two clicks on a value is not a double click */
const DOUBLE_CLICK_MS = 250;

/**
 * A slider's value: a click opens it for typing, a double click resets it, if it can. Enter or leaving the field
 * applies the number, Escape cancels. draft is what the field starts with; a unit or a sign may be typed along
 */
export function SliderValue({
  className,
  draft,
  onChange,
  onReset,
  children,
}: {
  className: string;
  draft: string;
  onChange: (value: number) => void;
  onReset?: () => void;
  children: ReactNode;
}) {
  const [text, setText] = useState<string | null>(null);
  const click = useRef<number | null>(null);
  useEffect(() => () => void (click.current !== null && clearTimeout(click.current)), []);
  const commit = () => {
    const digits = (text ?? "").replace(",", ".").replace(/[^\d.+-]/g, "");
    const value = Number(digits);
    setText(null);
    if (digits !== "" && Number.isFinite(value)) onChange(value);
  };
  if (text !== null) {
    return (
      <span className={className}>
        <input
          className="ag-slider-value nodrag"
          autoFocus
          value={text}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => setText(e.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") {
              // Escape cancels the typing instead of closing the menu
              e.stopPropagation();
              setText(null);
            }
          }}
        />
      </span>
    );
  }
  return (
    <span
      className={className}
      onClick={() => {
        if (click.current !== null) return;
        click.current = window.setTimeout(() => {
          click.current = null;
          setText(draft);
        }, DOUBLE_CLICK_MS);
      }}
      onDoubleClick={() => {
        if (click.current !== null) clearTimeout(click.current);
        click.current = null;
        onReset?.();
      }}
    >
      {children}
    </span>
  );
}
