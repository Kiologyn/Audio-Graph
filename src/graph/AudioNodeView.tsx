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
import { Popover } from "./Popover";
import { nodeNotes } from "./problems";
import { useGraph } from "./store";
import { NODE_KINDS, nodeSliders, type AudioNode, type NodeSlider } from "./types";

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
 * A slider's value: a click opens it for typing, a double click resets it. Enter or leaving the field
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
  onReset: () => void;
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
        onReset();
      }}
    >
      {children}
    </span>
  );
}

/** A node's volume slider */
function VolumeSlider({ id, value, max }: { id: string; value: number; max: number }) {
  const setVolume = useGraph((s) => s.setVolume);
  const send = useLiveSend((graphId, volume) => useEngine.getState().volume(graphId, id, volume));
  const t = useT();
  const change = (volume: number) => {
    setVolume(id, volume);
    send(volume);
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
      <SliderValue
        className={`ag-volume__value${percent > 100 ? " is-boost" : ""}`}
        draft={String(percent)}
        onChange={(typed) => change(Math.min(max, Math.max(0, Math.round(typed) / 100)))}
        onReset={() => change(1)}
      >
        {percent}%
      </SliderValue>
    </div>
  );
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
 * A node's delay. An output's goes from minus the range to plus, no delay in the middle; a cable's (hold)
 * from no delay to plus. The engine gets it with autosave: a delay change restarts the sound for a moment
 * anyway, so there is no live sending. Double-clicking the value resets it
 */
function DelaySlider({ id, value, limit, hold }: { id: string; value: number; limit: number; hold: boolean }) {
  const setDelay = useGraph((s) => s.setDelay);
  const language = useSettings((s) => s.language);
  const t = useT();
  const min = hold ? 0 : -limit;
  // a delay beyond a narrowed range acts as the range's edge, in the engine too
  const shown = Math.min(limit, Math.max(min, value));
  const seconds = (v: number, digits: number) =>
    t("delay_seconds", {
      value: new Intl.NumberFormat(language, {
        minimumFractionDigits: digits,
        maximumFractionDigits: 3,
        signDisplay: "exceptZero",
      }).format(v),
    });
  // where a value is on the slider, 0..1: the fill goes from no delay to the knob, the value is written under it
  const position = (v: number) => (v - min) / (limit - min);
  const [at, zero] = [position(shown), position(0)];
  return (
    <div className="ag-delay nodrag" title={t(hold ? "cable_delay_hint" : "delay_hint")}>
      <span className="ag-delay__edge">{seconds(min, 0)}</span>
      <div className="ag-delay__slider" style={{ "--at": at } as CSSProperties}>
        <input
          type="range"
          min={min * 1000}
          max={limit * 1000}
          step={1}
          value={Math.round(shown * 1000)}
          style={{ "--from": `${Math.min(at, zero) * 100}%`, "--to": `${Math.max(at, zero) * 100}%` } as CSSProperties}
          onChange={(e) => setDelay(id, Number(e.currentTarget.value) / 1000)}
        />
        <SliderValue
          className={`ag-delay__value${shown !== 0 ? " is-set" : ""}`}
          draft={new Intl.NumberFormat(language, { maximumFractionDigits: 3, useGrouping: false }).format(shown)}
          onChange={(typed) => setDelay(id, Math.min(limit, Math.max(min, Math.round(typed * 1000) / 1000)))}
          onReset={() => setDelay(id, 0)}
        >
          {seconds(shown, 3)}
        </SliderValue>
      </div>
      <span className="ag-delay__edge">{seconds(limit, 0)}</span>
    </div>
  );
}

const SLIDER_NAMES: Record<NodeSlider, string> = {
  volume: "node_option_volume",
  balance: "node_option_balance",
  delay: "node_option_delay",
};

/** One of a node's sliders, the same in its menu and pinned to the node */
function SliderOf({ id, data, slider }: { id: string; data: AudioNode["data"]; slider: NodeSlider }) {
  const meta = NODE_KINDS[data.kind];
  const delayLimit = useGraph((s) => s.meta?.delayLimit ?? 0.5);
  if (slider === "volume") return <VolumeSlider id={id} value={data.volume ?? 1} max={meta.maxVolume ?? 1} />;
  if (slider === "balance") return <BalanceSlider id={id} value={data.balance ?? 0} />;
  return <DelaySlider id={id} value={data.delay ?? 0} limit={delayLimit} hold={meta.delay === "hold"} />;
}

/** The arrow in a node's corner: a menu with its sliders, each can be pinned to the node. A node without sliders has no arrow */
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
        <Icon name="chevron" size={12} />
      </button>
      {open && (
        <Popover anchor={anchor} onClose={() => setOpen(false)}>
          <div className={`ag-node-options ag-node--${data.kind}`}>
            {sliders.map((slider) => {
              const pinned = data.pinned.includes(slider);
              return (
                <div key={slider} className="ag-node-options__row">
                  <div className="ag-node-options__head">
                    <span>{t(SLIDER_NAMES[slider])}</span>
                    <button
                      className={`ag-node-options__pin${pinned ? " is-on" : ""}`}
                      title={t(pinned ? "node_unpin" : "node_pin")}
                      aria-pressed={pinned}
                      onClick={() => togglePinned(id, slider)}
                    >
                      <Icon name="pin" size={14} />
                    </button>
                  </div>
                  <SliderOf id={id} data={data} slider={slider} />
                </div>
              );
            })}
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
        <Handle type="source" position={Position.Right} id="out" className="ag-port" />
      )}
    </div>
  );
}

export default memo(AudioNodeView);
