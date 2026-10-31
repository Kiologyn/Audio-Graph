import { t } from "../i18n";
import { cableDeviceIds, type Cable } from "./cables";
import type { AudioDevice } from "./devices";
import type { AppStatus, EngineStatus } from "./engine";
import type { AudioNode, AudioNodeData, DeviceRef } from "./types";

/** Пометка на ноде: error — что-то не работает, info — подождёт (приложение не запущено) */
export type Note = { text: string; level: "error" | "info" };

/** Строка списка проблем в шапке */
export type Problem = Note & { key: string; who: string | null };

/** Пояснение к приложению, которому устройства пока назначить нельзя (ключи перевода) */
const WAITING: Partial<Record<AppStatus["state"], string>> = {
  not_running: "Не запущено: устройства назначатся при запуске",
  no_audio: "Ещё не открывало звук: устройства назначатся, когда начнёт играть",
};

// ошибки устройств приходят из Rust со строчной буквы
const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** Системные устройства ноды: по ним ошибки потоков движка находят свои ноды */
function nodeDevices(data: AudioNodeData, cables: Cable[], devices: AudioDevice[]): string[] {
  const resolve = (ref: DeviceRef | null, flow: AudioDevice["flow"]) =>
    ref?.type === "device" ? ref.id : ref?.type === "default" ? devices.find((d) => d.flow === flow && d.isDefault)?.id : undefined;
  const ids =
    data.kind === "input"
      ? [resolve(data.source, "capture")]
      : data.kind === "output"
        ? [resolve(data.sink, "render")]
        : data.kind === "cable"
          ? cables.filter((c) => c.id === data.cable).flatMap(cableDeviceIds)
          : [];
  return ids.filter((id): id is string => !!id);
}

/**
 * Пометки ноды. Текст проблемы, касающейся нескольких нод (петля, связь приложения
 * с внутренним кабелем), пишется на первой из них, остальные только подсвечиваются.
 */
export function nodeNotes(
  id: string,
  data: AudioNodeData,
  status: EngineStatus,
  graphId: string | undefined,
  cables: Cable[],
  devices: AudioDevice[],
): { notes: Note[]; flagged: boolean } {
  if (!graphId || status.graphId !== graphId) return { notes: [], flagged: false };
  const notes: Note[] = [];
  let flagged = false;
  for (const issue of status.issues) {
    if (!issue.nodes.includes(id)) continue;
    flagged = true;
    if (issue.nodes[0] === id) notes.push({ text: issue.message, level: "error" });
  }
  if (status.running.includes(graphId)) {
    const app = status.apps.find((a) => a.node === id);
    if (app?.state === "failed") notes.push({ text: app.error ?? t("Не удалось назначить устройства"), level: "error" });
    const waiting = app && WAITING[app.state];
    if (waiting) notes.push({ text: t(waiting), level: "info" });
    const mine = new Set(nodeDevices(data, cables, devices));
    const seen = new Set<string>();
    for (const e of status.streams.flatMap((s) => s.errors)) {
      if (!mine.has(e.device) || seen.has(e.device + e.message)) continue;
      seen.add(e.device + e.message);
      notes.push({ text: capitalize(e.message), level: "error" });
    }
  }
  return { notes, flagged: flagged || notes.some((n) => n.level === "error") };
}

/** Все проблемы открытого графа для списка в шапке */
export function allProblems(
  status: EngineStatus,
  graphId: string | undefined,
  nodes: AudioNode[],
  cables: Cable[],
  devices: AudioDevice[],
): Problem[] {
  // подписи нод известны только для открытого графа
  const own = !!graphId && status.graphId === graphId;
  const label = (id: string) => (own ? (nodes.find((n) => n.id === id)?.data.label ?? null) : null);
  const list: Problem[] = status.issues.map((issue, i) => ({
    key: `issue-${i}`,
    who: issue.nodes.map(label).filter(Boolean).join(", ") || null,
    text: issue.message,
    level: "error",
  }));
  // у движка приложения и устройства всех запущенных графов, здесь — только этого
  if (own && status.running.includes(graphId)) {
    const mine = new Set(nodes.map((n) => n.id));
    const myDevices = new Set(nodes.flatMap((n) => nodeDevices(n.data, cables, devices)));
    for (const app of status.apps.filter((a) => mine.has(a.node))) {
      if (app.state === "failed") list.push({ key: `app-${app.node}`, who: label(app.node), text: app.error ?? t("Не удалось назначить устройства"), level: "error" });
      const waiting = WAITING[app.state];
      if (waiting) list.push({ key: `app-${app.node}`, who: label(app.node), text: t(waiting), level: "info" });
    }
    const seen = new Set<string>();
    for (const e of status.streams.flatMap((s) => s.errors)) {
      if (!myDevices.has(e.device) || seen.has(e.device + e.message)) continue;
      seen.add(e.device + e.message);
      list.push({ key: `device-${e.device}`, who: devices.find((d) => d.id === e.device)?.name ?? t("Устройство"), text: capitalize(e.message), level: "error" });
    }
  }
  return list;
}
