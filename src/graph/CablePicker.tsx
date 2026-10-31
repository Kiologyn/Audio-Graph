import { useRef, useState } from "react";
import { useT } from "../i18n";
import { cablePower, useCablePanel, useCables, type Cable } from "./cables";
import { describeDevice, useDevices } from "./devices";
import { PickerChip, Popover, PopoverItem } from "./Popover";

/** Выбор кабеля из общего списка для ноды кабеля. null — внутренний кабель без системных устройств. */
export function CablePicker({ value, onChange }: { value: string | null; onChange: (id: string | null) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const cables = useCables((s) => s.cables);
  const loaded = useCables((s) => s.loaded);
  const devices = useDevices((s) => s.devices);
  const devicesLoaded = useDevices((s) => s.loaded);
  const loading = useDevices((s) => s.loading);
  const t = useT();

  // «вход → выход» с отметками: какое-то устройство недоступно, кабель выключен в системе
  const route = (c: Cable) => {
    const r = describeDevice(c.render, "render", devices, devicesLoaded);
    const cap = describeDevice(c.capture, "capture", devices, devicesLoaded);
    const power = cablePower(c, devices);
    const state = power === "off" ? t(" · выключен") : power === "mixed" ? t(" · выключен частично") : "";
    const side = (d: typeof r) => `${d.text ?? t("не выбрано")}${d.missing ? t(" (недоступно)") : d.disabled ? t(" (отключено)") : ""}`;
    return {
      text: `${r.text ?? t("не выбрано")} → ${cap.text ?? t("не выбрано")}${state}`,
      title: `${t("Вход: {device}", { device: side(r) })}\n${t("Выход: {device}", { device: side(cap) })}`,
      missing: r.missing || cap.missing,
      off: power === "off" || power === "mixed",
    };
  };

  const cable = value ? cables.find((c) => c.id === value) : undefined;
  const deleted = !!value && loaded && !cable;
  const info = cable ? route(cable) : null;

  let text = t("Внутренний");
  let title = t("Внутренний: без системных устройств");
  if (deleted) {
    text = t("Кабель удалён");
    title = t("Кабель удалён из списка кабелей, выберите другой");
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
                name={t("Новый кабель…")}
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
                name={t("Настроить кабели…")}
                className="is-dim"
                onClick={() => {
                  setOpen(false);
                  useCablePanel.getState().show(value && !deleted ? value : null);
                }}
              />
            </>
          }
        >
          {deleted && <PopoverItem selected className="is-missing" name={t("Кабель удалён")} hint={t("выберите другой")} onClick={() => setOpen(false)} />}
          <PopoverItem selected={!value} name={t("Внутренний")} hint={t("без системных устройств")} onClick={() => select(null)} />
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
          {loaded && cables.length === 0 && <div className="ag-popover__empty">{t("Кабелей пока нет")}</div>}
        </Popover>
      )}
    </>
  );
}
