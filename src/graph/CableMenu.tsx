import { useEffect, useMemo, useRef, useState } from "react";
import {
  cableDeviceOwners,
  cablePower,
  cableDeviceIds,
  idsToToggle,
  useCablePanel,
  useCables,
  type Cable,
} from "./cables";
import { useT } from "../i18n";
import { describeDevice, useDevices } from "./devices";
import { DevicePicker } from "./DevicePicker";
import { Icon, KindIcon } from "./icons";
import type { DeviceSlot } from "./types";

const CABLE_SLOT: DeviceSlot = { placeholder: "Не выбрано", allowDefault: false };

// ключи перевода
const POWER_TITLE = {
  on: "Кабель включён — выключить",
  off: "Кабель выключен — включить",
  mixed: "Кабель включён частично — включить полностью",
  none: "Нечего включать: устройства кабеля не выбраны или их нет в системе",
};

/** Переключатель кабеля: включает и выключает оба его устройства в системе */
function PowerSwitch({ cable }: { cable: Cable }) {
  const devices = useDevices((s) => s.devices);
  const pending = useDevices((s) => s.pending);
  const setEnabled = useDevices((s) => s.setEnabled);
  const t = useT();
  const power = cablePower(cable, devices);
  const busy = cableDeviceIds(cable).some((id) => pending.includes(id));
  // выключенный и частично выключенный кабель по нажатию включается целиком
  const enable = power !== "on";

  return (
    <button
      className={`ag-switch is-${power}${busy ? " is-busy" : ""}`}
      role="switch"
      aria-checked={power === "on"}
      disabled={power === "none" || busy}
      title={t(POWER_TITLE[power])}
      onClick={() => setEnabled(idsToToggle([cable], devices, enable), enable)}
    />
  );
}

/** Включить или выключить все кабели разом */
function BulkPower({ cables }: { cables: Cable[] }) {
  const devices = useDevices((s) => s.devices);
  const pending = useDevices((s) => s.pending);
  const setEnabled = useDevices((s) => s.setEnabled);
  const toEnable = idsToToggle(cables, devices, true);
  const toDisable = idsToToggle(cables, devices, false);
  const busy = pending.length > 0;
  const t = useT();

  return (
    <div className="ag-cables__bulk">
      <span>{t("Все кабели")}</span>
      <button
        className="ag-panel__small"
        disabled={busy || toEnable.length === 0}
        onClick={() => setEnabled(toEnable, true)}
      >
        {t("Включить")}
      </button>
      <button
        className="ag-panel__small"
        disabled={busy || toDisable.length === 0}
        onClick={() => setEnabled(toDisable, false)}
      >
        {t("Выключить")}
      </button>
    </div>
  );
}

function CableRow({ cable, onEdit, onDelete }: { cable: Cable; onEdit: () => void; onDelete: () => void }) {
  const devices = useDevices((s) => s.devices);
  const loaded = useDevices((s) => s.loaded);
  const t = useT();
  const sides = [
    { caption: "Вход", d: describeDevice(cable.render, "render", devices, loaded) },
    { caption: "Выход", d: describeDevice(cable.capture, "capture", devices, loaded) },
  ];

  return (
    <div className="ag-panel__row">
      <button className="ag-cables__info" onClick={onEdit} title={t("Настроить кабель")}>
        <span className="ag-cables__name">{cable.name}</span>
        {sides.map(({ caption, d }) => (
          <span
            key={caption}
            className={`ag-cables__side${d.missing ? " is-missing" : ""}${d.text ? "" : " is-empty"}`}
          >
            {t(caption)}: {d.text ?? t("не выбрано")}
            {d.missing && t(" — недоступно")}
            {d.disabled && t(" — отключено")}
          </span>
        ))}
      </button>
      <PowerSwitch cable={cable} />
      <button className="ag-panel__action" title={t("Настроить")} onClick={onEdit}>
        <Icon name="pencil" />
      </button>
      <button className="ag-panel__action is-danger" title={t("Удалить")} onClick={onDelete}>
        <Icon name="trash" />
      </button>
    </div>
  );
}

/** Раскрытый кабель: имя и два устройства. Всё сохраняется сразу, ✓ только сворачивает. */
function CableEditor({ cable, onDone }: { cable: Cable; onDone: () => void }) {
  const update = useCables((s) => s.update);
  const cables = useCables((s) => s.cables);
  const t = useT();
  // устройства, уже взятые другими кабелями, подписываем, чтобы не собрать два кабеля из одного устройства
  const taken = useMemo(() => {
    const owners = cableDeviceOwners(cables.filter((c) => c.id !== cable.id));
    return new Map([...owners].map(([id, name]) => [id, t("в кабеле «{name}»", { name })]));
  }, [cables, cable.id, t]);
  const draft = useRef(cable.name);
  const cancelled = useRef(false);

  const commitName = () => {
    if (cancelled.current) return;
    const name = draft.current.trim();
    const current = useCables.getState().cables.find((c) => c.id === cable.id);
    if (current && name && name !== current.name) useCables.getState().update(cable.id, { name });
  };

  // панель могут закрыть кликом мимо, поле при этом просто исчезнет — имя сохраняем при размонтировании
  useEffect(() => () => commitName(), []);

  return (
    <div className="ag-cables__editor">
      <div className="ag-cables__editor-head">
        <input
          className="ag-panel__input"
          autoFocus
          defaultValue={cable.name}
          maxLength={100}
          onFocus={(e) => e.currentTarget.select()}
          onChange={(e) => (draft.current = e.currentTarget.value)}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              commitName();
              onDone();
            }
            if (e.key === "Escape") {
              // Escape отменяет правку имени и сворачивает кабель, а не закрывает панель
              e.stopPropagation();
              cancelled.current = true;
              onDone();
            }
          }}
        />
        <button
          className="ag-panel__action is-confirm"
          title={t("Готово")}
          // без этого поле потеряет фокус раньше клика
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            commitName();
            onDone();
          }}
        >
          <Icon name="check" />
        </button>
      </div>

      <div className="ag-cables__field">
        {t("Вход")} <span>{t("— устройство вывода, в него играют приложения")}</span>
      </div>
      <DevicePicker
        flow="render"
        slot={CABLE_SLOT}
        value={cable.render}
        deviceHints={taken}
        onChange={(v) => update(cable.id, { render: v })}
      />

      <div className="ag-cables__field">
        {t("Выход")} <span>{t("— устройство ввода, с него забирается звук")}</span>
      </div>
      <DevicePicker
        flow="capture"
        slot={CABLE_SLOT}
        value={cable.capture}
        deviceHints={taken}
        onChange={(v) => update(cable.id, { capture: v })}
      />
    </div>
  );
}

/** Кабели в шапке: список, добавление, настройка и удаление */
export function CableMenu() {
  const cables = useCables((s) => s.cables);
  const create = useCables((s) => s.create);
  const remove = useCables((s) => s.remove);
  const open = useCablePanel((s) => s.open);
  const editing = useCablePanel((s) => s.editing);
  const { show, hide, edit } = useCablePanel.getState();
  const [confirming, setConfirming] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();

  useEffect(() => {
    if (!open) {
      setConfirming(null);
      return;
    }
    const onDown = (e: PointerEvent) => {
      const target = e.target as Element;
      // выпадающий список устройств рисуется отдельно от панели, но относится к ней
      if (root.current?.contains(target) || target.closest?.(".ag-popover")) return;
      hide();
    };
    const onKey = (e: KeyboardEvent) => {
      // первый Escape закрывает открытый список устройств, панель — только следующий
      if (e.key === "Escape" && !document.querySelector(".ag-popover")) hide();
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, hide]);

  return (
    <div className="ag-cables" ref={root}>
      <button className={`ag-header-btn${open ? " is-open" : ""}`} onClick={() => (open ? hide() : show())} title={t("Кабели")}>
        <span className="ag-cables__icon">
          <KindIcon kind="cable" />
        </span>
        {t("Кабели")}
        {cables.length > 0 && <span className="ag-count">{cables.length}</span>}
        <Icon name="chevron" size={12} />
      </button>

      {open && (
        <div className="ag-panel ag-cables__panel">
          {cables.length > 0 && <BulkPower cables={cables} />}
          <div className="ag-panel__list">
            {cables.length === 0 && (
              <div className="ag-cables__empty">
                {t(
                  "Кабелей пока нет. Кабель — это пара устройств: устройство вывода, в которое играют приложения, и устройство ввода, с которого забирается звук.",
                )}
              </div>
            )}
            {cables.map((c) =>
              editing === c.id ? (
                <CableEditor key={c.id} cable={c} onDone={() => edit(null)} />
              ) : confirming === c.id ? (
                <div key={c.id} className="ag-panel__row is-confirm">
                  <span className="ag-panel__confirm-text">{t("Удалить «{name}»?", { name: c.name })}</span>
                  <button
                    className="ag-panel__small is-danger"
                    onClick={() => {
                      remove(c.id);
                      setConfirming(null);
                    }}
                  >
                    {t("Удалить")}
                  </button>
                  <button className="ag-panel__small" onClick={() => setConfirming(null)}>
                    {t("Отмена")}
                  </button>
                </div>
              ) : (
                <CableRow
                  key={c.id}
                  cable={c}
                  onEdit={() => {
                    setConfirming(null);
                    edit(c.id);
                  }}
                  onDelete={() => setConfirming(c.id)}
                />
              ),
            )}
          </div>
          <div className="ag-panel__footer">
            <button
              className="ag-panel__add"
              onClick={async () => {
                const id = await create();
                if (id) edit(id);
              }}
            >
              <Icon name="plus" />
              {t("Новый кабель")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
