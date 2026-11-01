import { Handle, Position, type NodeProps } from "@xyflow/react";
import { memo, useMemo, useRef, useState, type CSSProperties } from "react";
import { useT } from "../i18n";
import { dynamicCables, virtualDeviceName } from "../platform";
import { useSettings } from "../settings";
import { AppPicker } from "./AppPicker";
import { CablePicker } from "./CablePicker";
import { useCables } from "./cables";
import { DevicePicker } from "./DevicePicker";
import { useDevices } from "./devices";
import { useEngine } from "./engine";
import { KindIcon } from "./icons";
import { nodeNotes } from "./problems";
import { useGraph } from "./store";
import { NODE_KINDS, type AudioNode } from "./types";

/** The system's default devices now; while a graph with this node runs, it sets them */
function SystemDefaults() {
  const devices = useDevices((s) => s.devices);
  const t = useT();
  const name = (flow: "render" | "capture") => devices.find((d) => d.flow === flow && d.isDefault)?.name ?? t("none");
  return (
    <dl
      className="ag-node__defaults"
      title={t(
        "system_node_hint",
      )}
    >
      <dt>{t("node_output")}</dt>
      <dd>{name("render")}</dd>
      <dt>{t("node_input")}</dt>
      <dd>{name("capture")}</dd>
    </dl>
  );
}

/** A cable on Linux: a device the graph creates itself; programs see it under this name */
function VirtualCable({ label }: { label: string }) {
  const t = useT();
  return (
    <div
      className="ag-node__virtual"
      title={t("virtual_cable_hint")}
    >
      <span>{virtualDeviceName(label)}</span>
    </div>
  );
}

/** A node's volume slider. Double-clicking the percentage resets it to 100% */
function VolumeSlider({ id, value, max }: { id: string; value: number; max: number }) {
  const setVolume = useGraph((s) => s.setVolume);
  const graphId = useGraph((s) => s.meta?.id);
  const t = useT();
  // to the engine at most once per 50 ms: dragging fires far more events
  const last = useRef(0);
  const pending = useRef<number | null>(null);
  const change = (volume: number) => {
    setVolume(id, volume);
    if (!graphId) return;
    const send = () => {
      last.current = Date.now();
      pending.current = null;
      useEngine.getState().volume(graphId, id, volume);
    };
    if (pending.current !== null) clearTimeout(pending.current);
    const wait = 50 - (Date.now() - last.current);
    if (wait <= 0) send();
    else pending.current = window.setTimeout(send, wait);
  };
  const percent = Math.round(value * 100);
  return (
    <div className="ag-volume nodrag" title={t("volume_hint")}>
      <input
        type="range"
        min={0}
        max={max * 100}
        step={1}
        value={percent}
        style={{ "--fill": `${(percent / (max * 100)) * 100}%` } as CSSProperties}
        onChange={(e) => change(Number(e.currentTarget.value) / 100)}
      />
      <span className={`ag-volume__value${percent > 100 ? " is-boost" : ""}`} onDoubleClick={() => change(1)}>
        {percent}%
      </span>
    </div>
  );
}

/**
 * An output's delay: from minus the range to plus, no delay in the middle. The engine gets it
 * with autosave: a delay change restarts the sound for a moment anyway, so there is no live sending.
 * Double-clicking the value resets it
 */
function DelaySlider({ id, value, limit }: { id: string; value: number; limit: number }) {
  const setDelay = useGraph((s) => s.setDelay);
  const language = useSettings((s) => s.language);
  const t = useT();
  // a delay beyond a narrowed range acts as the range's edge, in the engine too
  const shown = Math.min(limit, Math.max(-limit, value));
  const seconds = (v: number, digits: number) =>
    t("delay_seconds", {
      value: new Intl.NumberFormat(language, {
        minimumFractionDigits: digits,
        maximumFractionDigits: 3,
        signDisplay: "exceptZero",
      }).format(v),
    });
  // where the knob is, 0..1: the fill goes from the middle to it, the value is written under it
  const at = 0.5 + shown / limit / 2;
  return (
    <div className="ag-delay nodrag" title={t("delay_hint")}>
      <span className="ag-delay__edge">{seconds(-limit, 0)}</span>
      <div className="ag-delay__slider" style={{ "--at": at } as CSSProperties}>
        <input
          type="range"
          min={-limit * 1000}
          max={limit * 1000}
          step={1}
          value={Math.round(shown * 1000)}
          style={{ "--from": `${Math.min(at, 0.5) * 100}%`, "--to": `${Math.max(at, 0.5) * 100}%` } as CSSProperties}
          onChange={(e) => setDelay(id, Number(e.currentTarget.value) / 1000)}
        />
        <span className={`ag-delay__value${shown !== 0 ? " is-set" : ""}`} onDoubleClick={() => setDelay(id, 0)}>
          {seconds(shown, 3)}
        </span>
      </div>
      <span className="ag-delay__edge">{seconds(limit, 0)}</span>
    </div>
  );
}

function AudioNodeView({ id, data, selected }: NodeProps<AudioNode>) {
  const meta = NODE_KINDS[data.kind];
  const renameNode = useGraph((s) => s.renameNode);
  const setDevice = useGraph((s) => s.setDevice);
  const setCable = useGraph((s) => s.setCable);
  const setApp = useGraph((s) => s.setApp);
  const [editing, setEditing] = useState(false);
  const status = useEngine((s) => s.status);
  const graphId = useGraph((s) => s.meta?.id);
  const cables = useCables((s) => s.cables);
  const devices = useDevices((s) => s.devices);
  const showVolume = useGraph((s) => s.meta?.showVolume ?? true);
  const showDelay = useGraph((s) => s.meta?.showDelay ?? false);
  const delayLimit = useGraph((s) => s.meta?.delayLimit ?? 0.5);
  const t = useT();
  // t in the dependencies: note texts are translated when the language changes
  const { notes, flagged } = useMemo(
    () => nodeNotes(id, data, status, graphId, cables, devices),
    [id, data, status, graphId, cables, devices, t],
  );

  const commit = (value: string) => {
    const label = value.trim();
    if (label) renameNode(id, label);
    setEditing(false);
  };

  return (
    <div className={`ag-node ag-node--${data.kind}${selected ? " is-selected" : ""}${flagged ? " has-issue" : ""}`}>
      {meta.hasInput && (
        <Handle type="target" position={Position.Left} id="in" className="ag-port" />
      )}

      <div className="ag-node__kind">
        <KindIcon kind={data.kind} />
        {t(meta.title)}
      </div>

      {editing ? (
        <input
          className="ag-node__rename nodrag"
          autoFocus
          defaultValue={data.label}
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => commit(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget.value);
            if (e.key === "Escape") setEditing(false);
          }}
        />
      ) : (
        <div className="ag-node__label" onDoubleClick={() => setEditing(true)} title={t("rename_hint")}>
          {data.label}
        </div>
      )}

      {meta.pickCable &&
        (dynamicCables() ? (
          <VirtualCable label={data.label} />
        ) : (
          <div className="ag-node__devices">
            <CablePicker value={data.cable} onChange={(cable) => setCable(id, cable)} />
          </div>
        ))}

      {(meta.sink || meta.source) && (
        <div className="ag-node__devices">
          {meta.sink && (
            <DevicePicker
              flow="render"
              slot={meta.sink}
              value={data.sink}
              hideCableDevices
              hideDisabled
              onChange={(v) => setDevice(id, "sink", v)}
            />
          )}
          {meta.source && (
            <DevicePicker
              flow="capture"
              slot={meta.source}
              value={data.source}
              hideCableDevices
              hideDisabled
              onChange={(v) => setDevice(id, "source", v)}
            />
          )}
        </div>
      )}

      {meta.pickApp && (
        <div className="ag-node__devices">
          <AppPicker value={data.app} onChange={(app) => setApp(id, app)} />
        </div>
      )}

      {meta.showDefaults && <SystemDefaults />}

      {showVolume && meta.maxVolume && <VolumeSlider id={id} value={data.volume ?? 1} max={meta.maxVolume} />}

      {showDelay && meta.delay && <DelaySlider id={id} value={data.delay ?? 0} limit={delayLimit} />}

      {notes.length > 0 && (
        <div className="ag-node__notes">
          {notes.map((n, i) => (
            <div key={i} className={`ag-node__note is-${n.level}`}>
              {n.text}
            </div>
          ))}
        </div>
      )}

      {meta.hasOutput && (
        <Handle type="source" position={Position.Right} id="out" className="ag-port" />
      )}
    </div>
  );
}

export default memo(AudioNodeView);
