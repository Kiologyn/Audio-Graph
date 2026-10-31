import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@xyflow/react/dist/style.css";
import "./styles.css";
import App from "./App";
import { useSettings, watchSettings } from "./settings";

// меню браузера (Назад, Обновить, Печать…) приложению не нужно; в полях ввода оставлено копирование и вставка
window.addEventListener("contextmenu", (e) => {
  const target = e.target as HTMLElement;
  if (!target.closest("input:not([type]), input[type=text], textarea, [contenteditable]")) e.preventDefault();
});

// тема и язык известны до первой отрисовки: без мелькания чужой темы и языка
useSettings
  .getState()
  .load()
  .finally(() => {
    watchSettings();
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  });
