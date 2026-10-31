import { useEffect, useRef, useState } from "react";
import { LANGUAGES, useT } from "../i18n";
import { useSettings, type Lang, type Theme } from "../settings";
import { Icon } from "./icons";
import { PickerChip, Popover, PopoverItem } from "./Popover";

/** Переключатель из нескольких вариантов */
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

/** Выбор языка: названия языков — на самих языках, подсказка — по-английски */
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

/** Настройки в шапке слева: тема, язык, поведение кабелей и холста */
export function SettingsMenu() {
  const theme = useSettings((s) => s.theme);
  const language = useSettings((s) => s.language);
  const disableIdleCables = useSettings((s) => s.disableIdleCables);
  const hideMinimap = useSettings((s) => s.hideMinimap);
  const newGraphVolume = useSettings((s) => s.newGraphVolume);
  const set = useSettings((s) => s.set);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element;
      // список языков рисуется отдельно от панели, но относится к ней
      if (root.current?.contains(target) || target.closest?.(".ag-popover")) return;
      setOpen(false);
    };
    // первый Escape закрывает открытый список языков, панель — только следующий
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
        title={t("Настройки")}
      >
        <Icon name="settings" size={16} />
      </button>

      {open && (
        <div className="ag-panel ag-settings__panel">
          <div className="ag-settings__field">
            <span className="ag-settings__label">{t("Тема")}</span>
            <Segmented<Theme>
              value={theme}
              onChange={(theme) => set({ theme })}
              options={[
                { value: "light", label: t("Светлая") },
                { value: "system", label: t("Системная") },
                { value: "dark", label: t("Тёмная") },
              ]}
            />
          </div>
          <div className="ag-settings__field">
            <span className="ag-settings__label">{t("Язык")}</span>
            <LanguagePicker value={language} onChange={(language) => set({ language })} />
          </div>
          <label className="ag-settings__check">
            <input
              type="checkbox"
              checked={disableIdleCables}
              onChange={(e) => set({ disableIdleCables: e.currentTarget.checked })}
            />
            <span>
              {t("Выключать кабели, когда граф остановлен")}
              <small>
                {t(
                  "Кабели запущенного графа включаются при его запуске и выключаются после остановки и при выходе из программы",
                )}
              </small>
            </span>
          </label>
          <label className="ag-settings__check">
            <input type="checkbox" checked={hideMinimap} onChange={(e) => set({ hideMinimap: e.currentTarget.checked })} />
            <span>
              {t("Убрать карту графа")}
              <small>{t("Уменьшенный вид всего графа в правом нижнем углу")}</small>
            </span>
          </label>
          <label className="ag-settings__check">
            <input
              type="checkbox"
              checked={newGraphVolume}
              onChange={(e) => set({ newGraphVolume: e.currentTarget.checked })}
            />
            <span>
              {t("Регуляторы громкости в новых графах")}
              <small>{t("С какой галочкой громкости создаётся граф; потом её можно сменить в списке графов")}</small>
            </span>
          </label>
        </div>
      )}
    </div>
  );
}
