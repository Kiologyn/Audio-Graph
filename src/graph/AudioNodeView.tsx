import { Handle, Position, type NodeProps } from "@xyflow/react";
import { memo, useMemo, useRef, useState, type CSSProperties } from "react";
import { useT } from "../i18n";
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

/** Устройства по умолчанию в Windows сейчас; пока граф с этой нодой запущен, их назначает он */
function SystemDefaults() {
  const devices = useDevices((s) => s.devices);
  const t = useT();
  const name = (flow: "render" | "capture") => devices.find((d) => d.flow === flow && d.isDefault)?.name ?? t("нет");
  return (
    <dl
      className="ag-node__defaults"
      title={t(
        "Выход ноды — звук приложений, которые играют в устройство вывода по умолчанию. Вход — то, что они слышат как микрофон по умолчанию. Пока граф запущен, устройства по умолчанию назначает он, после остановки возвращаются прежние",
      )}
    >
      <dt>{t("Вывод")}</dt>
      <dd>{name("render")}</dd>
      <dt>{t("Ввод")}</dt>
      <dd>{name("capture")}</dd>
    </dl>
  );
}

/** Ползунок громкости ноды. Двойной клик по проценту — снова 100% */
function VolumeSlider({ id, value, max }: { id: string; value: number; max: number }) {
  const setVolume = useGraph((s) => s.setVolume);
  const graphId = useGraph((s) => s.meta?.id);
  const t = useT();
  // движку — не чаще раза в 50 мс: при перетаскивании событий намного больше
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
    <div className="ag-volume nodrag" title={t("Громкость. Двойной клик по проценту — 100%")}>
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
  const t = useT();
  // t в зависимостях: тексты пометок переводятся при смене языка
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
        <div className="ag-node__label" onDoubleClick={() => setEditing(true)} title={t("Двойной клик — переименовать")}>
          {data.label}
        </div>
      )}

      {meta.pickCable && (
        <div className="ag-node__devices">
          <CablePicker value={data.cable} onChange={(cable) => setCable(id, cable)} />
        </div>
      )}

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
