//! Graph storage: every graph is a separate JSON file in the app's data folder.

use crate::engine::Engine;
use crate::storage::{check_id, clean_name, err, now_ms, write_atomic, CmdResult};
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
}

impl From<&Graph> for GraphMeta {
    fn from(g: &Graph) -> Self {
        Self {
            id: g.id.clone(),
            name: g.name.clone(),
            created_at: g.created_at,
            updated_at: g.updated_at,
            show_volume: g.show_volume,
        }
    }
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

/// Volume sliders of a graph not open in the window (the open one is saved whole)
#[tauri::command]
pub fn graph_set_show_volume(store: State<GraphStore>, engine: State<Engine>, id: String, show: bool) -> CmdResult<GraphMeta> {
    let _guard = store.lock.lock().map_err(err)?;
    let mut graph = store.read(&id)?;
    graph.show_volume = show;
    graph.updated_at = now_ms();
    store.write(&graph)?;
    engine.saved(&graph);
    Ok(GraphMeta::from(&graph))
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
