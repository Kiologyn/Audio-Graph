import { getCurrentWindow } from "@tauri-apps/api/window";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@xyflow/react/dist/style.css";
import "./styles.css";
import App from "./App";
import { loadPlatform } from "./platform";
import { useSettings, watchSettings } from "./settings";
import { TrayMenu } from "./TrayMenu";

// the tray's menu on Windows is a window of its own with the same page
const trayMenu = getCurrentWindow().label === "tray-menu";

// the app doesn't need the browser menu (Back, Refresh, Print…); text fields keep copy and paste
window.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement;
  if (!target.closest("input:not([type]), input[type=text], textarea, [contenteditable]")) e.preventDefault();
});

// theme, language and platform are known before the first render: no flash of a wrong theme or language
Promise.all([useSettings.getState().load(), loadPlatform()])
  .finally(() => {
    watchSettings();
    createRoot(document.getElementById("root")!).render(
      <StrictMode>{trayMenu ? <TrayMenu /> : <App />}</StrictMode>,
    );
  });
