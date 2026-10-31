//! Хранилище графов: каждый граф — отдельный JSON-файл в папке данных приложения.

use crate::engine::Engine;
use crate::storage::{check_id, clean_name, err, now_ms, write_atomic, CmdResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// Версия формата файла графа. Повышается при несовместимых изменениях.
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
    /// Устройства по умолчанию в Windows
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

/// Привязка порта ноды к системному устройству
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceRef {
    /// Устройство по умолчанию: следует за его сменой в системе
    Default,
    /// Конкретное устройство. Имя запоминается, чтобы показывать его, пока устройство отключено
    Device { id: String, name: String },
}

/// Приложение, которое представляет нода приложения
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AppRef {
    /// Имя исполняемого файла: по нему приложение узнаётся, даже если сменилась папка
    pub exe: String,
    /// Полный путь, под которым приложение было выбрано
    #[serde(default)]
    pub path: String,
    /// Название, чтобы показывать его, пока приложение не запущено
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// Устройство вывода за входным портом: в него пишется звук (вывод)
    #[serde(default)]
    pub sink: Option<DeviceRef>,
    /// Устройство записи за выходным портом: из него берётся звук (ввод)
    #[serde(default)]
    pub source: Option<DeviceRef>,
    /// Кабель из общего списка (cables.rs), который представляет нода кабеля; нет — внутренний
    #[serde(default)]
    pub cable: Option<String>,
    /// Приложение, которое представляет нода приложения
    #[serde(default)]
    pub app: Option<AppRef>,
    /// Громкость: 1 — без изменений, 0 — тишина, больше 1 — усиление (только в потоках движка)
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
    /// Регуляторы громкости на нодах этого графа
    #[serde(default = "yes")]
    pub show_volume: bool,
}

/// Краткие сведения о графе для списка
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
    // команды Tauri выполняются параллельно, запись в файлы сериализуем
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

    /// Граф с диска для движка
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
            // повреждённый файл не должен ломать весь список
            match fs::read_to_string(&path).map_err(err).and_then(|t| serde_json::from_str::<Graph>(&t).map_err(err)) {
                Ok(g) => graphs.push(GraphMeta::from(&g)),
                Err(e) => eprintln!("пропущен файл графа {}: {e}", path.display()),
            }
        }
        graphs.sort_by_key(|g| g.created_at);
        Ok(graphs)
    }
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
    // граф могли удалить, пока висело отложенное сохранение: не воскрешаем его
    check_id(&graph.id)?;
    if !store.path(&graph.id).exists() {
        return Err(tr!("граф уже удалён"));
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

/// Регуляторы громкости графа, который не открыт в окне (открытый сохраняется целиком)
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
