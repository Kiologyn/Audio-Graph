import { Handle, Position, type NodeProps } from "@xyflow/react";
import { memo, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useT } from "../i18n";
import { dynamicCables, virtualDeviceName } from "../platform";
import { useSettings } from "../settings";
import { AppPicker } from "./AppPicker";
import { CablePicker } from "./CablePicker";
import { useCables } from "./cables";
import { DevicePicker } from "./DevicePicker";
import { useDevices } from "./devices";
import { useEngine } from "./engine";
import { Icon, KindIcon } from "./icons";
import { nextMeter, toDb, watchLevel, type Meter } from "./levels";
import { Popover } from "./Popover";
import { nodeNotes } from "./problems";
import { useGraph } from "./store";
import { useSystemVolumes } from "./systemVolume";
import { NODE_KINDS, NO_GATE, nodeSliders, type AudioNode, type NodeSlider } from "./types";
import { totalVolume } from "./volume";

/** The system's default devices now; while a graph with this node runs, it sets them */
function SystemDefaults() {
  const devices = useDevices((s) => s.devices);
  const t = useT();
  const name = (flow: "render" | "capture") => devices.find((d) => d.flow === flow && d.isDefault)?.name ?? t("none");
  return (
    <dl className="ag-node__defaults" title={t("system_node_hint")}>
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
    <div className="ag-node__virtual" title={t("virtual_cable_hint")}>
      <span>{virtualDeviceName(label)}</span>
    </div>
  );
}

/** Sends a slider's value to the engine at most once per 50 ms: dragging fires far more events */
function useLiveSend(send: (graphId: string, value: number) => void) {
  const graphId = useGraph((s) => s.meta?.id);
  const last = useRef(0);
  const pending = useRef<number | null>(null);
  return (value: number) => {
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
function SliderValue({
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

/** A volume slider with a mark at 100%. Without a value there is nothing to move: the system has nothing to show */
function VolumeBar({
  value,
  max,
  hint,
  total,
  unset,
  onChange,
  onReset,
}: {
  value: number | null;
  max: number;
  hint: string;
  total: string | null;
  /** The value only shows what the system has now, the graph doesn't set it */
  unset?: boolean;
  onChange: (volume: number) => void;
  onReset?: () => void;
}) {
  const t = useT();
  // a volume beyond a lowered maximum acts as the maximum, in the engine too
  const percent = Math.round(Math.min(value ?? 0, max) * 100);
  return (
    <div className={`ag-volume nodrag${value === null ? " is-off" : ""}`} title={hint}>
      <div
        className={`ag-volume__track${max > 1 ? " has-mark" : ""}`}
        style={{ "--mark": 1 / max, "--fill": `${(percent / (max * 100)) * 100}%` } as CSSProperties}
      >
        <input
          type="range"
          min={0}
          max={max * 100}
          step={1}
          value={percent}
          disabled={value === null}
          onChange={(e) => onChange(Number(e.currentTarget.value) / 100)}
        />
      </div>
      {value === null ? (
        <span className="ag-volume__value">—</span>
      ) : (
        <SliderValue
          className={`ag-volume__value${percent > 100 ? " is-boost" : ""}${unset ? " is-unset" : ""}`}
          draft={String(percent)}
          onChange={(typed) => onChange(Math.min(max, Math.max(0, Math.round(typed) / 100)))}
          onReset={onReset}
        >
          {percent}%
        </SliderValue>
      )}
      {total && (
        <span className="ag-volume__total" title={t("volume_total_hint")}>
          → {total}
        </span>
      )}
    </div>
  );
}

/** A cable's volume: the engine applies it */
function CableVolume({ id, value, max }: { id: string; value: number; max: number }) {
  const setVolume = useGraph((s) => s.setVolume);
  const send = useLiveSend((graphId, volume) => useEngine.getState().volume(graphId, id, volume));
  const t = useT();
  const change = (volume: number) => {
    setVolume(id, volume);
    send(volume);
  };
  return <VolumeBar value={value} max={max} hint={t("volume_hint")} total={null} onChange={change} onReset={() => change(Math.min(1, max))} />;
}

/**
 * The system's slider of a device or app as a setting of the graph: it reaches the system only while the
 * graph runs, and what it was goes back when the graph stops. Without a setting the graph leaves the slider
 * alone and only shows it; a double click returns to that, it must not make a device loud
 */
function SystemVolume({ id, data }: { id: string; data: AudioNode["data"] }) {
  const live = useSystemVolumes((s) => s.volumes[id] ?? null);
  const volumes = useSystemVolumes((s) => s.volumes);
  const total = useGraph((s) => (data.kind === "output" ? totalVolume(id, s.nodes, s.edges, s.meta?.volumeLimit ?? 2, volumes) : null));
  const graphId = useGraph((s) => s.meta?.id);
  const running = useEngine((s) => !!graphId && s.status.running.includes(graphId));
  const setSystemVolume = useGraph((s) => s.setSystemVolume);
  const send = useLiveSend((graph, volume) => useEngine.getState().volume(graph, id, volume));
  const t = useT();
  const set = data.systemVolume ?? null;
  const value = running ? (live ?? set) : (set ?? live);
  const change = (volume: number) => {
    setSystemVolume(id, volume);
    if (running) {
      useSystemVolumes.getState().moved(id, volume);
      send(volume);
    }
  };
  const hint = value === null ? "volume_unavailable_hint" : set === null ? "system_volume_unset_hint" : "system_volume_hint";
  return <VolumeBar value={value} max={1} hint={t(hint)} total={total} unset={set === null} onChange={change} onReset={() => setSystemVolume(id, null)} />;
}

/** A node's left-right balance: the middle is even, the fill goes from it to the knob. Double-clicking the value centers it */
function BalanceSlider({ id, value }: { id: string; value: number }) {
  const setBalance = useGraph((s) => s.setBalance);
  const send = useLiveSend((graphId, balance) => useEngine.getState().balance(graphId, id, balance));
  const t = useT();
  const change = (balance: number) => {
    setBalance(id, balance);
    send(balance);
  };
  const percent = Math.round(value * 100);
  const at = 0.5 + value / 2;
  const text = percent === 0 ? "0" : t(percent < 0 ? "balance_left" : "balance_right", { percent: Math.abs(percent) });
  return (
    <div className="ag-volume ag-balance nodrag" title={t("balance_hint")}>
      <span className="ag-balance__edge">{t("balance_left_short")}</span>
      <input
        type="range"
        min={-100}
        max={100}
        step={1}
        value={percent}
        style={{ "--from": `${Math.min(at, 0.5) * 100}%`, "--to": `${Math.max(at, 0.5) * 100}%` } as CSSProperties}
        onChange={(e) => change(Number(e.currentTarget.value) / 100)}
      />
      <span className="ag-balance__edge">{t("balance_right_short")}</span>
      <SliderValue
        className={`ag-volume__value${percent !== 0 ? " is-boost" : ""}`}
        draft={String(percent)}
        onChange={(typed) => change(Math.min(100, Math.max(-100, Math.round(typed))) / 100)}
        onReset={() => change(0)}
      >
        {text}
      </SliderValue>
    </div>
  );
}

/**
 * A node's delay. An output's goes from minus the range to none; a cable's (hold) from none to plus.
 * The engine gets it with autosave: a delay change restarts the sound for a moment
 * anyway, so there is no live sending. Double-clicking the value resets it
 */
function DelaySlider({ id, value, limit, hold }: { id: string; value: number; limit: number; hold: boolean }) {
  const setDelay = useGraph((s) => s.setDelay);
  const language = useSettings((s) => s.language);
  const t = useT();
  const [min, max] = hold ? [0, limit] : [-limit, 0];
  // a delay beyond a narrowed range acts as the range's edge, in the engine too
  const shown = Math.min(max, Math.max(min, value));
  const seconds = (v: number, digits: number) =>
    t("delay_seconds", {
      value: new Intl.NumberFormat(language, {
        minimumFractionDigits: digits,
        maximumFractionDigits: 3,
        signDisplay: "exceptZero",
      }).format(v),
    });
  // where a value is on the slider, 0..1: the fill goes from no delay to the knob, the value is written under it
  const position = (v: number) => (v - min) / (max - min);
  const [at, zero] = [position(shown), position(0)];
  return (
    <div className="ag-delay nodrag" title={t(hold ? "cable_delay_hint" : "delay_hint")}>
      <span className="ag-delay__edge">{seconds(min, 0)}</span>
      <div className="ag-delay__slider" style={{ "--at": at } as CSSProperties}>
        <input
          type="range"
          min={min * 1000}
          max={max * 1000}
          step={1}
          value={Math.round(shown * 1000)}
          style={{ "--from": `${Math.min(at, zero) * 100}%`, "--to": `${Math.max(at, zero) * 100}%` } as CSSProperties}
          onChange={(e) => setDelay(id, Number(e.currentTarget.value) / 1000)}
        />
        <SliderValue
          className={`ag-delay__value${shown !== 0 ? " is-set" : ""}`}
          draft={new Intl.NumberFormat(language, { maximumFractionDigits: 3, useGrouping: false }).format(shown)}
          onChange={(typed) => setDelay(id, Math.min(max, Math.max(min, Math.round(typed * 1000) / 1000)))}
          onReset={() => setDelay(id, 0)}
        >
          {seconds(shown, 3)}
        </SliderValue>
      </div>
      <span className="ag-delay__edge">{seconds(max, 0)}</span>
    </div>
  );
}

/** Where a level in dB is on a noise gate's slider, 0..1 */
const gatePosition = (db: number) => (db - NO_GATE) / -NO_GATE;

/**
 * The live part of a noise gate's slider: what comes into the cable, lit up to its level. Nothing here goes
 * through React: a tick only moves two layers, which the GPU does without repainting, and only if the level changed
 */
function useGateMeter(id: string, threshold: number, active: boolean) {
  const row = useRef<HTMLDivElement>(null);
  const reach = useRef<HTMLDivElement>(null);
  const sound = useRef<HTMLDivElement>(null);
  const gate = useRef(threshold);
  gate.current = threshold;
  useEffect(() => {
    // refs are empty by the time a slider being taken off the node cleans up
    const [line, bar, inner] = [row.current, reach.current, sound.current];
    if (!active || !line || !bar || !inner) return;
    let meter: Meter = { db: NO_GATE, openUntil: 0 };
    let shown = 0;
    const show = (at: number) => {
      // along the knob's center line, like the threshold; nothing at all in silence
      const edge = at > 0 ? `(6px + (100% - 12px) * ${at})` : "0px";
      bar.style.transform = `translateX(calc(-100% + ${edge}))`;
      inner.style.transform = `translateX(calc(100% - ${edge}))`;
    };
    const stop = watchLevel(id, (peak) => {
      const now = performance.now();
      meter = nextMeter(meter, toDb(peak, NO_GATE), gate.current, now, NO_GATE);
      const at = Math.round(gatePosition(meter.db) * 1000) / 1000;
      if (at !== shown) show((shown = at));
      const open = now < meter.openUntil && gate.current > NO_GATE;
      if ((line.dataset.open === "") !== open) {
        if (open) line.dataset.open = "";
        else delete line.dataset.open;
      }
    });
    return () => {
      stop();
      show(0);
      delete line.dataset.open;
    };
  }, [id, active]);
  return { row, reach, sound };
}

/**
 * A cable's noise gate, like input sensitivity in voice chats. The knob is the threshold; the rail is built like the
 * other sliders, with the sound in it: what stays below the knob is grey, what gets past it takes the node's color,
 * and the knob fills up while the gate is open. Double-clicking the value turns it off
 */
function GateSlider({ id, value }: { id: string; value: number }) {
  const setGate = useGraph((s) => s.setGate);
  const send = useLiveSend((graphId, gate) => useEngine.getState().gate(graphId, id, gate));
  const graphId = useGraph((s) => s.meta?.id);
  const running = useEngine((s) => !!graphId && s.status.running.includes(graphId));
  const t = useT();
  const change = (gate: number) => {
    setGate(id, gate);
    send(gate);
  };
  const db = Math.round(Math.min(0, Math.max(NO_GATE, value)));
  const meter = useGateMeter(id, db, running);
  return (
    <div ref={meter.row} className="ag-volume ag-gate nodrag" title={t("gate_hint")}>
      <div className="ag-volume__track" style={{ "--at": gatePosition(db) } as CSSProperties}>
        <div className="ag-gate__meter">
          <div ref={meter.reach} className="ag-gate__window">
            <div ref={meter.sound} className="ag-gate__sound" />
          </div>
        </div>
        <input type="range" min={NO_GATE} max={0} step={1} value={db} onChange={(e) => change(Number(e.currentTarget.value))} />
      </div>
      <SliderValue
        className={`ag-volume__value${db > NO_GATE ? " is-boost" : ""}`}
        draft={String(db)}
        onChange={(typed) => change(Math.min(0, Math.max(NO_GATE, Math.round(typed))))}
        onReset={() => change(NO_GATE)}
      >
        {db > NO_GATE ? t("gate_value", { value: db }) : t("gate_off")}
      </SliderValue>
    </div>
  );
}

const SLIDER_NAMES: Record<NodeSlider, string> = {
  volume: "node_option_volume",
  balance: "node_option_balance",
  delay: "node_option_delay",
  gate: "node_option_gate",
};

/** One of the sliders shown on a node */
function SliderOf({ id, data, slider }: { id: string; data: AudioNode["data"]; slider: NodeSlider }) {
  const meta = NODE_KINDS[data.kind];
  const delayLimit = useGraph((s) => s.meta?.delayLimit ?? 0.5);
  const volumeLimit = useGraph((s) => s.meta?.volumeLimit ?? 2);
  if (slider === "volume") {
    if (meta.systemVolume) return <SystemVolume id={id} data={data} />;
    return <CableVolume id={id} value={data.volume ?? 1} max={Math.min(meta.maxVolume ?? 1, volumeLimit)} />;
  }
  if (slider === "balance") return <BalanceSlider id={id} value={data.balance ?? 0} />;
  if (slider === "gate") return <GateSlider id={id} value={data.gate ?? NO_GATE} />;
  return <DelaySlider id={id} value={data.delay ?? 0} limit={delayLimit} hold={meta.delay === "hold"} />;
}

/** The arrow in a node's corner: a menu with a checkbox per slider, a checked one is shown on the node. A node without sliders has no arrow */
function NodeOptions({ id, data }: { id: string; data: AudioNode["data"] }) {
  const togglePinned = useGraph((s) => s.togglePinned);
  const sliders = nodeSliders(data.kind);
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const t = useT();
  if (sliders.length === 0) return null;
  return (
    <>
      <button
        ref={anchor}
        className={`ag-node__options nodrag${open ? " is-open" : ""}`}
        title={t("node_options")}
        onClick={() => setOpen(!open)}
      >
        <Icon name="sliders" size={14} />
      </button>
      {open && (
        <Popover anchor={anchor} minimum={0} onClose={() => setOpen(false)}>
          <div className={`ag-node-options ag-node--${data.kind}`}>
            {sliders.map((slider) => (
              <label key={slider} className="ag-node-options__check">
                <input type="checkbox" checked={data.pinned.includes(slider)} onChange={() => togglePinned(id, slider)} />
                {t(SLIDER_NAMES[slider])}
              </label>
            ))}
          </div>
        </Popover>
      )}
    </>
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
        <NodeOptions id={id} data={data} />
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

      {nodeSliders(data.kind)
        .filter((slider) => data.pinned.includes(slider))
        .map((slider) => (
          <div key={slider} className="ag-node__slider">
            <span>{t(SLIDER_NAMES[slider])}</span>
            <SliderOf id={id} data={data} slider={slider} />
          </div>
        ))}

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
        <Handle
          type="source"
          position={Position.Right}
          id="out"
          className="ag-port"
          title={data.kind === "output" ? t("port_output_hint") : undefined}
        />
      )}
    </div>
  );
}

export default memo(AudioNodeView);
