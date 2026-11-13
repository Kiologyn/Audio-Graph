import { DEFAULT_DISTANCE_LIMIT, distanceGain } from "./space";
import { NODE_KINDS, type AudioEdge, type AudioNode } from "./types";

/** Node id → the position of its system slider, as read last */
export type SystemVolumes = Record<string, number | null>;

/** A node's volume as it acts: a device's or app's is what the graph sets or the system has, a cable's within the graph's limit */
export function effectiveVolume(node: AudioNode, limit: number, system: SystemVolumes): number {
  const { systemVolume, maxVolume } = NODE_KINDS[node.data.kind];
  if (systemVolume) return Math.min(node.data.systemVolume ?? system[node.id] ?? 1, 1);
  return Math.min(node.data.volume ?? 1, limit, maxVolume ?? 1);
}

/**
 * The volume of every path into an output, from where its sound starts: the nodes' volumes multiply.
 * Apps and "System" are where sound starts: what they listen to isn't what they play.
 * A cable nothing goes into plays what comes from outside the graph. An output passes on what is played
 * into its device: its own slider comes after that, so it doesn't count here. A cable far away in space is quieter;
 * which way it is doesn't count: in front it plays as loud as set, elsewhere the head only shades it a little.
 */
export function pathVolumes(id: string, nodes: AudioNode[], edges: AudioEdge[], limit: number, system: SystemVolumes, distanceLimit = DEFAULT_DISTANCE_LIMIT): number[] {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const output = byId.get(id);
  if (!output) return [];

  const into = (target: string, seen: string[]): number[] =>
    edges.flatMap((e) => {
      const source = byId.get(e.source);
      if (e.target !== target || !source || seen.includes(source.id)) return [];
      if (source.data.kind === "system") return [1];
      const own = source.data.kind === "output" ? 1 : effectiveVolume(source, limit, system) * distanceGain(source.data.place ?? null, distanceLimit);
      if (source.data.kind !== "cable" && source.data.kind !== "output") return [own];
      const before = into(source.id, [...seen, source.id]);
      return before.length > 0 ? before.map((v) => v * own) : [own];
    });

  const volume = effectiveVolume(output, limit, system);
  return into(id, [id]).map((v) => v * volume);
}

/** The real volume of an output if the way there changes it: "150%" or "80–150%"; null if it is just the slider's */
export function totalVolume(id: string, nodes: AudioNode[], edges: AudioEdge[], limit: number, system: SystemVolumes, distanceLimit = DEFAULT_DISTANCE_LIMIT): string | null {
  const output = nodes.find((n) => n.id === id);
  if (!output) return null;
  const percents = [...new Set(pathVolumes(id, nodes, edges, limit, system, distanceLimit).map((v) => Math.round(v * 100)))].sort((a, b) => a - b);
  const own = Math.round(effectiveVolume(output, limit, system) * 100);
  if (percents.length === 0 || percents.every((p) => p === own)) return null;
  const [low, high] = [percents[0], percents[percents.length - 1]];
  return low === high ? `${low}%` : `${low}–${high}%`;
}
