//! Graph storage: every graph is a separate JSON file in the app's data folder.

use crate::cables::{Cable, CableStore};
use crate::engine::{Engine, DYNAMIC_CABLES};
use crate::storage::{check_id, clean_name, err, now_ms, unique_name, write_atomic, CmdResult};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// Graph file format version. Raised on incompatible changes.
const FORMAT_VERSION: u32 = 1;

fn format_version() -> u32 {
    FORMAT_VERSION
}

fn full_volume() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

fn half_second() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Input,
    Output,
    App,
    Cable,
    /// The system's default devices
    System,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// A node port's binding to a system device
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceRef {
    /// The default device: follows its changes in the system
    Default,
    /// A specific device. The name is remembered to show it while the device is unplugged
    Device { id: String, name: String },
}

/// The app an app node stands for
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AppRef {
    /// Executable file name: the app is recognized by it even if its folder changed
    pub exe: String,
    /// The full path the app was chosen under
    #[serde(default)]
    pub path: String,
    /// The name, shown while the app isn't running
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// The output device behind the input port: sound is written into it (output)
    #[serde(default)]
    pub sink: Option<DeviceRef>,
    /// The recording device behind the output port: sound is taken from it (input)
    #[serde(default)]
    pub source: Option<DeviceRef>,
    /// The cable from the shared list (cables.rs) a cable node stands for; none means internal
    #[serde(default)]
    pub cable: Option<String>,
    /// The app an app node stands for
    #[serde(default)]
    pub app: Option<AppRef>,
    /// Volume: 1 is unchanged, 0 is silence, above 1 is gain (engine streams only)
    #[serde(default = "full_volume")]
    pub volume: f32,
    /// An output's delay in seconds relative to the other outputs: plus plays later, minus earlier
    #[serde(default)]
    pub delay: f32,
    /// Left-right balance from -1 (left only) to 1 (right only); engine streams only
    #[serde(default)]
    pub balance: f32,
    pub position: Position,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    #[serde(default = "format_version")]
    pub version: u32,
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub viewport: Option<Viewport>,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    /// Volume sliders on this graph's nodes
    #[serde(default = "yes")]
    pub show_volume: bool,
    /// Delay sliders on this graph's output nodes; while off, its delays aren't applied
    #[serde(default)]
    pub show_delay: bool,
    /// The delay sliders' range: from minus this many seconds to plus
    #[serde(default = "half_second")]
    pub delay_limit: f32,
    /// Start the graph when the app starts with the system
    #[serde(default)]
    pub start_on_launch: bool,
    /// Balance sliders on this graph's nodes; while off, its balances aren't applied
    #[serde(default)]
    pub show_balance: bool,
}

/// The widest delay slider range: delays relative to each other add up, so two of these must fit
/// into what a link can delay (plan::MAX_DELAY)
pub const MAX_DELAY_LIMIT: f32 = crate::engine::plan::MAX_DELAY / 2.0;

impl Graph {
    /// The delay slider range if the graph's delays are on, within what the engine can do
    pub fn delay_limit(&self) -> Option<f32> {
        self.show_delay.then(|| self.delay_limit.clamp(0.001, MAX_DELAY_LIMIT))
    }
}

/// A short summary of a graph for the list
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphMeta {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub show_volume: bool,
    pub show_delay: bool,
    pub delay_limit: f32,
    pub start_on_launch: bool,
    pub show_balance: bool,
}

impl From<&Graph> for GraphMeta {
    fn from(g: &Graph) -> Self {
        Self {
            id: g.id.clone(),
            name: g.name.clone(),
            created_at: g.created_at,
            updated_at: g.updated_at,
            show_volume: g.show_volume,
            show_delay: g.show_delay,
            delay_limit: g.delay_limit,
            start_on_launch: g.start_on_launch,
            show_balance: g.show_balance,
        }
    }
}

/// A graph's options from its menu in the graph list
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphPrefs {
    pub show_volume: bool,
    pub show_delay: bool,
    pub delay_limit: f32,
    pub start_on_launch: bool,
    pub show_balance: bool,
}

/// What a graph export file says about itself
const EXPORT_FORMAT: &str = "audio-graph";

/// An exported graph: the graph and the listed cables it uses, so the cables can be recreated
/// on another computer (on Linux the graph creates its cables itself, the list is empty)
#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    graph: Graph,
    #[serde(default)]
    cables: Vec<Cable>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    last_opened: Option<String>,
    /// Graph ids in the order of the list, as the user arranged it; graphs not in it go last
    #[serde(default)]
    graph_order: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInfo {
    pub graphs: Vec<GraphMeta>,
    pub last_opened: Option<String>,
}

pub struct GraphStore {
    graphs_dir: PathBuf,
    settings_path: PathBuf,
    // Tauri commands run in parallel, file writes are serialized
    lock: Mutex<()>,
}

impl GraphStore {
    pub fn new(data_dir: PathBuf) -> io::Result<Self> {
        let graphs_dir = data_dir.join("graphs");
        fs::create_dir_all(&graphs_dir)?;
        Ok(Self {
            graphs_dir,
            settings_path: data_dir.join("settings.json"),
            lock: Mutex::new(()),
        })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.graphs_dir.join(format!("{id}.json"))
    }

    /// A graph from disk for the engine
    pub fn load(&self, id: &str) -> CmdResult<Graph> {
        let _guard = self.lock.lock().map_err(err)?;
        self.read(id)
    }

    fn read(&self, id: &str) -> CmdResult<Graph> {
        check_id(id)?;
        let text = fs::read_to_string(self.path(id)).map_err(err)?;
        serde_json::from_str(&text).map_err(err)
    }

    fn write(&self, graph: &Graph) -> CmdResult<()> {
        check_id(&graph.id)?;
        let json = serde_json::to_vec_pretty(graph).map_err(err)?;
        write_atomic(&self.path(&graph.id), &json).map_err(err)
    }

    fn settings(&self) -> Settings {
        fs::read_to_string(&self.settings_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn set_last_opened(&self, id: Option<&str>) -> CmdResult<()> {
        let mut s = self.settings();
        s.last_opened = id.map(str::to_owned);
        let json = serde_json::to_vec_pretty(&s).map_err(err)?;
        write_atomic(&self.settings_path, &json).map_err(err)
    }

    fn list(&self) -> CmdResult<Vec<GraphMeta>> {
        let mut graphs = Vec::new();
        for entry in fs::read_dir(&self.graphs_dir).map_err(err)? {
            let path = entry.map_err(err)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // a damaged file must not break the whole list
            match fs::read_to_string(&path).map_err(err).and_then(|t| serde_json::from_str::<Graph>(&t).map_err(err)) {
                Ok(g) => graphs.push(GraphMeta::from(&g)),
                Err(e) => eprintln!("skipped graph file {}: {e}", path.display()),
            }
        }
        let order = self.settings().graph_order;
        let place = |id: &str| order.iter().position(|o| o == id).unwrap_or(usize::MAX);
        graphs.sort_by_key(|g| (place(&g.id), g.created_at));
        Ok(graphs)
    }

    /// The graphs to start when the app starts with the system, in the order of the list
    pub fn launch_graphs(&self) -> CmdResult<Vec<Graph>> {
        let _guard = self.lock.lock().map_err(err)?;
        let ids: Vec<String> = self.list()?.into_iter().filter(|g| g.start_on_launch).map(|g| g.id).collect();
        ids.iter().map(|id| self.read(id)).collect()
    }
}

/// Remembers the order of the graph list
#[tauri::command]
pub fn graphs_reorder(store: State<GraphStore>, ids: Vec<String>) -> CmdResult<()> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut s = store.settings();
    s.graph_order = ids;
    let json = serde_json::to_vec_pretty(&s).map_err(err)?;
    write_atomic(&store.settings_path, &json).map_err(err)
}

#[tauri::command]
pub fn graphs_init(store: State<GraphStore>) -> CmdResult<LibraryInfo> {
    let _guard = store.lock.lock().map_err(err)?;
    Ok(LibraryInfo {
        graphs: store.list()?,
        last_opened: store.settings().last_opened,
    })
}

#[tauri::command]
pub fn graph_load(store: State<GraphStore>, engine: State<Engine>, id: String) -> CmdResult<Graph> {
    let _guard = store.lock.lock().map_err(err)?;
    let graph = store.read(&id)?;
    store.set_last_opened(Some(&id))?;
    engine.opened(&graph);
    Ok(graph)
}

#[tauri::command]
pub fn graph_create(
    store: State<GraphStore>,
    engine: State<Engine>,
    name: String,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    show_volume: Option<bool>,
    show_delay: Option<bool>,
) -> CmdResult<Graph> {
    let _guard = store.lock.lock().map_err(err)?;
    let now = now_ms();
    let graph = Graph {
        version: FORMAT_VERSION,
        id: uuid::Uuid::new_v4().to_string(),
        name: clean_name(&name)?,
        created_at: now,
        updated_at: now,
        viewport: None,
        nodes,
        edges,
        show_volume: show_volume.unwrap_or(true),
        show_delay: show_delay.unwrap_or(false),
        delay_limit: half_second(),
        start_on_launch: false,
        show_balance: false,
    };
    store.write(&graph)?;
    store.set_last_opened(Some(&graph.id))?;
    engine.opened(&graph);
    Ok(graph)
}

#[tauri::command]
pub fn graph_save(store: State<GraphStore>, engine: State<Engine>, mut graph: Graph) -> CmdResult<GraphMeta> {
    let _guard = store.lock.lock().map_err(err)?;
    // the graph may have been deleted while a save was pending: don't bring it back
    check_id(&graph.id)?;
    if !store.path(&graph.id).exists() {
        return Err(tr!("error_graph_deleted"));
    }
    graph.version = FORMAT_VERSION;
    graph.name = clean_name(&graph.name)?;
    graph.updated_at = now_ms();
    store.write(&graph)?;
    engine.saved(&graph);
    Ok(GraphMeta::from(&graph))
}

#[tauri::command]
pub fn graph_rename(store: State<GraphStore>, engine: State<Engine>, id: String, name: String) -> CmdResult<GraphMeta> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut graph = store.read(&id)?;
    graph.name = clean_name(&name)?;
    graph.updated_at = now_ms();
    store.write(&graph)?;
    engine.saved(&graph);
    Ok(GraphMeta::from(&graph))
}

/// Options of a graph not open in the window (the open one is saved whole)
#[tauri::command]
pub fn graph_set_prefs(store: State<GraphStore>, engine: State<Engine>, id: String, prefs: GraphPrefs) -> CmdResult<GraphMeta> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut graph = store.read(&id)?;
    graph.show_volume = prefs.show_volume;
    graph.show_delay = prefs.show_delay;
    graph.delay_limit = prefs.delay_limit.clamp(0.001, MAX_DELAY_LIMIT);
    graph.start_on_launch = prefs.start_on_launch;
    graph.show_balance = prefs.show_balance;
    graph.updated_at = now_ms();
    store.write(&graph)?;
    engine.saved(&graph);
    Ok(GraphMeta::from(&graph))
}

/// Writes a graph into a file of the user's choice
#[tauri::command]
pub fn graph_export(store: State<GraphStore>, cables: State<CableStore>, id: String, path: PathBuf) -> CmdResult<()> {
    let graph = store.load(&id)?;
    let used: Vec<&str> = graph.nodes.iter().filter_map(|n| n.cable.as_deref()).collect();
    let listed = cables.all()?.into_iter().filter(|c| used.contains(&c.id.as_str())).collect();
    let file = ExportFile {
        format: EXPORT_FORMAT.into(),
        graph,
        cables: listed,
    };
    fs::write(path, serde_json::to_vec_pretty(&file).map_err(err)?).map_err(err)
}

/// Adds a graph from an export file (or a graph file from another computer's data folder) and opens it.
/// It gets new ids: the same file may be imported twice, and node ids are unique across graphs.
/// Its cables join the cable list
#[tauri::command]
pub fn graph_import(
    store: State<GraphStore>,
    cables: State<CableStore>,
    engine: State<Engine>,
    path: PathBuf,
) -> CmdResult<Graph> {
    let text = fs::read_to_string(&path).map_err(err)?;
    let (mut graph, listed) = match serde_json::from_str::<ExportFile>(&text) {
        Ok(f) if f.format == EXPORT_FORMAT => (f.graph, f.cables),
        _ => (serde_json::from_str::<Graph>(&text).map_err(|_| tr!("error_import_not_graph"))?, Vec::new()),
    };
    if !DYNAMIC_CABLES {
        let ids = cables.adopt(&listed, &engine)?;
        for node in &mut graph.nodes {
            if let Some(id) = node.cable.as_ref().and_then(|c| ids.get(c)) {
                node.cable = Some(id.clone());
            }
        }
    }
    renew_ids(&mut graph);

    let _guard = store.lock.lock().map_err(err)?;
    let names: Vec<String> = store.list()?.into_iter().map(|g| g.name).collect();
    let now = now_ms();
    graph.version = FORMAT_VERSION;
    graph.name = unique_name(&clean_name(&graph.name)?, names.iter().map(String::as_str));
    graph.created_at = now;
    graph.updated_at = now;
    // an imported graph starts with the app only once it is marked on this computer
    graph.start_on_launch = false;
    store.write(&graph)?;
    store.set_last_opened(Some(&graph.id))?;
    engine.opened(&graph);
    Ok(graph)
}

/// New ids for a graph, its nodes and edges; edges follow their nodes
fn renew_ids(graph: &mut Graph) {
    let new_id = || uuid::Uuid::new_v4().to_string();
    graph.id = new_id();
    let ids: HashMap<String, String> = graph.nodes.iter().map(|n| (n.id.clone(), new_id())).collect();
    for node in &mut graph.nodes {
        node.id = ids[&node.id].clone();
    }
    // edges to nodes that aren't there are dropped
    graph.edges.retain(|e| ids.contains_key(&e.source) && ids.contains_key(&e.target));
    for edge in &mut graph.edges {
        edge.source = ids[&edge.source].clone();
        edge.target = ids[&edge.target].clone();
        edge.id = format!("{}->{}", edge.source, edge.target);
    }
}

#[tauri::command]
pub fn graph_delete(store: State<GraphStore>, engine: State<Engine>, id: String) -> CmdResult<()> {
    let _guard = store.lock.lock().map_err(err)?;
    check_id(&id)?;
    fs::remove_file(store.path(&id)).map_err(err)?;
    engine.deleted(&id);
    if store.settings().last_opened.as_deref() == Some(id.as_str()) {
        store.set_last_opened(None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_graph_gets_new_ids() {
        let text = r#"{"id":"g","name":"Music","createdAt":1,"updatedAt":1,
            "nodes":[{"id":"a","kind":"system","label":"System","position":{"x":0,"y":0}},
                     {"id":"b","kind":"output","label":"Speakers","position":{"x":1,"y":0}}],
            "edges":[{"id":"a->b","source":"a","target":"b"},{"id":"a->gone","source":"a","target":"gone"}]}"#;
        let mut graph: Graph = serde_json::from_str(text).unwrap();
        renew_ids(&mut graph);
        assert_ne!(graph.id, "g");
        let (a, b) = (&graph.nodes[0].id, &graph.nodes[1].id);
        assert!(a != "a" && b != "b");
        // the edge follows its nodes, the edge to a missing node is gone
        assert_eq!(graph.edges.len(), 1);
        assert_eq!((&graph.edges[0].source, &graph.edges[0].target), (a, b));
        assert_eq!(graph.edges[0].id, format!("{a}->{b}"));
    }

    #[test]
    fn unique_names() {
        assert_eq!(unique_name("Music", ["Games"]), "Music");
        assert_eq!(unique_name("Music", ["Music", "Music 2"]), "Music 3");
    }
}
