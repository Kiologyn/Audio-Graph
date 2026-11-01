import { useRef, useState } from "react";
import { useT } from "../i18n";
import { cablePower, useCablePanel, useCables, type Cable } from "./cables";
import { describeDevice, useDevices } from "./devices";
import { PickerChip, Popover, PopoverItem } from "./Popover";

/** Cable choice from the shared list for a cable node. null is an internal cable without system devices. */
export function CablePicker({ value, onChange }: { value: string | null; onChange: (id: string | null) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const cables = useCables((s) => s.cables);
  const loaded = useCables((s) => s.loaded);
  const devices = useDevices((s) => s.devices);
  const devicesLoaded = useDevices((s) => s.loaded);
  const loading = useDevices((s) => s.loading);
  const t = useT();

  // "input → output" with marks: a device is unavailable, the cable is off in the system
  const route = (c: Cable) => {
    const r = describeDevice(c.render, "render", devices, devicesLoaded);
    const cap = describeDevice(c.capture, "capture", devices, devicesLoaded);
    const power = cablePower(c, devices);
    const state = power === "off" ? t("cable_off_suffix") : power === "mixed" ? t("cable_partly_off_suffix") : "";
    const side = (d: typeof r) => `${d.text ?? t("not_selected")}${d.missing ? t("cable_unavailable_suffix") : d.disabled ? t("cable_disabled_suffix") : ""}`;
    return {
      text: `${r.text ?? t("not_selected")} → ${cap.text ?? t("not_selected")}${state}`,
      title: `${t("cable_in_device", { device: side(r) })}\n${t("cable_out_device", { device: side(cap) })}`,
      missing: r.missing || cap.missing,
      off: power === "off" || power === "mixed",
    };
  };

  const cable = value ? cables.find((c) => c.id === value) : undefined;
  const deleted = !!value && loaded && !cable;
  const info = cable ? route(cable) : null;

  let text = t("cable_internal");
  let title = t("cable_internal_hint");
  if (deleted) {
    text = t("cable_deleted");
    title = t("cable_deleted_hint");
  } else if (value) {
    text = cable?.name ?? "…";
    title = info?.title ?? "";
  }

  const select = (id: string | null) => {
    onChange(id);
    setOpen(false);
  };

  return (
    <>
      <PickerChip
        ref={anchor}
        text={text}
        title={title}
        empty={!value}
        missing={deleted || !!info?.missing}
        off={!!info?.off}
        open={open}
        onClick={() => {
          if (open) return setOpen(false);
          setOpen(true);
          useDevices.getState().refresh();
        }}
      />
      {open && (
        <Popover
          anchor={anchor}
          busy={loading}
          onClose={() => setOpen(false)}
          footer={
            <>
              <PopoverItem
                name={t("new_cable_menu")}
                className="is-dim"
                onClick={async () => {
                  setOpen(false);
                  const id = await useCables.getState().create();
                  if (!id) return;
                  onChange(id);
                  useCablePanel.getState().show(id);
                }}
              />
              <PopoverItem
                name={t("setup_cables_menu")}
                className="is-dim"
                onClick={() => {
                  setOpen(false);
                  useCablePanel.getState().show(value && !deleted ? value : null);
                }}
              />
            </>
          }
        >
          {deleted && <PopoverItem selected className="is-missing" name={t("cable_deleted")} hint={t("cable_deleted_detail")} onClick={() => setOpen(false)} />}
          <PopoverItem selected={!value} name={t("cable_internal")} hint={t("cable_internal_detail")} onClick={() => select(null)} />
          {cables.map((c) => {
            const r = route(c);
            return (
              <PopoverItem
                key={c.id}
                selected={value === c.id}
                name={c.name}
                hint={r.text}
                className={r.missing ? "has-missing" : r.off ? "is-off" : ""}
                onClick={() => select(c.id)}
              />
            );
          })}
          {loaded && cables.length === 0 && <div className="ag-popover__empty">{t("cables_none_yet")}</div>}
        </Popover>
      )}
    </>
  );
}
