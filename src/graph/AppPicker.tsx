import { useRef, useState } from "react";
import { useT } from "../i18n";
import { findApp, toAppRef, useApps, type AudioApp } from "./apps";
import { PickerChip, Popover, PopoverItem } from "./Popover";
import type { AppRef } from "./types";

/** App choice for an app node. Refreshes the list of running apps every time it opens. */
export function AppPicker({ value, onChange }: { value: AppRef | null; onChange: (app: AppRef | null) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const apps = useApps((s) => s.apps);
  const loaded = useApps((s) => s.loaded);
  const loading = useApps((s) => s.loading);
  const error = useApps((s) => s.error);
  const t = useT();

  const current = findApp(value, apps);
  // the chosen app is closed now: it isn't among the running ones, but the choice must be shown
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
      hint={a.playing ? t("app_playing", { exe: a.exe }) : a.exe}
      onClick={() => select(toAppRef(a))}
    />
  );

  return (
    <>
      <PickerChip
        ref={anchor}
        text={value?.name ?? t("app_none_selected")}
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
          footer={value && <PopoverItem name={t("clear_selection")} className="is-dim" onClick={() => select(null)} />}
        >
          {notRunning && (
            <PopoverItem
              selected
              className="is-off"
              name={notRunning.name}
              hint={t("app_not_running_hint", { exe: notRunning.exe })}
              onClick={() => select(notRunning)}
            />
          )}
          {withAudio.length > 0 && <div className="ag-popover__section">{t("apps_with_audio")}</div>}
          {withAudio.map(item)}
          {others.length > 0 && <div className="ag-popover__section">{t("apps_other_running")}</div>}
          {others.map(item)}
          {apps.length === 0 && (
            <div className="ag-popover__empty">{loaded ? t("apps_none_running") : t("loading")}</div>
          )}
          {error && <div className="ag-popover__empty is-error">{error}</div>}
        </Popover>
      )}
    </>
  );
}
