import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { RunDot } from "./graph/GraphMenu";
import { useT } from "./i18n";
import { useSettings } from "./settings";

/** Matches TrayMenu in src-tauri/src/tray.rs */
type Shown = {
  /** troubled: running, and something in it doesn't work */
  graphs: { id: string; name: string; running: boolean; troubled: boolean }[];
  /** Any graph at all, so "Stop all" has something to stop */
  anyRunning: boolean;
};

/**
 * The tray's menu on Windows, in a small window of its own: the first graphs of the list with the list's dot by
 * those that run. A click starts or stops one and the menu stays for more; it goes when anything else is clicked
 */
export function TrayMenu() {
  const t = useT();
  const [shown, setShown] = useState<Shown>({ graphs: [], anyRunning: false });
  const menu = useRef<HTMLDivElement>(null);
  // the tray asked for the menu: it shows itself once what it shows is drawn
  const opening = useRef(false);

  useEffect(() => {
    document.body.classList.add("ag-tray-window");
    const load = () =>
      invoke<Shown>("tray_menu")
        .then(setShown)
        .catch(() => {});
    const stops = [
      listen("tray-menu-open", async () => {
        (document.activeElement as HTMLElement | null)?.blur();
        // the language or the theme may have changed in the app's window since
        await useSettings.getState().load();
        const next = await invoke<Shown>("tray_menu").catch(() => null);
        if (!next) return;
        opening.current = true;
        setShown(next);
      }),
      listen("tray-menu-changed", load),
      listen("engine-status", load),
    ];
    load();

    const keys = (e: KeyboardEvent) => {
      if (e.key === "Escape") invoke("tray_menu_close", { open: false });
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.key === "ArrowDown" ? at + 1 : at < 0 ? items.length - 1 : at - 1;
      items[(next + items.length) % items.length]?.focus();
    };
    window.addEventListener("keydown", keys);
    return () => {
      stops.forEach((stop) => stop.then((unlisten) => unlisten()));
      window.removeEventListener("keydown", keys);
    };
  }, []);

  // the window takes the menu's size and goes by the tray
  useLayoutEffect(() => {
    if (!opening.current || !menu.current) return;
    opening.current = false;
    const { width, height } = menu.current.getBoundingClientRect();
    invoke("tray_menu_show", { width, height });
  });

  const toggle = (id: string) => {
    // the dot follows the click at once; the engine's answer sets it right
    setShown((s) => ({ ...s, graphs: s.graphs.map((g) => (g.id === id ? { ...g, running: !g.running } : g)) }));
    invoke("tray_toggle", { id }).catch(() => {});
  };

  return (
    <div ref={menu} className="ag-tray" role="menu">
      {shown.graphs.map((g) => (
        <button key={g.id} className="ag-tray__item" role="menuitemcheckbox" aria-checked={g.running} onClick={() => toggle(g.id)}>
          <span className="ag-tray__state">
            <RunDot running={g.running} troubled={g.troubled} />
          </span>
          <span className="ag-tray__name">{g.name}</span>
        </button>
      ))}
      {shown.graphs.length > 0 && <div className="ag-tray__line" role="separator" />}
      <button className="ag-tray__item" role="menuitem" disabled={!shown.anyRunning} onClick={() => invoke("engine_stop", { id: null })}>
        <span className="ag-tray__state" />
        <span className="ag-tray__name">{t("tray_stop_all")}</span>
      </button>
      <div className="ag-tray__line" role="separator" />
      <button className="ag-tray__item" role="menuitem" onClick={() => invoke("tray_menu_close", { open: true })}>
        <span className="ag-tray__state" />
        <span className="ag-tray__name">{t("tray_open")}</span>
      </button>
      <button className="ag-tray__item" role="menuitem" onClick={() => invoke("app_quit")}>
        <span className="ag-tray__state" />
        <span className="ag-tray__name">{t("tray_quit")}</span>
      </button>
    </div>
  );
}
