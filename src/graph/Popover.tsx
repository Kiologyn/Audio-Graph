import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type Ref, type RefObject } from "react";
import { createPortal } from "react-dom";
import { t } from "../i18n";
import { Icon } from "./icons";

/** Кнопка с текущим выбором, открывающая выпадающий список */
export function PickerChip({
  ref,
  text,
  title,
  empty,
  missing,
  off,
  open,
  onClick,
}: {
  ref?: Ref<HTMLButtonElement>;
  text: string;
  title?: string;
  /** ничего не выбрано: текст-заглушка */
  empty?: boolean;
  /** выбранное недоступно: устройства нет в системе или кабель удалён */
  missing?: boolean;
  /** выбранное выключено в системе */
  off?: boolean;
  open: boolean;
  onClick: () => void;
}) {
  return (
    <button
      ref={ref}
      className={`ag-device nodrag${empty ? " is-empty" : ""}${missing ? " is-missing" : ""}${off ? " is-off" : ""}${open ? " is-open" : ""}`}
      onClick={onClick}
      title={title}
    >
      <span className="ag-device__name">{text}</span>
      <Icon name="chevron" size={12} />
    </button>
  );
}

export function PopoverItem({
  name,
  hint,
  selected,
  className = "",
  onClick,
}: {
  name: string;
  hint?: string | null;
  selected?: boolean;
  className?: string;
  onClick: () => void;
}) {
  return (
    <button className={`ag-popover__item${selected ? " is-selected" : ""} ${className}`} onClick={onClick} title={name}>
      <span className="ag-popover__name">{name}</span>
      {hint && <span className="ag-popover__hint">{hint}</span>}
    </button>
  );
}

/**
 * Выпадающий список у кнопки-якоря. Рисуется поверх всего приложения (портал),
 * поэтому не масштабируется вместе с холстом и не прячется под нодами.
 */
export function Popover({
  anchor,
  busy,
  footer,
  onClose,
  children,
}: {
  anchor: RefObject<HTMLElement | null>;
  /** идёт обновление списка */
  busy?: boolean;
  footer?: ReactNode;
  onClose: () => void;
  children: ReactNode;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties>({ visibility: "hidden" });
  // обработчики подписываются один раз, а onClose каждый раз новый
  const close = useRef(onClose);
  close.current = onClose;

  // под якорем, а если снизу мало места — над ним; не вылезать за правый край.
  // Ширина меняется, когда приходит обновлённый список, поэтому следим за размером.
  useLayoutEffect(() => {
    const el = menu.current;
    if (!el) return;
    const place = () => {
      const a = anchor.current?.getBoundingClientRect();
      if (!a) return;
      const gap = 4;
      const below = window.innerHeight - a.bottom - gap - 8;
      const above = a.top - gap - 8;
      const placeBelow = below >= 240 || below >= above;
      const minWidth = Math.max(a.width, 240);
      const width = Math.max(el.offsetWidth, minWidth);
      setStyle({
        left: Math.max(8, Math.min(a.left, window.innerWidth - width - 8)),
        minWidth,
        maxHeight: placeBelow ? below : above,
        ...(placeBelow ? { top: a.bottom + gap } : { bottom: window.innerHeight - a.top + gap }),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(el);
    return () => observer.disconnect();
  }, [anchor]);

  // закрывать по клику мимо, Escape, прокрутке/зуму холста и изменению размера окна
  useEffect(() => {
    const inside = (e: Event) =>
      menu.current?.contains(e.target as Node) || anchor.current?.contains(e.target as Node);
    const onDown = (e: PointerEvent) => !inside(e) && close.current();
    const onWheel = (e: WheelEvent) => !inside(e) && close.current();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close.current();
    const onResize = () => close.current();
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("wheel", onWheel, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("wheel", onWheel, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", onResize);
    };
  }, [anchor]);

  return createPortal(
    <div className="ag-popover" ref={menu} style={style}>
      {busy && <span className="ag-spinner ag-popover__spinner" aria-label={t("Обновление")} />}
      <div className="ag-popover__list">{children}</div>
      {footer && <div className="ag-popover__footer">{footer}</div>}
    </div>,
    document.body,
  );
}
