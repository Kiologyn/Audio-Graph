import { invoke } from "@tauri-apps/api/core";
import type { DocEdge, DocNode, GraphDoc, GraphMeta, GraphPrefs } from "./doc";

// wrappers over the commands of src-tauri/src/graphs.rs

export const graphsApi = {
  init: () => invoke<{ graphs: GraphMeta[]; lastOpened: string | null }>("graphs_init"),
  load: (id: string) => invoke<GraphDoc>("graph_load", { id }),
  create: (name: string, nodes: DocNode[], edges: DocEdge[]) => invoke<GraphDoc>("graph_create", { name, nodes, edges }),
  save: (graph: GraphDoc) => invoke<GraphMeta>("graph_save", { graph }),
  rename: (id: string, name: string) => invoke<GraphMeta>("graph_rename", { id, name }),
  setPrefs: (id: string, { delayLimit, volumeLimit, startOnLaunch }: GraphPrefs) =>
    invoke<GraphMeta>("graph_set_prefs", { id, prefs: { delayLimit, volumeLimit, startOnLaunch } }),
  remove: (id: string) => invoke<void>("graph_delete", { id }),
  reorder: (ids: string[]) => invoke<void>("graphs_reorder", { ids }),
  exportTo: (id: string, path: string) => invoke<void>("graph_export", { id, path }),
  importFrom: (path: string) => invoke<GraphDoc>("graph_import", { path }),
};
