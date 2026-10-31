import { useEffect, useRef, useState } from "react";
import { useT } from "../i18n";
import type { GraphMeta } from "./doc";
import { useEngine } from "./engine";
import { Icon } from "./icons";
import { useLibrary } from "./library";
import { useGraph } from "./store";

type RowMode = "view" | "edit" | "confirm";

function GraphRow({
  graph,
  current,
  mode,
  setMode,
  onOpen,
}: {
  graph: GraphMeta;
  current: boolean;
  mode: RowMode;
  setMode: (mode: RowMode) => void;
  onOpen: () => void;
}) {
  const rename = useLibrary((s) => s.rename);
  const remove = useLibrary((s) => s.remove);
  const input = useRef<HTMLInputElement>(null);
  const t = useT();

  if (mode === "edit") {
    const commit = (value: string) => {
      if (value.trim() && value.trim() !== graph.name) rename(graph.id, value);
      setMode("view");
    };
    return (
      <div className="ag-panel__row is-editing">
        <input
          ref={input}
          className="ag-panel__input"
          autoFocus
          defaultValue={graph.name}
          maxLength={100}
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => commit(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(e.currentTarget.value);
            if (e.key === "Escape") {
              // Escape отменяет переименование, а не закрывает меню
              e.stopPropagation();
              setMode("view");
            }
          }}
        />
        <button
          className="ag-panel__action is-confirm"
          title={t("Сохранить")}
          // без этого поле потеряет фокус раньше клика и строка закроется по blur
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => commit(input.current?.value ?? "")}
        >
          <Icon name="check" />
        </button>
      </div>
    );
  }

  if (mode === "confirm") {
    return (
      <div className="ag-panel__row is-confirm">
        <span className="ag-panel__confirm-text">{t("Удалить «{name}»?", { name: graph.name })}</span>
        <button className="ag-panel__small is-danger" onClick={() => remove(graph.id)}>
          {t("Удалить")}
        </button>
        <button className="ag-panel__small" onClick={() => setMode("view")}>
          {t("Отмена")}
        </button>
      </div>
    );
  }

  return (
    <div className={`ag-panel__row${current ? " is-current" : ""}`}>
      <RunState id={graph.id} />
      <button className="ag-panel__name" onClick={onOpen} onDoubleClick={() => setMode("edit")} title={graph.name}>
        {graph.name}
      </button>
      <VolumeToggle graph={graph} />
      <RunToggle id={graph.id} />
      <button className="ag-panel__action" title={t("Переименовать")} onClick={() => setMode("edit")}>
        <Icon name="pencil" />
      </button>
      <button className="ag-panel__action is-danger" title={t("Удалить")} onClick={() => setMode("confirm")}>
        <Icon name="trash" />
      </button>
    </div>
  );
}

/** Точка у графа в списке: зелёная — работает, красная — работает с проблемами */
function RunState({ id }: { id: string }) {
  const running = useEngine((s) => s.status.running.includes(id));
  const troubled = useEngine((s) => s.status.troubled.includes(id));
  const t = useT();
  if (!running) return <span className="ag-graphs__state" />;
  return (
    <span
      className={`ag-graphs__state is-running${troubled ? " is-troubled" : ""}`}
      title={troubled ? t("Работает, но не всё: есть проблемы") : t("Работает")}
    />
  );
}

/** Галочка графа: показывать ли на его нодах регуляторы громкости */
function VolumeToggle({ graph }: { graph: GraphMeta }) {
  const setShowVolume = useLibrary((s) => s.setShowVolume);
  const t = useT();
  return (
    <label className="ag-graphs__volume" title={t("Громкость нод: показывать регуляторы")}>
      <input
        type="checkbox"
        checked={graph.showVolume ?? true}
        onChange={(e) => setShowVolume(graph.id, e.currentTarget.checked)}
      />
      <Icon name="volume" />
    </label>
  );
}

/** Запуск и остановка графа прямо из списка */
function RunToggle({ id }: { id: string }) {
  const running = useEngine((s) => s.status.running.includes(id));
  const t = useT();
  return (
    <button
      className={`ag-panel__action${running ? " is-danger" : " is-run"}`}
      title={running ? t("Остановить") : t("Запустить")}
      onClick={() => (running ? useEngine.getState().stop(id) : useEngine.getState().start(id))}
    >
      <Icon name={running ? "stop" : "play"} />
    </button>
  );
}

/** Выбор открытого графа и управление списком графов */
export function GraphMenu() {
  const graphs = useLibrary((s) => s.graphs);
  const open = useLibrary((s) => s.open);
  const create = useLibrary((s) => s.create);
  const current = useGraph((s) => s.meta);
  const runningCount = useEngine((s) => s.status.running.length);
  const [isOpen, setOpen] = useState(false);
  // в каждый момент не больше одной строки переименовывается или ждёт подтверждения
  const [active, setActive] = useState<{ id: string; mode: RowMode } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const t = useT();

  const close = () => {
    setOpen(false);
    setActive(null);
  };

  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  const createAndRename = async () => {
    const id = await create();
    if (id) setActive({ id, mode: "edit" });
  };

  return (
    <div className="ag-graphs" ref={root}>
      <button
        className={`ag-header-btn${isOpen ? " is-open" : ""}`}
        onClick={() => (isOpen ? close() : setOpen(true))}
        title={t("Графы")}
      >
        <Icon name="graph" />
        <span className="ag-header-btn__name">{current?.name ?? "…"}</span>
        {runningCount > 0 && (
          <span className="ag-count is-running" title={t("Запущено графов: {count}", { count: runningCount })}>
            {runningCount}
          </span>
        )}
        <Icon name="chevron" size={12} />
      </button>

      {isOpen && (
        <div className="ag-panel">
          <div className="ag-panel__list">
            {graphs.map((g) => (
              <GraphRow
                key={g.id}
                graph={g}
                current={g.id === current?.id}
                mode={active?.id === g.id ? active.mode : "view"}
                setMode={(mode) => setActive(mode === "view" ? null : { id: g.id, mode })}
                onOpen={() => open(g.id)}
              />
            ))}
          </div>
          <div className="ag-panel__footer">
            <button className="ag-panel__add" onClick={createAndRename}>
              <Icon name="plus" />
              {t("Новый граф")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
