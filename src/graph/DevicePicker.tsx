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
  /** Не предлагать устройства, из которых собраны кабели (для нод ввода и вывода) */
  hideCableDevices?: boolean;
  /** Не предлагать отключённые в системе устройства (для нод ввода и вывода) */
  hideDisabled?: boolean;
  /** Дополнительные подсказки к устройствам: id → текст */
  deviceHints?: Map<string, string>;
};

/** Выбор системного устройства. При каждом открытии просит обновить общий буфер устройств. */
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

  const hidden = (d: AudioDevice) => (hideCableDevices && owners.has(d.id)) || (hideDisabled && d.state === "disabled");
  const visible = devices.filter((d) => d.flow === flow && !hidden(d));
  // сначала включённые, выключенные — в конце
  const list = [...visible.filter((d) => d.state === "active"), ...visible.filter((d) => d.state !== "active")];
  const defaultDevice = devices.find((d) => d.flow === flow && d.isDefault);
  const { text, missing, disabled } = describeDevice(value, flow, devices, loaded);
  // устройство, выбранное до того, как его взяли в кабель или отключили: из списка скрыто, но выбор надо показать
  const owner = value?.type === "device" ? owners.get(value.id) : undefined;
  const hiddenCurrent =
    value?.type === "device" && ((hideCableDevices && owner) || (hideDisabled && disabled)) ? value : null;

  let title = text ?? undefined;
  if (value?.type === "default") title = defaultDevice ? t("Сейчас: {name}", { name: defaultDevice.name }) : undefined;
  if (disabled) title = t("{name} — отключено", { name: text ?? "" });
  if (missing && value?.type === "device") title = t("{name} — сейчас недоступно", { name: value.name });

  const hint = (d: AudioDevice) =>
    [d.isDefault && t("по умолчанию"), d.state === "disabled" && t("отключено"), deviceHints?.get(d.id)]
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
          footer={value && <PopoverItem name={t("Сбросить выбор")} className="is-dim" onClick={() => select(null)} />}
        >
          {missing && value?.type === "device" && (
            <PopoverItem selected className="is-missing" name={value.name} hint={t("недоступно")} onClick={() => select(value)} />
          )}
          {hiddenCurrent && (
            <PopoverItem
              selected
              className={disabled ? "is-off" : ""}
              name={text ?? hiddenCurrent.name}
              hint={[disabled && t("отключено"), owner && t("устройство кабеля «{name}»", { name: owner })].filter(Boolean).join(" · ")}
              onClick={() => select(hiddenCurrent)}
            />
          )}
          {slot.allowDefault && (
            <PopoverItem
              selected={value?.type === "default"}
              className="is-default"
              name={t("По умолчанию")}
              hint={defaultDevice?.name ?? t("нет устройства")}
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
            <div className="ag-popover__empty">{loaded ? t("Нет подходящих устройств") : t("Загрузка…")}</div>
          )}
          {error && <div className="ag-popover__empty is-error">{error}</div>}
        </Popover>
      )}
    </>
  );
}
