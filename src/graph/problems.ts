import { t } from "../i18n";
import { dynamicCables, virtualDeviceIds } from "../platform";
import { cableDeviceIds, type Cable } from "./cables";
import type { AudioDevice } from "./devices";
import type { AppStatus, EngineStatus } from "./engine";
import type { AudioNode, AudioNodeData, DeviceRef } from "./types";

/** A note on a node: error means something doesn't work, info means it waits (app not running) */
export type Note = { text: string; level: "error" | "info" };

/** A row of the problem list in the header */
export type Problem = Note & { key: string; who: string | null };

/** Explanation for an app that can't get devices yet (translation keys) */
const WAITING: Partial<Record<AppStatus["state"], string>> = {
  not_running: "app_not_running",
  no_audio: "app_no_audio",
};

// device errors come from Rust in lower case
const capitalize = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** A node's system devices: engine stream errors find their nodes by them */
function nodeDevices(id: string, data: AudioNodeData, cables: Cable[], devices: AudioDevice[]): string[] {
  const resolve = (ref: DeviceRef | null, flow: AudioDevice["flow"]) =>
    ref?.type === "device" ? ref.id : ref?.type === "default" ? devices.find((d) => d.flow === flow && d.isDefault)?.id : undefined;
  const ids =
    data.kind === "input"
      ? [resolve(data.source, "capture")]
      : data.kind === "output"
        ? [resolve(data.sink, "render")]
        : data.kind === "cable"
          ? dynamicCables()
            ? virtualDeviceIds(id)
            : cables.filter((c) => c.id === data.cable).flatMap(cableDeviceIds)
          : [];
  return ids.filter((id): id is string => !!id);
}

/**
 * A node's notes. The text of a problem involving several nodes (a loop, an app's link
 * with an internal cable) goes on the first of them, the others are only highlighted.
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
    if (app?.state === "failed") notes.push({ text: app.error ?? t("error_assign_devices"), level: "error" });
    const waiting = app && WAITING[app.state];
    if (waiting) notes.push({ text: t(waiting), level: "info" });
    const mine = new Set(nodeDevices(id, data, cables, devices));
    const seen = new Set<string>();
    for (const e of status.streams.flatMap((s) => s.errors)) {
      if (!mine.has(e.device) || seen.has(e.device + e.message)) continue;
      seen.add(e.device + e.message);
      notes.push({ text: capitalize(e.message), level: "error" });
    }
  }
  return { notes, flagged: flagged || notes.some((n) => n.level === "error") };
}

/** All problems of the open graph for the header list */
export function allProblems(
  status: EngineStatus,
  graphId: string | undefined,
  nodes: AudioNode[],
  cables: Cable[],
  devices: AudioDevice[],
): Problem[] {
  // node labels are known only for the open graph
  const own = !!graphId && status.graphId === graphId;
  const label = (id: string) => (own ? (nodes.find((n) => n.id === id)?.data.label ?? null) : null);
  const list: Problem[] = status.issues.map((issue, i) => ({
    key: `issue-${i}`,
    who: issue.nodes.map(label).filter(Boolean).join(", ") || null,
    text: issue.message,
    level: "error",
  }));
  // the engine has apps and devices of all running graphs, here only this one's
  if (own && status.running.includes(graphId)) {
    const mine = new Set(nodes.map((n) => n.id));
    const myDevices = new Set(nodes.flatMap((n) => nodeDevices(n.id, n.data, cables, devices)));
    for (const app of status.apps.filter((a) => mine.has(a.node))) {
      if (app.state === "failed") list.push({ key: `app-${app.node}`, who: label(app.node), text: app.error ?? t("error_assign_devices"), level: "error" });
      const waiting = WAITING[app.state];
      if (waiting) list.push({ key: `app-${app.node}`, who: label(app.node), text: t(waiting), level: "info" });
    }
    const seen = new Set<string>();
    for (const e of status.streams.flatMap((s) => s.errors)) {
      if (!myDevices.has(e.device) || seen.has(e.device + e.message)) continue;
      seen.add(e.device + e.message);
      list.push({ key: `device-${e.device}`, who: devices.find((d) => d.id === e.device)?.name ?? t("problem_device"), text: capitalize(e.message), level: "error" });
    }
  }
  return list;
}
