import {
  Background,
  BackgroundVariant,
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  useReactFlow,
  type DefaultEdgeOptions,
  type IsValidConnection,
  type XYPosition,
} from "@xyflow/react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import { useT } from "../i18n";
import { resolvedTheme, useSettings } from "../settings";
import AudioNodeView from "./AudioNodeView";
import { useEngine } from "./engine";
import { KindIcon } from "./icons";
import { useGraph } from "./store";
import { KIND_ORDER, NODE_KINDS, type AudioEdge, type AudioNode, type AudioNodeKind } from "./types";

const nodeTypes = { audio: AudioNodeView };

const defaultEdgeOptions: DefaultEdgeOptions = {
  markerEnd: { type: MarkerType.ArrowClosed, width: 18, height: 18, color: "var(--ag-edge)" },
};

// the minimap paints nodes with the fill attribute, CSS variables don't work there
const MINIMAP_COLORS: Record<AudioNodeKind, string> = {
  input: "#2fb87c",
  output: "#4a7ef0",
  app: "#9c66ef",
  cable: "#e5a02a",
  system: "#8c96a6",
};

// an arrow into itself makes no sense; addEdge drops duplicate edges
const isValidConnection: IsValidConnection<AudioEdge> = (c) => c.source !== c.target;

type Menu =
  | { type: "pane"; x: number; y: number; flow: XYPosition }
  | { type: "node"; x: number; y: number; nodeId: string }
  | { type: "edge"; x: number; y: number; edgeId: string }
  | { type: "selection"; x: number; y: number };

export function Canvas() {
  const { nodes, edges, onNodesChange, onEdgesChange, onConnect, onReconnect, removeEdge, addNode } =
    useGraph(
      useShallow((s) => ({
        nodes: s.nodes,
        edges: s.edges,
        onNodesChange: s.onNodesChange,
        onEdgesChange: s.onEdgesChange,
        onConnect: s.onConnect,
        onReconnect: s.onReconnect,
        removeEdge: s.removeEdge,
        addNode: s.addNode,
      })),
    );
  // an arrow pulled off a port and dropped elsewhere is deleted.
  // Dragging starts on mouse down already, so a plain click at a port
  // is told apart by distance, otherwise a click would delete the arrow.
  const reconnected = useRef(true);
  const reconnectFrom = useRef<XYPosition | null>(null);
  const setViewport = useGraph((s) => s.setViewport);
  // the graph's camera is needed only on mount, after that the canvas manages it
  const [initialViewport] = useState(() => useGraph.getState().viewport);
  const { screenToFlowPosition, deleteElements } = useReactFlow<AudioNode, AudioEdge>();
  const status = useEngine((s) => s.status);
  const graphId = useGraph((s) => s.meta?.id);
  // edges the engine reported a problem with are drawn as red dashes
  // The mark is set and cleared explicitly: an edge that once got into the store with the mark
  // doesn't stay red after the fix
  const shownEdges = useMemo(() => {
    const own = !!graphId && status.graphId === graphId;
    const flagged = new Set(own ? status.issues.flatMap((i) => i.edges) : []);
    return edges.map((e) => {
      const className = flagged.has(e.id) ? "is-issue" : undefined;
      return e.className === className ? e : { ...e, className };
    });
  }, [edges, status, graphId]);
  const wrapper = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const theme = useSettings((s) => s.theme);
  const hideMinimap = useSettings((s) => s.hideMinimap);
  const t = useT();

  // Ctrl+Z undoes, Ctrl+Y and Ctrl+Shift+Z redo. By key code, so it works
  // in other keyboard layouts too. Text fields have the browser's own text undo;
  // a volume slider keeps focus after dragging but has no undo of its own
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.altKey || e.metaKey) return;
      const el = e.target as HTMLElement | null;
      const typing =
        el?.isContentEditable ||
        el instanceof HTMLTextAreaElement ||
        (el instanceof HTMLInputElement && !["range", "checkbox", "radio", "button"].includes(el.type));
      if (typing) return;
      const { undo, redo } = useGraph.getState();
      if (e.code === "KeyZ") (e.shiftKey ? redo : undo)();
      else if (e.code === "KeyY" && !e.shiftKey) redo();
      else return;
      e.preventDefault();
      setMenu(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenu(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menu]);

  // a menu opened near the canvas edge moves inside it instead of being cut off; measured before it is shown
  const menuBox = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<XYPosition | null>(null);
  useLayoutEffect(() => {
    const el = menuBox.current;
    const box = wrapper.current;
    if (!menu || !el || !box) return;
    setPlace({
      x: Math.max(4, Math.min(menu.x, box.clientWidth - el.offsetWidth - 4)),
      y: Math.max(4, Math.min(menu.y, box.clientHeight - el.offsetHeight - 4)),
    });
  }, [menu]);

  // menu coordinates relative to the canvas container
  const local = (e: MouseEvent | ReactMouseEvent) => {
    const rect = wrapper.current!.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  return (
    // the canvas doesn't need the WebView system menu (Back, Refresh, Inspect)
    <div className="ag-canvas" ref={wrapper} onContextMenu={(e) => e.preventDefault()}>
      <ReactFlow<AudioNode, AudioEdge>
        nodes={nodes}
        edges={shownEdges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        reconnectRadius={14}
        // the selected arrow is raised to the top, so on a shared port it is the one grabbed
        elevateEdgesOnSelect
        onReconnectStart={(e) => {
          reconnected.current = false;
          reconnectFrom.current = { x: e.clientX, y: e.clientY };
        }}
        onReconnect={(oldEdge, connection) => {
          reconnected.current = true;
          onReconnect(oldEdge, connection);
        }}
        onReconnectEnd={(e, edge) => {
          const p = "changedTouches" in e ? e.changedTouches[0] : e;
          const from = reconnectFrom.current;
          const dragged = from !== null && Math.hypot(p.clientX - from.x, p.clientY - from.y) > 6;
          if (!reconnected.current && dragged) removeEdge(edge.id);
          reconnected.current = true;
          reconnectFrom.current = null;
        }}
        isValidConnection={isValidConnection}
        defaultEdgeOptions={defaultEdgeOptions}
        deleteKeyCode={["Delete", "Backspace"]}
        colorMode={theme === "system" ? "system" : resolvedTheme(theme)}
        proOptions={{ hideAttribution: true }}
        // a new graph is fit to the window, a saved one opens where it was left
        fitView={!initialViewport}
        fitViewOptions={{ padding: 0.3, maxZoom: 1 }}
        defaultViewport={initialViewport ?? undefined}
        onPaneClick={() => setMenu(null)}
        onMoveStart={() => setMenu(null)}
        onMoveEnd={(_, viewport) => setViewport(viewport)}
        onPaneContextMenu={(e) => {
          e.preventDefault();
          setMenu({ type: "pane", ...local(e), flow: screenToFlowPosition({ x: e.clientX, y: e.clientY }) });
        }}
        onNodeContextMenu={(e, node) => {
          e.preventDefault();
          setMenu({ type: "node", ...local(e), nodeId: node.id });
        }}
        onEdgeContextMenu={(e, edge) => {
          e.preventDefault();
          setMenu({ type: "edge", ...local(e), edgeId: edge.id });
        }}
        onSelectionContextMenu={(e) => {
          e.preventDefault();
          setMenu({ type: "selection", ...local(e) });
        }}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1.5} />
        <Controls showInteractive={false} />
        {!hideMinimap && (
          <MiniMap pannable zoomable nodeColor={(n) => MINIMAP_COLORS[(n as AudioNode).data.kind]} />
        )}
      </ReactFlow>

      {menu && (
        <div
          ref={menuBox}
          className="ag-menu"
          style={{ left: place?.x ?? menu.x, top: place?.y ?? menu.y }}
          onContextMenu={(e) => e.preventDefault()}
        >
          {menu.type === "pane" ? (
            <>
              <div className="ag-menu__title">{t("add_node")}</div>
              {KIND_ORDER.map((kind: AudioNodeKind) => (
                <button
                  key={kind}
                  className={`ag-menu__item ag-menu__item--${kind}`}
                  onClick={() => {
                    addNode(kind, menu.flow);
                    setMenu(null);
                  }}
                >
                  <KindIcon kind={kind} />
                  {t(NODE_KINDS[kind].title)}
                </button>
              ))}
            </>
          ) : (
            <button
              className="ag-menu__item ag-menu__item--danger"
              onClick={() => {
                if (menu.type === "node") deleteElements({ nodes: [{ id: menu.nodeId }] });
                else if (menu.type === "edge") deleteElements({ edges: [{ id: menu.edgeId }] });
                else
                  deleteElements({
                    nodes: nodes.filter((n) => n.selected),
                    edges: edges.filter((e) => e.selected),
                  });
                setMenu(null);
              }}
            >
              {t("delete")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
