import { ReactFlowProvider } from "@xyflow/react";
import { useEffect } from "react";
import { CableMenu } from "./graph/CableMenu";
import { useCables } from "./graph/cables";
import { Canvas } from "./graph/Canvas";
import { useDevices } from "./graph/devices";
import { useEngine } from "./graph/engine";
import { GraphMenu } from "./graph/GraphMenu";
import { useLibrary } from "./graph/library";
import { RunControl } from "./graph/RunControl";
import { SettingsMenu } from "./graph/SettingsMenu";
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
  if (toggleError) return fail(t("Не удалось переключить устройство: {error}", { error: toggleError }));
  if (cablesError) return fail(t("Не удалось сохранить кабели: {error}", { error: cablesError }));
  if (settingsError) return fail(t("Не удалось сохранить настройки: {error}", { error: settingsError }));
  if (!ready && status !== "error") return null;
  if (status === "error") return fail(t("Не удалось сохранить: {error}", { error: error ?? "" }));
  if (status === "saved") return <span className="ag-save">{t("Сохранено")}</span>;
  return <span className="ag-save">{t("Сохранение…")}</span>;
}

export default function App() {
  const graphId = useGraph((s) => s.meta?.id);

  useEffect(() => {
    useLibrary.getState().init();
    useCables.getState().load();
    // список устройств нужен сразу: по нему видно, какие выбранные устройства сейчас недоступны
    useDevices.getState().refresh();
    useEngine.getState().init();
  }, []);

  return (
    <div className="ag-app">
      <header className="ag-toolbar">
        <SettingsMenu />
        <GraphMenu />
        <CableMenu />
        <RunControl />
      </header>
      {graphId ? (
        // свой провайдер на каждый граф: холст, камера и выделение начинаются с чистого листа
        <ReactFlowProvider key={graphId}>
          <Canvas />
        </ReactFlowProvider>
      ) : (
        <div className="ag-canvas" />
      )}
      <footer className="ag-statusbar">
        <SaveIndicator />
      </footer>
    </div>
  );
}
