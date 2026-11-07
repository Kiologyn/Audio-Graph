import type { Camera } from "./camera";
import {
  NODE_KINDS, NO_GATE,
  nodeSliders,
  type AudioEdge,
  type AudioNode,
  type AudioNodeKind,
  type AppRef,
  type DeviceRef,
  type NodeSlider,
} from "./types";

/** The graph file format. Matches graphs.rs, independent of React Flow. */
export type GraphDoc = {
  version: number;
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
  viewport: Camera | null;
  nodes: DocNode[];
  edges: DocEdge[];
  /** The delay sliders' range: from minus this many seconds to plus (cables from 0) */
  delayLimit: number;
  /** The volume sliders' maximum: 2 is 200% */
  volumeLimit: number;
  /** Start the graph when the app starts with the system */
  startOnLaunch: boolean;
};

/** The widest delay slider range, matches MAX_DELAY_LIMIT in graphs.rs */
export const MAX_DELAY_LIMIT = 600;

export type DocNode = {
  id: string;
  kind: AudioNodeKind;
  label: string;
  sink: DeviceRef | null;
  source: DeviceRef | null;
  cable: string | null;
  app: AppRef | null;
  volume: number;
  /** The system's slider while the graph runs; null leaves it alone */
  systemVolume: number | null;
  /** Output or cable delay in seconds; the engine gets it with autosave */
  delay: number;
  /** Left-right balance, -1..1 */
  balance: number;
  /** A cable's noise gate in dB */
  gate: number;
  /** The sliders that stay on the node */
  pinned: NodeSlider[];
  position: { x: number; y: number };
};

export type DocEdge = { id: string; source: string; target: string };

/** What the graph's options menu changes */
export type GraphPrefs = Pick<GraphDoc, "delayLimit" | "volumeLimit" | "startOnLaunch">;

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
      systemVolume: meta.systemVolume ? (n.data.systemVolume ?? null) : null,
      delay: meta.delay ? (n.data.delay ?? 0) : 0,
      balance: meta.balance ? (n.data.balance ?? 0) : 0,
      gate: meta.gate ? (n.data.gate ?? NO_GATE) : NO_GATE,
      pinned: nodeSliders(n.data.kind).filter((slider) => n.data.pinned?.includes(slider)),
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
      systemVolume: n.systemVolume ?? null,
      delay: n.delay ?? 0,
      balance: n.balance ?? 0,
      gate: n.gate ?? NO_GATE,
      pinned: n.pinned ?? [],
    },
  }));
}

export function edgesFromDoc(edges: DocEdge[]): AudioEdge[] {
  return edges.map((e) => ({ ...e, sourceHandle: "out", targetHandle: "in" }));
}
