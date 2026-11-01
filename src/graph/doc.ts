import type { Viewport } from "@xyflow/react";
import { NODE_KINDS, type AudioEdge, type AudioNode, type AudioNodeKind, type AppRef, type DeviceRef } from "./types";

/** The graph file format. Matches graphs.rs, independent of React Flow. */
export type GraphDoc = {
  version: number;
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
  viewport: Viewport | null;
  nodes: DocNode[];
  edges: DocEdge[];
  /** Volume sliders on nodes */
  showVolume: boolean;
  /** Delay sliders on output nodes; while off, the graph's delays aren't applied */
  showDelay: boolean;
  /** The delay sliders' range: from minus this many seconds to plus */
  delayLimit: number;
  /** Start the graph when the app starts with the system */
  startOnLaunch: boolean;
};

/** The widest delay slider range, matches MAX_DELAY_LIMIT in graphs.rs */
export const MAX_DELAY_LIMIT = 1;

export type DocNode = {
  id: string;
  kind: AudioNodeKind;
  label: string;
  sink: DeviceRef | null;
  source: DeviceRef | null;
  cable: string | null;
  app: AppRef | null;
  volume: number;
  /** Output delay in seconds; the engine gets it with autosave */
  delay: number;
  position: { x: number; y: number };
};

export type DocEdge = { id: string; source: string; target: string };

/** What the graph's options menu changes */
export type GraphPrefs = Pick<GraphDoc, "showVolume" | "showDelay" | "delayLimit" | "startOnLaunch">;

export type GraphMeta = Pick<GraphDoc, "id" | "name" | "createdAt" | "updatedAt"> & GraphPrefs;

export function nodesToDoc(nodes: AudioNode[]): DocNode[] {
  return nodes.map((n) => {
    const meta = NODE_KINDS[n.data.kind];
    // save only what a node of this kind can choose: nodes from old saves
    // may lack these fields, and cables kept devices from the former scheme
    return {
      id: n.id,
      kind: n.data.kind,
      label: n.data.label,
      sink: meta.sink ? (n.data.sink ?? null) : null,
      source: meta.source ? (n.data.source ?? null) : null,
      cable: meta.pickCable ? (n.data.cable ?? null) : null,
      app: meta.pickApp ? (n.data.app ?? null) : null,
      volume: meta.maxVolume ? (n.data.volume ?? 1) : 1,
      delay: meta.delay ? (n.data.delay ?? 0) : 0,
      position: { x: n.position.x, y: n.position.y },
    };
  });
}

export function edgesToDoc(edges: AudioEdge[]): DocEdge[] {
  return edges.map((e) => ({ id: e.id, source: e.source, target: e.target }));
}

export function nodesFromDoc(nodes: DocNode[]): AudioNode[] {
  return nodes.map((n) => ({
    id: n.id,
    type: "audio",
    position: n.position,
    data: {
      kind: n.kind,
      label: n.label,
      sink: n.sink ?? null,
      source: n.source ?? null,
      cable: n.cable ?? null,
      app: n.app ?? null,
      volume: n.volume ?? 1,
      delay: n.delay ?? 0,
    },
  }));
}

export function edgesFromDoc(edges: DocEdge[]): AudioEdge[] {
  return edges.map((e) => ({ ...e, sourceHandle: "out", targetHandle: "in" }));
}
