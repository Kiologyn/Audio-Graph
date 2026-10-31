import { useRef, useState } from "react";
import { useT } from "../i18n";
import { findApp, toAppRef, useApps, type AudioApp } from "./apps";
import { PickerChip, Popover, PopoverItem } from "./Popover";
import type { AppRef } from "./types";

/** Выбор приложения для ноды приложения. При каждом открытии обновляет список запущенных. */
export function AppPicker({ value, onChange }: { value: AppRef | null; onChange: (app: AppRef | null) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const apps = useApps((s) => s.apps);
  const loaded = useApps((s) => s.loaded);
  const loading = useApps((s) => s.loading);
  const error = useApps((s) => s.error);
  const t = useT();

  const current = findApp(value, apps);
  // выбранное приложение сейчас закрыто: в списке запущенных его нет, но выбор надо показать
  const notRunning = value && loaded && !current ? value : null;
  const withAudio = apps.filter((a) => a.hasAudio);
  const others = apps.filter((a) => !a.hasAudio);

  const select = (next: AppRef | null) => {
    onChange(next);
    setOpen(false);
  };

  const item = (a: AudioApp) => (
    <PopoverItem
      key={a.path}
      selected={a === current}
      name={a.name}
      hint={a.playing ? t("{exe} · играет", { exe: a.exe }) : a.exe}
      onClick={() => select(toAppRef(a))}
    />
  );

  return (
    <>
      <PickerChip
        ref={anchor}
        text={value?.name ?? t("Приложение не выбрано")}
        title={value ? value.path || value.exe : undefined}
        empty={!value}
        open={open}
        onClick={() => {
          if (open) return setOpen(false);
          setOpen(true);
          useApps.getState().refresh();
        }}
      />
      {open && (
        <Popover
          anchor={anchor}
          busy={loading}
          onClose={() => setOpen(false)}
          footer={value && <PopoverItem name={t("Сбросить выбор")} className="is-dim" onClick={() => select(null)} />}
        >
          {notRunning && (
            <PopoverItem
              selected
              className="is-off"
              name={notRunning.name}
              hint={t("{exe} · не запущено", { exe: notRunning.exe })}
              onClick={() => select(notRunning)}
            />
          )}
          {withAudio.length > 0 && <div className="ag-popover__section">{t("Со звуком")}</div>}
          {withAudio.map(item)}
          {others.length > 0 && <div className="ag-popover__section">{t("Другие запущенные")}</div>}
          {others.map(item)}
          {apps.length === 0 && (
            <div className="ag-popover__empty">{loaded ? t("Нет запущенных приложений") : t("Загрузка…")}</div>
          )}
          {error && <div className="ag-popover__empty is-error">{error}</div>}
        </Popover>
      )}
    </>
  );
}
