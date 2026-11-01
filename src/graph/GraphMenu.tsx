import { useEffect, useRef, useState, type ReactNode } from "react";
import { useT } from "../i18n";
import { useSettings } from "../settings";
import { MAX_DELAY_LIMIT, type GraphMeta } from "./doc";
import { useEngine } from "./engine";
import { Icon } from "./icons";
import { useLibrary } from "./library";
import { Popover } from "./Popover";
import { useSortable } from "./sortable";
import { useGraph } from "./store";

type RowMode = "view" | "edit" | "confirm";

function GraphRow({
  graph,
  current,
  mode,
  setMode,
  onOpen,
  grip,
}: {
  graph: GraphMeta;
  current: boolean;
  mode: RowMode;
  setMode: (mode: RowMode) => void;
  onOpen: () => void;
  /** The handle to drag the row by */
  grip: ReactNode;
}) {
  const rename = useLibrary((s) => s.rename);
  const remove = useLibrary((s) => s.remove);
  const input = useRef<HTMLInputElement>(null);
  const t = useT();

  if (mode === "edit") {
    const commit = (value: string) => {
      if (value.trim() && value.trim() !== graph.name) rename(graph.id, value);
      setMode("view");
    };
    return (
      <div className="ag-panel__row is-editing">
        <input
          ref={input}
          className="ag-panel__input"
          autoFocus
          defaultValue={graph.name}
          maxLength={100}
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => commit(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget.value);
            if (e.key === "Escape") {
              // Escape cancels renaming instead of closing the menu
              e.stopPropagation();
              setMode("view");
            }
          }}
        />
        <button
          className="ag-panel__action is-confirm"
          title={t("save")}
          // without this the field loses focus before the click and the row closes on blur
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => commit(input.current?.value ?? "")}
        >
          <Icon name="check" />
        </button>
      </div>
    );
  }

  if (mode === "confirm") {
    return (
      <div className="ag-panel__row is-confirm">
        <span className="ag-panel__confirm-text">{t("delete_confirm", { name: graph.name })}</span>
        <button className="ag-panel__small is-danger" onClick={() => remove(graph.id)}>
          {t("delete")}
        </button>
        <button className="ag-panel__small" onClick={() => setMode("view")}>
          {t("cancel")}
        </button>
      </div>
    );
  }

  return (
    <div className={`ag-panel__row${current ? " is-current" : ""}`}>
      {grip}
      <RunState id={graph.id} />
      <button className="ag-panel__name" onClick={onOpen} onDoubleClick={() => setMode("edit")} title={graph.name}>
        {graph.name}
      </button>
      <GraphOptions graph={graph} />
      <RunToggle id={graph.id} />
      <button className="ag-panel__action" title={t("rename")} onClick={() => setMode("edit")}>
        <Icon name="pencil" />
      </button>
      <button className="ag-panel__action is-danger" title={t("delete")} onClick={() => setMode("confirm")}>
        <Icon name="trash" />
      </button>
    </div>
  );
}

/** The dot of a graph in the list: green means running, red means running with problems */
function RunState({ id }: { id: string }) {
  const running = useEngine((s) => s.status.running.includes(id));
  const troubled = useEngine((s) => s.status.troubled.includes(id));
  const t = useT();
  if (!running) return <span className="ag-graphs__state" />;
  return (
    <span
      className={`ag-graphs__state is-running${troubled ? " is-troubled" : ""}`}
      title={troubled ? t("graph_running_with_problems") : t("graph_running")}
    />
  );
}

/** A number of seconds: written freely, applied on Enter or leaving the field, kept within 0.001..max */
function SecondsField({
  value,
  max,
  disabled,
  title,
  onChange,
}: {
  value: number;
  max: number;
  disabled: boolean;
  title: string;
  onChange: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const t = useT();
  useEffect(() => setDraft(String(value)), [value]);
  const commit = () => {
    // a comma is a decimal separator too
    const n = Number(draft.trim().replace(",", "."));
    const next = Number.isFinite(n) && n > 0 ? Math.min(max, Math.max(0.001, Math.round(n * 1000) / 1000)) : value;
    setDraft(String(next));
    if (next !== value) onChange(next);
  };
  return (
    <label className={`ag-graph-options__seconds${disabled ? " is-disabled" : ""}`} title={title}>
      <span>±</span>
      <input
        type="text"
        inputMode="decimal"
        value={draft}
        disabled={disabled}
        onChange={(e) => setDraft(e.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") {
            // Escape cancels the edit instead of closing the menu
            e.stopPropagation();
            setDraft(String(value));
          }
        }}
      />
      <span>{t("seconds_short")}</span>
    </label>
  );
}

/** The arrow in a graph's row: a menu with its volume and delay sliders and the delay range */
function GraphOptions({ graph }: { graph: GraphMeta }) {
  const setPrefs = useLibrary((s) => s.setPrefs);
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const t = useT();
  const showDelay = graph.showDelay ?? false;
  const autostart = useSettings((s) => s.autostart);
  return (
    <>
      <button
        ref={anchor}
        className={`ag-panel__action ag-graphs__options${open ? " is-open" : ""}`}
        title={t("graph_options")}
        onClick={() => setOpen(!open)}
      >
        <Icon name="chevron" size={12} />
      </button>
      {open && (
        <Popover anchor={anchor} onClose={() => setOpen(false)}>
          <div className="ag-graph-options">
            <label className="ag-graph-options__check">
              <input
                type="checkbox"
                checked={graph.showVolume ?? true}
                onChange={(e) => setPrefs(graph.id, { showVolume: e.currentTarget.checked })}
              />
              <span>
                {t("graph_volume_sliders")}
                <small>{t("graph_volume_sliders_hint")}</small>
              </span>
            </label>
            <div className="ag-graph-options__row">
              <label className="ag-graph-options__check">
                <input
                  type="checkbox"
                  checked={showDelay}
                  onChange={(e) => setPrefs(graph.id, { showDelay: e.currentTarget.checked })}
                />
                <span>
                  {t("graph_delay_sliders")}
                  <small>{t("graph_delay_sliders_hint")}</small>
                </span>
              </label>
              <SecondsField
                value={graph.delayLimit ?? 0.5}
                max={MAX_DELAY_LIMIT}
                disabled={!showDelay}
                title={t("graph_delay_limit", { max: MAX_DELAY_LIMIT })}
                onChange={(delayLimit) => setPrefs(graph.id, { delayLimit })}
              />
            </div>
            <label className={`ag-graph-options__check${autostart ? "" : " is-disabled"}`}>
              <input
                type="checkbox"
                checked={graph.startOnLaunch ?? false}
                disabled={!autostart}
                onChange={(e) => setPrefs(graph.id, { startOnLaunch: e.currentTarget.checked })}
              />
              <span>
                {t("graph_start_on_launch")}
                <small>{t(autostart ? "graph_start_on_launch_hint" : "graph_start_on_launch_off")}</small>
              </span>
            </label>
          </div>
        </Popover>
      )}
    </>
  );
}

/** Starting and stopping a graph right from the list */
function RunToggle({ id }: { id: string }) {
  const running = useEngine((s) => s.status.running.includes(id));
  const t = useT();
  return (
    <button
      className={`ag-panel__action${running ? " is-danger" : " is-run"}`}
      title={running ? t("stop") : t("start")}
      onClick={() => (running ? useEngine.getState().stop(id) : useEngine.getState().start(id))}
    >
      <Icon name={running ? "stop" : "play"} />
    </button>
  );
}

/** Choosing the open graph and managing the graph list */
export function GraphMenu() {
  const graphs = useLibrary((s) => s.graphs);
  const open = useLibrary((s) => s.open);
  const create = useLibrary((s) => s.create);
  const reorder = useLibrary((s) => s.reorder);
  const current = useGraph((s) => s.meta);
  const runningCount = useEngine((s) => s.status.running.length);
  const [isOpen, setOpen] = useState(false);
  // at most one row at a time is being renamed or awaits confirmation
  const [active, setActive] = useState<{ id: string; mode: RowMode } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();
  const sortable = useSortable(
    graphs.map((g) => g.id),
    reorder,
  );

  const close = () => {
    setOpen(false);
    setActive(null);
  };

  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element;
      // a graph's options menu is rendered apart from the panel but belongs to it
      if (root.current?.contains(target) || target.closest?.(".ag-popover")) return;
      close();
    };
    // the first Escape closes an open options menu, only the next one closes the list
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !document.querySelector(".ag-popover") && close();
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  const createAndRename = async () => {
    const id = await create();
    if (id) setActive({ id, mode: "edit" });
  };

  return (
    <div className="ag-graphs" ref={root}>
      <button
        className={`ag-header-btn${isOpen ? " is-open" : ""}`}
        onClick={() => (isOpen ? close() : setOpen(true))}
        title={t("graphs")}
      >
        <Icon name="graph" />
        <span className="ag-header-btn__name">{current?.name ?? "…"}</span>
        {runningCount > 0 && (
          <span className="ag-count is-running" title={t("graphs_running_count", { count: runningCount })}>
            {runningCount}
          </span>
        )}
        <Icon name="chevron" size={12} />
      </button>

      {isOpen && (
        <div className="ag-panel">
          <div className={`ag-panel__list${sortable.dragging ? " is-sorting" : ""}`}>
            {graphs.map((g) => (
              <div
                key={g.id}
                ref={sortable.row(g.id)}
                className={`ag-sortable${sortable.dragging === g.id ? " is-dragging" : ""}`}
                style={sortable.style(g.id)}
              >
                <GraphRow
                  graph={g}
                  current={g.id === current?.id}
                  mode={active?.id === g.id ? active.mode : "view"}
                  setMode={(mode) => setActive(mode === "view" ? null : { id: g.id, mode })}
                  onOpen={() => open(g.id)}
                  grip={
                    <span className="ag-panel__grip" title={t("drag_to_reorder")} {...sortable.handle(g.id)}>
                      <Icon name="grip" />
                    </span>
                  }
                />
              </div>
            ))}
          </div>
          <div className="ag-panel__footer">
            <button className="ag-panel__add" onClick={createAndRename}>
              <Icon name="plus" />
              {t("new_graph")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
