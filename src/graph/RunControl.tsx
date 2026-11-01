import { useMemo, useRef, useState } from "react";
import { useT } from "../i18n";
import { useCables } from "./cables";
import { useDevices } from "./devices";
import { useEngine } from "./engine";
import { Icon } from "./icons";
import { Popover } from "./Popover";
import { allProblems } from "./problems";
import { useGraph } from "./store";

/** Starting and stopping the graph, its state and problem list: on the right of the header */
export function RunControl() {
  const status = useEngine((s) => s.status);
  const error = useEngine((s) => s.error);
  const graphId = useGraph((s) => s.meta?.id);
  const nodes = useGraph((s) => s.nodes);
  const cables = useCables((s) => s.cables);
  const devices = useDevices((s) => s.devices);
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const t = useT();

  // t in the dependencies: problem texts are translated when the language changes
  const problems = useMemo(
    () => allProblems(status, graphId, nodes, cables, devices),
    [status, graphId, nodes, cables, devices, t],
  );
  const errors = problems.filter((p) => p.level === "error").length;
  const runningThis = !!graphId && status.running.includes(graphId);

  return (
    <div className="ag-run">
      {runningThis && (
        <span className="ag-run__state is-on">
          <span className="ag-run__dot" />
          {t("graph_running")}
        </span>
      )}
      {error && <span className="ag-run__state is-error">{error}</span>}

      {problems.length > 0 && (
        <button
          ref={anchor}
          className={`ag-header-btn ag-run__problems${errors ? " is-error" : ""}${open ? " is-open" : ""}`}
          onClick={() => setOpen(!open)}
          title={t("graph_problems")}
        >
          <Icon name="warning" />
          {problems.length}
        </button>
      )}
      {open && problems.length > 0 && (
        <Popover anchor={anchor} onClose={() => setOpen(false)}>
          {problems.map((p) => (
            <div key={p.key} className={`ag-problem is-${p.level}`}>
              {p.who && <div className="ag-problem__who">{p.who}</div>}
              <div className="ag-problem__text">{p.text}</div>
            </div>
          ))}
        </Popover>
      )}

      {runningThis ? (
        <button
          className="ag-run__btn is-stop"
          title={t("stop")}
          onClick={() => graphId && useEngine.getState().stop(graphId)}
        >
          <Icon name="stop" />
        </button>
      ) : (
        <button
          className="ag-run__btn"
          title={t("start")}
          disabled={!graphId}
          onClick={() => graphId && useEngine.getState().start(graphId)}
        >
          <Icon name="play" />
        </button>
      )}
    </div>
  );
}
