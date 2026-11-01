import { useMemo, useRef, useState } from "react";
import { useT } from "../i18n";
import { cableDeviceOwners, useCables } from "./cables";
import { describeDevice, useDevices, type AudioDevice } from "./devices";
import { PickerChip, Popover, PopoverItem } from "./Popover";
import type { DeviceRef, DeviceSlot } from "./types";

type Props = {
  flow: AudioDevice["flow"];
  slot: DeviceSlot;
  value: DeviceRef | null;
  onChange: (value: DeviceRef | null) => void;
  /** Don't offer devices that cables are built from (for input and output nodes) */
  hideCableDevices?: boolean;
  /** Don't offer devices disabled in the system (for input and output nodes) */
  hideDisabled?: boolean;
  /** Extra hints for devices: id → text */
  deviceHints?: Map<string, string>;
};

/** System device choice. Asks to refresh the shared device buffer every time it opens. */
export function DevicePicker({ flow, slot, value, onChange, hideCableDevices, hideDisabled, deviceHints }: Props) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const devices = useDevices((s) => s.devices);
  const loaded = useDevices((s) => s.loaded);
  const loading = useDevices((s) => s.loading);
  const error = useDevices((s) => s.error);
  const cables = useCables((s) => s.cables);
  const owners = useMemo(() => cableDeviceOwners(cables), [cables]);
  const t = useT();

  // devices of the graph's virtual cables are chosen by the cable node, not here
  const hidden = (d: AudioDevice) =>
    d.isVirtual || (hideCableDevices && owners.has(d.id)) || (hideDisabled && d.state === "disabled");
  const visible = devices.filter((d) => d.flow === flow && !hidden(d));
  // enabled first, disabled at the end
  const list = [...visible.filter((d) => d.state === "active"), ...visible.filter((d) => d.state !== "active")];
  const defaultDevice = devices.find((d) => d.flow === flow && d.isDefault);
  const { text, missing, disabled } = describeDevice(value, flow, devices, loaded);
  // a device chosen before it was taken into a cable or disabled: hidden from the list, but the choice must be shown
  const owner = value?.type === "device" ? owners.get(value.id) : undefined;
  const hiddenCurrent =
    value?.type === "device" && ((hideCableDevices && owner) || (hideDisabled && disabled)) ? value : null;

  let title = text ?? undefined;
  if (value?.type === "default") title = defaultDevice ? t("device_default_now", { name: defaultDevice.name }) : undefined;
  if (disabled) title = t("device_disabled_named", { name: text ?? "" });
  if (missing && value?.type === "device") title = t("device_unavailable_named", { name: value.name });

  const hint = (d: AudioDevice) =>
    [d.isDefault && t("device_default_lower"), d.state === "disabled" && t("device_disabled"), deviceHints?.get(d.id)]
      .filter(Boolean)
      .join(" · ") || null;

  const select = (next: DeviceRef | null) => {
    onChange(next);
    setOpen(false);
  };

  return (
    <>
      <PickerChip
        ref={anchor}
        text={text ?? t(slot.placeholder)}
        title={title}
        empty={!value}
        missing={missing}
        off={disabled}
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
          footer={value && <PopoverItem name={t("clear_selection")} className="is-dim" onClick={() => select(null)} />}
        >
          {missing && value?.type === "device" && (
            <PopoverItem selected className="is-missing" name={value.name} hint={t("device_unavailable")} onClick={() => select(value)} />
          )}
          {hiddenCurrent && (
            <PopoverItem
              selected
              className={disabled ? "is-off" : ""}
              name={text ?? hiddenCurrent.name}
              hint={[disabled && t("device_disabled"), owner && t("device_of_cable", { name: owner })].filter(Boolean).join(" · ")}
              onClick={() => select(hiddenCurrent)}
            />
          )}
          {slot.allowDefault && (
            <PopoverItem
              selected={value?.type === "default"}
              className="is-default"
              name={t("device_default")}
              hint={defaultDevice?.name ?? t("device_none")}
              onClick={() => select({ type: "default" })}
            />
          )}
          {list.map((d) => (
            <PopoverItem
              key={d.id}
              selected={value?.type === "device" && value.id === d.id}
              className={d.state === "disabled" ? "is-off" : ""}
              name={d.name}
              hint={hint(d)}
              onClick={() => select({ type: "device", id: d.id, name: d.name })}
            />
          ))}
          {list.length === 0 && (
            <div className="ag-popover__empty">{loaded ? t("devices_none_suitable") : t("loading")}</div>
          )}
          {error && <div className="ag-popover__empty is-error">{error}</div>}
        </Popover>
      )}
    </>
  );
}
