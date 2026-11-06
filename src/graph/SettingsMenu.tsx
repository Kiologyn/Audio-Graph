import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { LANGUAGES, useT } from "../i18n";
import { autostartSupported, dynamicCables, platform } from "../platform";
import { useSettings, type Lang, type Theme } from "../settings";
import { Icon } from "./icons";
import { PickerChip, Popover, PopoverItem } from "./Popover";

/** A switch between several options */
function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="ag-segmented" role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={o.value === value}
          className={o.value === value ? "is-active" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Language choice: language names in the languages themselves, the hint in English */
function LanguagePicker({ value, onChange }: { value: Lang; onChange: (language: Lang) => void }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const current = LANGUAGES.find((l) => l.code === value);
  return (
    <>
      <PickerChip ref={anchor} text={current?.name ?? value} open={open} onClick={() => setOpen(!open)} />
      {open && (
        <Popover anchor={anchor} onClose={() => setOpen(false)}>
          {LANGUAGES.map((l) => (
            <PopoverItem
              key={l.code}
              selected={l.code === value}
              name={l.name}
              hint={l.english !== l.name ? l.english : null}
              onClick={() => {
                onChange(l.code);
                setOpen(false);
              }}
            />
          ))}
        </Popover>
      )}
    </>
  );
}

/** Settings on the left of the header: theme, language, cable and canvas behavior */
export function SettingsMenu() {
  const theme = useSettings((s) => s.theme);
  const language = useSettings((s) => s.language);
  const disableIdleCables = useSettings((s) => s.disableIdleCables);
  const hideMinimap = useSettings((s) => s.hideMinimap);
  const autostart = useSettings((s) => s.autostart);
  const tray = useSettings((s) => s.tray);
  const set = useSettings((s) => s.set);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element;
      // the language list is rendered apart from the panel but belongs to it
      if (root.current?.contains(target) || target.closest?.(".ag-popover")) return;
      setOpen(false);
    };
    // the first Escape closes an open language list, only the next one closes the panel
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !document.querySelector(".ag-popover") && setOpen(false);
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="ag-settings" ref={root}>
      <button
        className={`ag-header-btn ag-header-btn--icon${open ? " is-open" : ""}`}
        onClick={() => setOpen(!open)}
        title={t("settings")}
      >
        <Icon name="settings" size={16} />
      </button>

      {open && (
        <div className="ag-panel ag-settings__panel">
          <div className="ag-settings__field">
            <span className="ag-settings__label">{t("settings_theme")}</span>
            <Segmented<Theme>
              value={theme}
              onChange={(theme) => set({ theme })}
              options={[
                { value: "light", label: t("settings_theme_light") },
                { value: "system", label: t("settings_theme_auto") },
                { value: "dark", label: t("settings_theme_dark") },
              ]}
            />
          </div>
          <div className="ag-settings__field">
            <span className="ag-settings__label">{t("settings_language")}</span>
            <LanguagePicker value={language} onChange={(language) => set({ language })} />
          </div>
          <label className="ag-settings__check">
            <input type="checkbox" checked={tray} onChange={(e) => set({ tray: e.currentTarget.checked })} />
            <span>
              {t("settings_tray")}
              <small>{t("settings_tray_hint")}</small>
            </span>
          </label>
          <label className={`ag-settings__check${autostartSupported() ? "" : " is-disabled"}`}>
            <input
              type="checkbox"
              checked={autostart}
              disabled={!autostartSupported()}
              onChange={(e) => set({ autostart: e.currentTarget.checked })}
            />
            <span>
              {t("settings_autostart")}
              <small>
                {t(!autostartSupported() ? "settings_autostart_portable" : tray ? "settings_autostart_hint" : "settings_autostart_hint_window")}
              </small>
            </span>
          </label>
          {/* Linux virtual cables exist only while the graph runs anyway */}
          {!dynamicCables() && <label className="ag-settings__check">
            <input
              type="checkbox"
              checked={disableIdleCables}
              onChange={(e) => set({ disableIdleCables: e.currentTarget.checked })}
            />
            <span>
              {t("settings_disable_idle_cables")}
              <small>
                {t(
                  "settings_disable_idle_cables_hint",
                )}
              </small>
            </span>
          </label>}
          <label className="ag-settings__check">
            <input type="checkbox" checked={hideMinimap} onChange={(e) => set({ hideMinimap: e.currentTarget.checked })} />
            <span>
              {t("settings_hide_minimap")}
              <small>{t("settings_hide_minimap_hint")}</small>
            </span>
          </label>
          {/* on Linux the tray icon isn't shown everywhere (GNOME without an extension), and quitting is in its menu;
              without the tray closing the window quits */}
          {platform() === "linux" && tray && (
            <button className="ag-settings__quit" onClick={() => invoke("app_quit")}>
              {t("quit_app")}
              <small>{t("quit_app_hint")}</small>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
