import { invoke } from "@tauri-apps/api/core";
import type { DocEdge, DocNode, GraphDoc, GraphMeta, GraphSliders } from "./doc";

// wrappers over the commands of src-tauri/src/graphs.rs

export const graphsApi = {
  init: () => invoke<{ graphs: GraphMeta[]; lastOpened: string | null }>("graphs_init"),
  load: (id: string) => invoke<GraphDoc>("graph_load", { id }),
  create: (name: string, nodes: DocNode[], edges: DocEdge[], showVolume: boolean, showDelay: boolean) =>
    invoke<GraphDoc>("graph_create", { name, nodes, edges, showVolume, showDelay }),
  save: (graph: GraphDoc) => invoke<GraphMeta>("graph_save", { graph }),
  rename: (id: string, name: string) => invoke<GraphMeta>("graph_rename", { id, name }),
  setSliders: (id: string, { showVolume, showDelay, delayLimit }: GraphSliders) =>
    invoke<GraphMeta>("graph_set_sliders", { id, volume: showVolume, delay: showDelay, delayLimit }),
  remove: (id: string) => invoke<void>("graph_delete", { id }),
  reorder: (ids: string[]) => invoke<void>("graphs_reorder", { ids }),
};
