import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";
import { create } from "zustand";
import { useT } from "../i18n";
import { useSettings } from "../settings";
import { MAX_DELAY_LIMIT } from "./doc";
import { describeDevice, useDevices } from "./devices";
import { DevicePicker } from "./DevicePicker";
import { useLibrary } from "./library";
import { useGraph } from "./store";
import type { AudioNode, DeviceRef } from "./types";

/** Whether the delay calibration window is open; it works with the graph open on the canvas */
export const useCalibration = create<{ open: boolean; setOpen: (open: boolean) => void }>()((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));

/** Matches Measurement in src-tauri/src/calibrate/mod.rs */
type Measurement = { latency: number; spread: number };

type Row =
  | { state: "waiting" }
  | { state: "measuring" }
  | { state: "done"; result: Measurement }
  | { state: "failed"; error: string };

/** Tries that differ by more than this make a measurement unreliable */
const UNSTEADY = 0.02;

/** An output that can't be calibrated, and why (translation key); null if it can */
function unavailable(node: AudioNode, nodes: AudioNode[], edges: { source: string; target: string }[]): string | null {
  if (!node.data.sink) return "calibration_no_device";
  // the system plays such sound, the engine can't delay it
  const direct = edges.some(
    (e) => e.target === node.id && ["app", "system"].includes(nodes.find((n) => n.id === e.source)?.data.kind ?? ""),
  );
  return direct ? "calibration_direct" : null;
}

/** Engine silence for a measurement: the microphone should hear only the test sound */
const quiet = (seconds: number) => invoke("engine_quiet", { seconds }).catch(() => {});

/**
 * Delay calibration: every chosen output plays a short sweep in turn, a microphone hears it, and the
 * difference in latencies becomes the outputs' delays. The fastest output gets none, the late ones a negative one
 */
export function Calibration() {
  const open = useCalibration((s) => s.open);
  const setOpen = useCalibration((s) => s.setOpen);
  const close = useCallback(() => setOpen(false), [setOpen]);
  if (!open) return null;
  return <CalibrationDialog onClose={close} />;
}

function CalibrationDialog({ onClose }: { onClose: () => void }) {
  const meta = useGraph((s) => s.meta);
  const nodes = useGraph((s) => s.nodes);
  const edges = useGraph((s) => s.edges);
  const setDelays = useGraph((s) => s.setDelays);
  const setPrefs = useLibrary((s) => s.setPrefs);
  const devices = useDevices((s) => s.devices);
  const loaded = useDevices((s) => s.loaded);
  const language = useSettings((s) => s.language);
  const t = useT();

  const outputs = useMemo(() => nodes.filter((n) => n.data.kind === "output"), [nodes]);
  const [mic, setMic] = useState<DeviceRef | null>({ type: "default" });
  const [chosen, setChosen] = useState<Set<string>>(
    () => new Set(outputs.filter((n) => !unavailable(n, nodes, edges)).map((n) => n.id)),
  );
  const [rows, setRows] = useState<Map<string, Row> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // the microphone list is fresh
  useEffect(() => {
    useDevices.getState().refresh();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // the first Escape closes an open device list
      if (e.key === "Escape" && !busy && !document.querySelector(".ag-popover")) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  if (!meta) return null;

  const ms = (seconds: number) =>
    t("calibration_ms", { value: new Intl.NumberFormat(language, { maximumFractionDigits: 0 }).format(seconds * 1000) });
  const secondsText = (seconds: number) =>
    t("delay_seconds", {
      value: new Intl.NumberFormat(language, { minimumFractionDigits: 3, maximumFractionDigits: 3, signDisplay: "exceptZero" }).format(
        seconds,
      ),
    });

  const measured = [...(rows ?? [])].flatMap(([id, row]) => (row.state === "done" ? [{ id, ...row.result }] : []));
  const fastest = Math.min(...measured.map((m) => m.latency));
  // the fastest output plays as it is, the late ones get ahead by as much as they lag:
  // the engine turns negative delays into waiting on the other outputs
  const delays = new Map(measured.map((m) => [m.id, Math.round((fastest - m.latency) * 1000) / 1000]));
  const needed = Math.max(0, ...[...delays.values()].map((d) => -d));
  const limit = meta.delayLimit;

  const measure = async (ids: string[]) => {
    if (!mic || ids.length === 0) return;
    setBusy(true);
    setError(null);
    setRows((prev) => {
      const next = new Map(prev ?? []);
      for (const id of ids) next.set(id, { state: "waiting" });
      return next;
    });
    try {
      for (const id of ids) {
        const node = outputs.find((n) => n.id === id);
        if (!node?.data.sink) continue;
        setRows((prev) => new Map(prev).set(id, { state: "measuring" }));
        await quiet(15);
        try {
          const result = await invoke<Measurement>("calibrate_measure", { output: node.data.sink, mic });
          setRows((prev) => new Map(prev).set(id, { state: "done", result }));
        } catch (e) {
          setRows((prev) => new Map(prev).set(id, { state: "failed", error: String(e) }));
        }
      }
    } finally {
      await quiet(0);
      setBusy(false);
    }
  };

  const listen = async () => {
    setBusy(true);
    setError(null);
    try {
      await quiet(5);
      const list = measured.flatMap((m) => {
        const sink = outputs.find((n) => n.id === m.id)?.data.sink;
        // ticks can only wait: the late outputs start first
        return sink ? [{ device: sink, delay: (delays.get(m.id) ?? 0) + needed }] : [];
      });
      await invoke("calibrate_preview", { outputs: list });
    } catch (e) {
      setError(String(e));
    } finally {
      await quiet(0);
      setBusy(false);
    }
  };

  const apply = () => {
    setDelays(Object.fromEntries([...delays].map(([id, delay]) => [id, Math.max(delay, -MAX_DELAY_LIMIT)])));
    // the sliders must show the delays: the range wide enough
    const range = needed > limit ? Math.min(MAX_DELAY_LIMIT, Math.ceil(needed * 20) / 20) : limit;
    if (range !== limit) setPrefs(meta.id, { delayLimit: range });
    onClose();
  };

  const status = (id: string) => {
    const row = rows?.get(id);
    if (!row) return null;
    if (row.state === "waiting") return <span className="ag-calibration__status">{t("calibration_waiting")}</span>;
    if (row.state === "measuring") return <span className="ag-calibration__status is-active">{t("calibration_measuring")}</span>;
    if (row.state === "failed") return <span className="ag-calibration__status is-error" title={row.error}>{row.error}</span>;
    return (
      <span
        className={`ag-calibration__status is-done${row.result.spread > UNSTEADY ? " is-unsteady" : ""}`}
        title={row.result.spread > UNSTEADY ? t("calibration_unsteady", { spread: ms(row.result.spread) }) : undefined}
      >
        {ms(row.result.latency)}
        {/* a single measurement has nothing to be compared with */}
        {measured.length >= 2 && delays.has(id) && <b>{secondsText(delays.get(id)!)}</b>}
      </span>
    );
  };

  const finished = rows !== null && !busy;
  const measureChosen = () =>
    measure(outputs.filter((n) => chosen.has(n.id) && !unavailable(n, nodes, edges)).map((n) => n.id));
  const failed = [...(rows ?? [])].filter(([, r]) => r.state === "failed").map(([id]) => id);

  return (
    <div className="ag-modal" onPointerDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div className="ag-modal__box ag-calibration" role="dialog" aria-label={t("calibration_title")}>
        <h2 className="ag-modal__title">{t("calibration_title")}</h2>
        <p className="ag-modal__text">{t("calibration_hint")}</p>

        <div className="ag-calibration__mic">
          <span>{t("calibration_mic")}</span>
          <DevicePicker
            flow="capture"
            slot={{ placeholder: "placeholder_no_device", allowDefault: true }}
            value={mic}
            onChange={setMic}
          />
        </div>

        <div className="ag-calibration__outputs">
          {outputs.length === 0 && <div className="ag-calibration__empty">{t("calibration_no_outputs")}</div>}
          {outputs.map((n) => {
            const why = unavailable(n, nodes, edges);
            const device = describeDevice(n.data.sink, "render", devices, loaded).text;
            return (
              <label key={n.id} className={`ag-calibration__output${why ? " is-disabled" : ""}`}>
                <input
                  type="checkbox"
                  disabled={!!why || busy}
                  checked={!why && chosen.has(n.id)}
                  onChange={(e) => {
                    const next = new Set(chosen);
                    if (e.currentTarget.checked) next.add(n.id);
                    else next.delete(n.id);
                    setChosen(next);
                  }}
                />
                <span className="ag-calibration__name">
                  {n.data.label}
                  <small>{why ? t(why) : device}</small>
                </span>
                {status(n.id)}
              </label>
            );
          })}
        </div>

        {finished && measured.length >= 2 && needed > limit && (
          <p className="ag-modal__note">
            {needed > MAX_DELAY_LIMIT
              ? t("calibration_too_far", { max: MAX_DELAY_LIMIT })
              : t("calibration_widen", { range: Math.min(MAX_DELAY_LIMIT, Math.ceil(needed * 20) / 20) })}
          </p>
        )}
        {finished && measured.length < 2 && measured.length + failed.length > 0 && (
          <p className="ag-modal__note">{t("calibration_need_two")}</p>
        )}
        {error && <p className="ag-modal__note is-error">{error}</p>}

        <div className="ag-modal__buttons">
          <button className="ag-button" disabled={busy} onClick={onClose}>
            {t("cancel")}
          </button>
          {finished && measured.length >= 2 && (
            <button className="ag-button" disabled={busy} onClick={listen} title={t("calibration_listen_hint")}>
              {t("calibration_listen")}
            </button>
          )}
          {finished && failed.length > 0 && (
            <button className="ag-button" disabled={busy} onClick={() => measure(failed)}>
              {t("calibration_retry_failed")}
            </button>
          )}
          {finished && measured.length >= 2 ? (
            <>
              <button className="ag-button" disabled={busy || !mic} onClick={measureChosen}>
                {t("calibration_again")}
              </button>
              <button className="ag-button is-primary" disabled={busy} onClick={apply}>
                {t("calibration_apply")}
              </button>
            </>
          ) : (
            <button className="ag-button is-primary" disabled={busy || !mic || chosen.size < 2} onClick={measureChosen}>
              {busy ? t("calibration_measuring") : rows ? t("calibration_again") : t("calibration_start")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
