import { useEffect, useMemo, useRef, useState } from "react";
import {
  cableDeviceOwners,
  cablePower,
  cableDeviceIds,
  idsToToggle,
  useCablePanel,
  useCables,
  type Cable,
} from "./cables";
import { useT } from "../i18n";
import { describeDevice, useDevices } from "./devices";
import { DevicePicker } from "./DevicePicker";
import { Icon, KindIcon } from "./icons";
import type { DeviceSlot } from "./types";

const CABLE_SLOT: DeviceSlot = { placeholder: "cable_slot_not_selected", allowDefault: false };

// translation keys
const POWER_TITLE = {
  on: "cable_power_on",
  off: "cable_power_off",
  mixed: "cable_power_mixed",
  none: "cable_power_none",
};

/** Cable switch: turns both of its devices on and off in the system */
function PowerSwitch({ cable }: { cable: Cable }) {
  const devices = useDevices((s) => s.devices);
  const pending = useDevices((s) => s.pending);
  const setEnabled = useDevices((s) => s.setEnabled);
  const t = useT();
  const power = cablePower(cable, devices);
  const busy = cableDeviceIds(cable).some((id) => pending.includes(id));
  // a cable that is off or partly off turns fully on when clicked
  const enable = power !== "on";

  return (
    <button
      className={`ag-switch is-${power}${busy ? " is-busy" : ""}`}
      role="switch"
      aria-checked={power === "on"}
      disabled={power === "none" || busy}
      title={t(POWER_TITLE[power])}
      onClick={() => setEnabled(idsToToggle([cable], devices, enable), enable)}
    />
  );
}

/** Turn all cables on or off at once */
function BulkPower({ cables }: { cables: Cable[] }) {
  const devices = useDevices((s) => s.devices);
  const pending = useDevices((s) => s.pending);
  const setEnabled = useDevices((s) => s.setEnabled);
  const toEnable = idsToToggle(cables, devices, true);
  const toDisable = idsToToggle(cables, devices, false);
  const busy = pending.length > 0;
  const t = useT();

  return (
    <div className="ag-cables__bulk">
      <span>{t("all_cables")}</span>
      <button
        className="ag-panel__small"
        disabled={busy || toEnable.length === 0}
        onClick={() => setEnabled(toEnable, true)}
      >
        {t("turn_on")}
      </button>
      <button
        className="ag-panel__small"
        disabled={busy || toDisable.length === 0}
        onClick={() => setEnabled(toDisable, false)}
      >
        {t("turn_off")}
      </button>
    </div>
  );
}

function CableRow({ cable, onEdit, onDelete }: { cable: Cable; onEdit: () => void; onDelete: () => void }) {
  const devices = useDevices((s) => s.devices);
  const loaded = useDevices((s) => s.loaded);
  const t = useT();
  const sides = [
    { caption: "cable_in", d: describeDevice(cable.render, "render", devices, loaded) },
    { caption: "cable_out", d: describeDevice(cable.capture, "capture", devices, loaded) },
  ];

  return (
    <div className="ag-panel__row">
      <button className="ag-cables__info" onClick={onEdit} title={t("cable_setup_hint")}>
        <span className="ag-cables__name">{cable.name}</span>
        {sides.map(({ caption, d }) => (
          <span
            key={caption}
            className={`ag-cables__side${d.missing ? " is-missing" : ""}${d.text ? "" : " is-empty"}`}
          >
            {t(caption)}: {d.text ?? t("not_selected")}
            {d.missing && t("device_unavailable_suffix")}
            {d.disabled && t("device_disabled_suffix")}
          </span>
        ))}
      </button>
      <PowerSwitch cable={cable} />
      <button className="ag-panel__action" title={t("cable_setup")} onClick={onEdit}>
        <Icon name="pencil" />
      </button>
      <button className="ag-panel__action is-danger" title={t("delete")} onClick={onDelete}>
        <Icon name="trash" />
      </button>
    </div>
  );
}

/** An expanded cable: name and two devices. Everything is saved at once, ✓ only collapses it. */
function CableEditor({ cable, onDone }: { cable: Cable; onDone: () => void }) {
  const update = useCables((s) => s.update);
  const cables = useCables((s) => s.cables);
  const t = useT();
  // devices already taken by other cables are labeled, so two cables aren't built from one device
  const taken = useMemo(() => {
    const owners = cableDeviceOwners(cables.filter((c) => c.id !== cable.id));
    return new Map([...owners].map(([id, name]) => [id, t("device_in_cable", { name })]));
  }, [cables, cable.id, t]);
  const draft = useRef(cable.name);
  const cancelled = useRef(false);

  const commitName = () => {
    if (cancelled.current) return;
    const name = draft.current.trim();
    const current = useCables.getState().cables.find((c) => c.id === cable.id);
    if (current && name && name !== current.name) useCables.getState().update(cable.id, { name });
  };

  // the panel can be closed by a click outside, and the field just disappears: save the name on unmount
  useEffect(() => () => commitName(), []);

  return (
    <div className="ag-cables__editor">
      <div className="ag-cables__editor-head">
        <input
          className="ag-panel__input"
          autoFocus
          defaultValue={cable.name}
          maxLength={100}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => (draft.current = e.currentTarget.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              commitName();
              onDone();
            }
            if (e.key === "Escape") {
              // Escape cancels the name edit and collapses the cable instead of closing the panel
              e.stopPropagation();
              cancelled.current = true;
              onDone();
            }
          }}
        />
        <button
          className="ag-panel__action is-confirm"
          title={t("done")}
          // without this the field loses focus before the click
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            commitName();
            onDone();
          }}
        >
          <Icon name="check" />
        </button>
      </div>

      <div className="ag-cables__field">
        {t("cable_in")} <span>{t("cable_in_description")}</span>
      </div>
      <DevicePicker
        flow="render"
        slot={CABLE_SLOT}
        value={cable.render}
        deviceHints={taken}
        onChange={(v) => update(cable.id, { render: v })}
      />

      <div className="ag-cables__field">
        {t("cable_out")} <span>{t("cable_out_description")}</span>
      </div>
      <DevicePicker
        flow="capture"
        slot={CABLE_SLOT}
        value={cable.capture}
        deviceHints={taken}
        onChange={(v) => update(cable.id, { capture: v })}
      />
    </div>
  );
}

/** Cables in the header: list, adding, setup and removal */
export function CableMenu() {
  const cables = useCables((s) => s.cables);
  const create = useCables((s) => s.create);
  const remove = useCables((s) => s.remove);
  const open = useCablePanel((s) => s.open);
  const editing = useCablePanel((s) => s.editing);
  const { show, hide, edit } = useCablePanel.getState();
  const [confirming, setConfirming] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();

  useEffect(() => {
    if (!open) {
      setConfirming(null);
      return;
    }
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element;
      // the device dropdown is rendered apart from the panel but belongs to it
      if (root.current?.contains(target) || target.closest?.(".ag-popover")) return;
      hide();
    };
    const onKey = (e: KeyboardEvent) => {
      // the first Escape closes an open device list, only the next one closes the panel
      if (e.key === "Escape" && !document.querySelector(".ag-popover")) hide();
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, hide]);

  return (
    <div className="ag-cables" ref={root}>
      <button className={`ag-header-btn${open ? " is-open" : ""}`} onClick={() => (open ? hide() : show())} title={t("cables")}>
        <span className="ag-cables__icon">
          <KindIcon kind="cable" />
        </span>
        {t("cables")}
        {cables.length > 0 && <span className="ag-count">{cables.length}</span>}
        <Icon name="chevron" size={12} />
      </button>

      {open && (
        <div className="ag-panel ag-cables__panel">
          {cables.length > 0 && <BulkPower cables={cables} />}
          <div className="ag-panel__list">
            {cables.length === 0 && (
              <div className="ag-cables__empty">
                {t(
                  "cables_empty",
                )}
              </div>
            )}
            {cables.map((c) =>
              editing === c.id ? (
                <CableEditor key={c.id} cable={c} onDone={() => edit(null)} />
              ) : confirming === c.id ? (
                <div key={c.id} className="ag-panel__row is-confirm">
                  <span className="ag-panel__confirm-text">{t("delete_confirm", { name: c.name })}</span>
                  <button
                    className="ag-panel__small is-danger"
                    onClick={() => {
                      remove(c.id);
                      setConfirming(null);
                    }}
                  >
                    {t("delete")}
                  </button>
                  <button className="ag-panel__small" onClick={() => setConfirming(null)}>
                    {t("cancel")}
                  </button>
                </div>
              ) : (
                <CableRow
                  key={c.id}
                  cable={c}
                  onEdit={() => {
                    setConfirming(null);
                    edit(c.id);
                  }}
                  onDelete={() => setConfirming(c.id)}
                />
              ),
            )}
          </div>
          <div className="ag-panel__footer">
            <button
              className="ag-panel__add"
              onClick={async () => {
                const id = await create();
                if (id) edit(id);
              }}
            >
              <Icon name="plus" />
              {t("new_cable")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
