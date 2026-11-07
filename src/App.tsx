import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { ReactFlowProvider } from "@xyflow/react";
import { useEffect, useState } from "react";
import { CableMenu } from "./graph/CableMenu";
import { useCables } from "./graph/cables";
import { Calibration } from "./graph/Calibration";
import { Canvas } from "./graph/Canvas";
import { useDevices } from "./graph/devices";
import { useEngine } from "./graph/engine";
import { GraphMenu } from "./graph/GraphMenu";
import { quitApp, useLibrary } from "./graph/library";
import { RunControl } from "./graph/RunControl";
import { SettingsMenu } from "./graph/SettingsMenu";
import { dynamicCables } from "./platform";
import { useGraph } from "./graph/store";
import { useT } from "./i18n";
import { useSettings } from "./settings";

function SaveIndicator() {
  const ready = useLibrary((s) => s.ready);
  const status = useLibrary((s) => s.status);
  const error = useLibrary((s) => s.error);
  const cablesError = useCables((s) => s.error);
  const toggleError = useDevices((s) => s.toggleError);
  const settingsError = useSettings((s) => s.error);
  const t = useT();
  const fail = (text: string) => <span className="ag-save is-error">{text}</span>;
  if (toggleError) return fail(t("error_switch_device", { error: toggleError }));
  if (cablesError) return fail(t("error_save_cables", { error: cablesError }));
  if (settingsError) return fail(t("error_save_settings", { error: settingsError }));
  if (!ready && status !== "error") return null;
  if (status === "error") return fail(t("error_save", { error: error ?? "" }));
  if (status === "saved") return <span className="ag-save">{t("saved")}</span>;
  return <span className="ag-save">{t("saving")}</span>;
}

/** The app's version, the one in package.json */
function Version() {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    getVersion().then(setVersion, () => {});
  }, []);
  return version && <span className="ag-version">v{version}</span>;
}

export default function App() {
  const graphId = useGraph((s) => s.meta?.id);

  useEffect(() => {
    useLibrary.getState().init();
    useCables.getState().load();
    // the device list is needed right away: it shows which chosen devices are unavailable now
    useDevices.getState().refresh();
    useEngine.getState().init();
    // the close button without a tray, the tray's menu: the app waits for this before it goes
    const stop = listen("quit-requested", quitApp);
    return () => void stop.then((unlisten) => unlisten());
  }, []);

  return (
    <div className="ag-app">
      <header className="ag-toolbar">
        <SettingsMenu />
        <GraphMenu />
        {/* on Linux the graph creates its cables itself: there is no shared list */}
        {!dynamicCables() && <CableMenu />}
        <RunControl />
      </header>
      {graphId ? (
        // a provider per graph: the canvas, camera and selection start from scratch
        <ReactFlowProvider key={graphId}>
          <Canvas />
        </ReactFlowProvider>
      ) : (
        <div className="ag-canvas" />
      )}
      <footer className="ag-statusbar">
        <SaveIndicator />
        <Version />
      </footer>
      <Calibration />
    </div>
  );
}
