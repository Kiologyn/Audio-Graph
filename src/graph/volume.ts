import { NODE_KINDS, type AudioEdge, type AudioNode } from "./types";

/** A node's volume as it acts: within the graph's limit and what its kind can do */
export function effectiveVolume(node: AudioNode, limit: number): number {
  return Math.min(node.data.volume ?? 1, limit, NODE_KINDS[node.data.kind].maxVolume ?? 1);
}

/**
 * The volume of every path into an output, from where its sound starts: the nodes' volumes multiply.
 * Apps and "System" are where sound starts: what they listen to isn't what they play.
 * A cable nothing goes into plays what comes from outside the graph.
 * Straight from "System" the output's volume doesn't apply; straight from an app it goes
 * through the app's volume in the mixer, at most 100%
 */
export function pathVolumes(id: string, nodes: AudioNode[], edges: AudioEdge[], limit: number): number[] {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const output = byId.get(id);
  if (!output) return [];
  const volume = effectiveVolume(output, limit);

  const into = (target: string, seen: string[]): number[] =>
    edges.flatMap((e) => {
      const source = byId.get(e.source);
      if (e.target !== target || !source || seen.includes(source.id)) return [];
      if (source.data.kind === "system") return [1];
      const own = effectiveVolume(source, limit);
      if (source.data.kind !== "cable") return [own];
      const before = into(source.id, [...seen, source.id]);
      return before.length > 0 ? before.map((v) => v * own) : [own];
    });

  return edges.flatMap((e) => {
    const source = byId.get(e.source);
    if (e.target !== id || !source) return [];
    if (source.data.kind === "system") return [1];
    if (source.data.kind === "app") return [Math.min(1, effectiveVolume(source, limit) * volume)];
    return into(id, [id]).map((v) => v * volume);
  });
}

/** The real volume of an output if the way there changes it: "150%" or "80–150%"; null if it is just the slider's */
export function totalVolume(id: string, nodes: AudioNode[], edges: AudioEdge[], limit: number): string | null {
  const output = nodes.find((n) => n.id === id);
  if (!output) return null;
  const percents = [...new Set(pathVolumes(id, nodes, edges, limit).map((v) => Math.round(v * 100)))].sort((a, b) => a - b);
  const own = Math.round(effectiveVolume(output, limit) * 100);
  if (percents.length === 0 || percents.every((p) => p === own)) return null;
  const [low, high] = [percents[0], percents[percents.length - 1]];
  return low === high ? `${low}%` : `${low}–${high}%`;
}
