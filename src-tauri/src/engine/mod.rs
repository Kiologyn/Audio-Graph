//! The audio engine: runs the started graphs and keeps them working while devices change,
//! apps start and close, and the graph or the cable list is edited.
//!
//! One supervisor thread does it all: it receives changes, checks the system every TICK,
//! rebuilds the plan (plan.rs) and brings audio streams and app devices in line with it.

#[cfg(windows)]
mod link;
#[cfg(target_os = "linux")]
mod linux;
pub(crate) mod plan;
#[cfg(windows)]
mod ring;
#[cfg(windows)]
mod win;

use crate::cables::Cable;
use crate::devices::{AudioDevice, Flow};
use crate::graphs::{DeviceRef, Graph, GraphStore, NodeKind};
use crate::settings::Settings;
use crate::storage::CmdResult;
use plan::Issue;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// How often the engine checks the system: default devices, failed streams,
/// running apps. More often on Linux: new app streams are moved by this poll,
/// and polling the audio server is cheap
const TICK: Duration = Duration::from_millis(if cfg!(target_os = "linux") { 500 } else { 1500 });

/// Cables are virtual devices the graph creates itself (Linux), not device pairs
/// from the shared list (on Windows a driver installs the virtual devices)
pub const DYNAMIC_CABLES: bool = cfg!(target_os = "linux");

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceError {
    pub device: String,
    pub message: String,
}

/// An engine stream: what plays into which device and what is wrong with it
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    pub sink: String,
    pub sources: Vec<String>,
    pub errors: Vec<DeviceError>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    /// Devices are assigned
    Routed,
    /// The app isn't running: devices are assigned when it starts
    NotRunning,
    /// The app runs but hasn't opened audio yet: devices are assigned when it does
    NoAudio,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub node: String,
    pub state: AppState,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// Running graphs in start order
    pub running: Vec<String>,
    /// The graph the issues belong to: the one open in the window, or else the first running one
    pub graph_id: Option<String>,
    pub issues: Vec<Issue>,
    /// Devices of all running graphs
    pub streams: Vec<StreamStatus>,
    /// Apps of all running graphs (node ids are unique across graphs)
    pub apps: Vec<AppStatus>,
    /// Running graphs where something doesn't work
    pub troubled: Vec<String>,
}

#[cfg(not(any(windows, target_os = "linux")))]
struct Backend;

#[cfg(not(any(windows, target_os = "linux")))]
impl Backend {
    fn new(_: &Path) -> Self {
        Backend
    }
    fn defaults(&self) -> plan::Defaults {
        plan::Defaults::default()
    }
    fn apply(&mut self, _: &plan::Plan) -> (Vec<StreamStatus>, Vec<AppStatus>) {
        (Vec::new(), Vec::new())
    }
    fn retry_now(&mut self) {}
    fn shutdown(&mut self) {}
}

#[cfg(windows)]
use win::Backend;

#[cfg(target_os = "linux")]
use linux::Backend;

enum Msg {
    /// A graph is open in the window: problems are shown for it while nothing runs
    Opened(Graph),
    Saved(Graph),
    Deleted(String),
    Cables(Vec<Cable>),
    /// Start a graph; refresh one that already runs
    Start(Graph),
    /// Stop a graph, None stops all
    Stop(Option<String>),
    /// From the tray: start or stop the open graph
    Toggle,
    /// A volume slider is moving: apply at once without waiting for autosave
    Volume { graph: String, node: String, volume: f32 },
    /// A balance slider is moving, the same way
    Balance { graph: String, node: String, balance: f32 },
    /// Settings changed: message and tray language, turning cables off
    Settings { disable_idle_cables: bool },
    /// A device was plugged in, unplugged, turned on or off: reconnect at once
    DevicesChanged,
    /// App exit: stop everything, give apps back their devices and reply
    Shutdown(mpsc::Sender<()>),
}

pub struct Engine {
    tx: mpsc::Sender<Msg>,
    status: Arc<Mutex<EngineStatus>>,
}

impl Engine {
    pub fn new(app: AppHandle, data_dir: PathBuf, cables: Vec<Cable>, settings: &Settings) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(EngineStatus::default()));
        let shared = status.clone();
        let disable_idle_cables = settings.disable_idle_cables;
        let watcher = tx.clone();
        crate::devices::watch(move || {
            let _ = watcher.send(Msg::DevicesChanged);
        });
        std::thread::Builder::new()
            .name("audio engine".into())
            .spawn(move || supervise(rx, app, &data_dir, cables, disable_idle_cables, &shared))
            .expect("failed to start the engine thread");
        Self { tx, status }
    }

    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
    }

    pub fn opened(&self, graph: &Graph) {
        self.send(Msg::Opened(graph.clone()));
    }

    pub fn saved(&self, graph: &Graph) {
        self.send(Msg::Saved(graph.clone()));
    }

    pub fn deleted(&self, id: &str) {
        self.send(Msg::Deleted(id.to_string()));
    }

    pub fn cables_changed(&self, cables: Vec<Cable>) {
        self.send(Msg::Cables(cables));
    }

    /// Starts a graph; refreshes one that already runs
    pub fn start(&self, graph: &Graph) {
        self.send(Msg::Start(graph.clone()));
    }

    pub fn toggle(&self) {
        self.send(Msg::Toggle);
    }

    pub fn stop_all(&self) {
        self.send(Msg::Stop(None));
    }

    pub fn settings_changed(&self, settings: &Settings) {
        self.send(Msg::Settings {
            disable_idle_cables: settings.disable_idle_cables,
        });
    }

    /// Stops a graph and gives apps back their devices; waits, but no longer than a couple of seconds
    pub fn shutdown(&self) {
        let (tx, rx) = mpsc::channel();
        self.send(Msg::Shutdown(tx));
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
}

/// Devices of listed cables that the graphs use
fn cable_devices(graphs: &[Graph], cables: &[Cable]) -> BTreeSet<String> {
    let used: HashSet<&str> = graphs
        .iter()
        .flat_map(|g| &g.nodes)
        .filter(|n| n.kind == NodeKind::Cable)
        .filter_map(|n| n.cable.as_deref())
        .collect();
    cables
        .iter()
        .filter(|c| used.contains(c.id.as_str()))
        .flat_map(|c| [&c.render, &c.capture])
        .filter_map(|d| match d {
            Some(DeviceRef::Device { id, .. }) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

/// Turns cable devices on or off; errors go to the log
fn power_cables(ids: &BTreeSet<String>, enabled: bool) {
    if ids.is_empty() {
        return;
    }
    let ids: Vec<String> = ids.iter().cloned().collect();
    if let Err(e) = crate::devices::set_enabled(&ids, enabled) {
        let what = if enabled { "turn on" } else { "turn off" };
        crate::log::write(format!("failed to {what} cables: {e}"));
    }
}

fn supervise(
    rx: mpsc::Receiver<Msg>,
    app: AppHandle,
    data_dir: &Path,
    mut cables: Vec<Cable>,
    mut disable_idle_cables: bool,
    status: &Mutex<EngineStatus>,
) {
    #[cfg(windows)]
    let _com = crate::com::ComGuard::init();
    let mut backend = Backend::new(data_dir);
    let mut opened: Option<Graph> = None;
    // in start order: when graphs contend for an app or the default devices, the one started first wins
    let mut running: Vec<Graph> = Vec::new();
    let mut reply = None;
    // what the tray shows now: names of running graphs, the open graph and whether it runs, the language
    let mut tray_shows: Option<(Vec<String>, Option<String>, bool, crate::settings::Language)> = None;
    // cable devices of running graphs: once no graph uses them, they can be turned off
    let mut powered: BTreeSet<String> = BTreeSet::new();
    // devices were turned on or off: the window needs to reread their list
    let mut force_emit = false;
    // what is already in the log: every error is written when it appears
    let mut logged_running: Vec<String> = Vec::new();
    let mut logged_errors: HashSet<String> = HashSet::new();
    // the system's devices: chosen devices that changed their id are found in it by name
    let mut devices = crate::devices::list().unwrap_or_default();
    let same = |g: &Option<Graph>, id: &str| g.as_ref().is_some_and(|g| g.id == id);

    loop {
        let mut messages: Vec<Msg> = Vec::new();
        match rx.recv_timeout(TICK) {
            Ok(msg) => messages.push(msg),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // a burst of changes (autosave while dragging) is applied at once
        messages.extend(rx.try_iter());
        let mut devices_changed = false;
        for msg in messages {
            match msg {
                Msg::Opened(g) => opened = Some(g),
                Msg::Saved(g) => {
                    for r in running.iter_mut().filter(|r| r.id == g.id) {
                        *r = g.clone();
                    }
                    if same(&opened, &g.id) {
                        opened = Some(g);
                    }
                }
                Msg::Deleted(id) => {
                    running.retain(|g| g.id != id);
                    if same(&opened, &id) {
                        opened = None;
                    }
                }
                Msg::Cables(c) => cables = c,
                Msg::Start(g) => match running.iter_mut().find(|r| r.id == g.id) {
                    Some(r) => *r = g,
                    None => running.push(g),
                },
                Msg::Stop(Some(id)) => running.retain(|g| g.id != id),
                Msg::Stop(None) => running.clear(),
                Msg::Volume { graph, node, volume } => {
                    for g in running.iter_mut().chain(opened.as_mut()).filter(|g| g.id == graph) {
                        if let Some(n) = g.nodes.iter_mut().find(|n| n.id == node) {
                            n.volume = volume;
                        }
                    }
                }
                Msg::Balance { graph, node, balance } => {
                    for g in running.iter_mut().chain(opened.as_mut()).filter(|g| g.id == graph) {
                        if let Some(n) = g.nodes.iter_mut().find(|n| n.id == node) {
                            n.balance = balance;
                        }
                    }
                }
                Msg::Toggle => {
                    if let Some(o) = &opened {
                        if running.iter().any(|r| r.id == o.id) {
                            running.retain(|r| r.id != o.id);
                        } else {
                            running.push(o.clone());
                        }
                    }
                }
                Msg::Settings { disable_idle_cables: disable } => {
                    disable_idle_cables = disable;
                    // the language may have changed: the tray is redrawn below, messages are rebuilt in the plan
                    tray_shows = None;
                }
                Msg::Shutdown(tx) => {
                    running.clear();
                    reply = Some(tx);
                }
                Msg::DevicesChanged => devices_changed = true,
            }
        }
        if devices_changed {
            devices = crate::devices::list().unwrap_or(devices);
            // streams of a device that is back open now, not after their retry pause
            backend.retry_now();
            let _ = app.emit("devices-changed", ());
        }

        // a graph's cables are turned on before the engine opens their devices
        let used = cable_devices(&running, &cables);
        if disable_idle_cables {
            let added: BTreeSet<String> = used.difference(&powered).cloned().collect();
            if !added.is_empty() {
                power_cables(&added, true);
                force_emit = true;
            }
        }

        let defaults = backend.defaults();
        // the open but stopped graph is checked last: it shows what it will run into when started
        let opened_running = opened.as_ref().is_some_and(|o| running.iter().any(|r| r.id == o.id));
        let resolved: Vec<Graph> = running
            .iter()
            .chain(opened.as_ref().filter(|_| !opened_running))
            .map(|g| resolve_devices(g, &devices))
            .collect();
        let checked: Vec<&Graph> = resolved.iter().collect();
        let listed: Vec<Cable> = cables.iter().map(|c| resolve_cable(c, &devices)).collect();
        let plans = if DYNAMIC_CABLES {
            // every cable node has its own virtual device: the plan is the same as with listed cables
            let virtualized: Vec<_> = checked.iter().map(|g| plan::virtualize(g)).collect();
            let graphs: Vec<&Graph> = virtualized.iter().map(|v| &v.0).collect();
            let cables: Vec<Cable> = virtualized.iter().flat_map(|v| v.1.iter().cloned()).collect();
            let mut plans = plan::compile_all(&graphs, &cables, &defaults);
            for (plan, v) in plans.iter_mut().zip(virtualized) {
                plan.virtual_cables = v.2;
            }
            plans
        } else {
            plan::compile_all(&checked, &listed, &defaults)
        };
        let mut plans = plans;
        // delays only in graphs that have them turned on
        let limits: Vec<Option<f32>> = checked.iter().map(|g| g.delay_limit()).collect();
        plan::apply_delays(&mut plans, running.len(), &limits);
        // while nothing runs the plan is empty: streams close, apps get their devices back
        let (streams, apps) = backend.apply(&plan::merge(&plans[..running.len()]));
        let shown = opened.as_ref().or(running.first());
        let issues = shown
            .and_then(|s| checked.iter().position(|g| g.id == s.id))
            .map(|i| plans[i].issues.clone())
            .unwrap_or_default();
        let troubled = running
            .iter()
            .zip(&plans)
            .filter(|(g, p)| {
                !p.issues.is_empty()
                    || apps.iter().any(|a| a.state == AppState::Failed && g.nodes.iter().any(|n| n.id == a.node))
            })
            .map(|(g, _)| g.id.clone())
            .collect();
        let next = EngineStatus {
            running: running.iter().map(|g| g.id.clone()).collect(),
            graph_id: shown.map(|g| g.id.clone()),
            issues,
            streams,
            apps,
            troubled,
        };
        // and turned off afterwards: streams are closed, default devices are restored
        if disable_idle_cables {
            let removed: BTreeSet<String> = powered.difference(&used).cloned().collect();
            if !removed.is_empty() {
                power_cables(&removed, false);
                force_emit = true;
            }
        }
        powered = used;

        log_changes(&running, &next, &mut logged_running, &mut logged_errors);
        let changed = {
            let mut current = status.lock().unwrap_or_else(|e| e.into_inner());
            let changed = *current != next;
            *current = next.clone();
            changed
        };
        if changed || std::mem::take(&mut force_emit) {
            let _ = app.emit("engine-status", next);
        }
        let names = (
            running.iter().map(|g| g.name.clone()).collect::<Vec<_>>(),
            opened.as_ref().map(|g| g.name.clone()),
            opened_running,
            crate::i18n::language(),
        );
        if tray_shows.as_ref() != Some(&names) {
            // the tray is created after the engine: nothing to update before that
            if let Some(tray) = app.try_state::<crate::tray::Tray>() {
                tray.update(&app, &names.0, names.1.as_deref(), names.2);
                tray_shows = Some(names);
            }
        }

        if let Some(tx) = reply.take() {
            backend.shutdown();
            let _ = tx.send(());
            return;
        }
    }
}

/// A chosen device's id as the system has it now (devices::find); unchanged if it isn't found
fn resolve_device(device: &mut Option<DeviceRef>, flow: Flow, devices: &[AudioDevice]) {
    if let Some(DeviceRef::Device { id, name }) = device {
        if let Some(found) = crate::devices::find(devices, flow, id, name) {
            *id = found.id.clone();
        }
    }
}

/// The graph with the ids of its chosen devices as the system has them now
fn resolve_devices(graph: &Graph, devices: &[AudioDevice]) -> Graph {
    let mut graph = graph.clone();
    for node in &mut graph.nodes {
        resolve_device(&mut node.sink, Flow::Render, devices);
        resolve_device(&mut node.source, Flow::Capture, devices);
    }
    graph
}

fn resolve_cable(cable: &Cable, devices: &[AudioDevice]) -> Cable {
    let mut cable = cable.clone();
    resolve_device(&mut cable.render, Flow::Render, devices);
    resolve_device(&mut cable.capture, Flow::Capture, devices);
    cable
}

/// Logs graph starts and stops and new engine errors
fn log_changes(running: &[Graph], status: &EngineStatus, last_running: &mut Vec<String>, last_errors: &mut HashSet<String>) {
    for g in running.iter().filter(|g| !last_running.contains(&g.id)) {
        crate::log::write(format!("graph started: \"{}\"", g.name));
    }
    for id in last_running.iter().filter(|id| !status.running.contains(id)) {
        crate::log::write(format!("graph stopped: {id}"));
    }
    *last_running = status.running.clone();
    let label = |id: &str| {
        running
            .iter()
            .flat_map(|g| &g.nodes)
            .find(|n| n.id == id)
            .map_or_else(|| id.to_string(), |n| n.label.clone())
    };
    let mut errors = HashSet::new();
    for app in &status.apps {
        if let Some(e) = &app.error {
            errors.insert(format!("«{}»: {e}", label(&app.node)));
        }
    }
    for e in status.streams.iter().flat_map(|s| &s.errors) {
        errors.insert(format!("device {}: {}", e.device, e.message));
    }
    for e in errors.difference(last_errors) {
        crate::log::write(format!("error: {e}"));
    }
    *last_errors = errors;
}

#[tauri::command]
pub fn engine_status(engine: State<Engine>) -> EngineStatus {
    engine.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub fn engine_start(engine: State<Engine>, graphs: State<GraphStore>, id: String) -> CmdResult<()> {
    engine.send(Msg::Start(graphs.load(&id)?));
    Ok(())
}

/// id: the graph to stop; without it, all
#[tauri::command]
pub fn engine_stop(engine: State<Engine>, id: Option<String>) {
    engine.send(Msg::Stop(id));
}

#[tauri::command]
pub fn engine_balance(engine: State<Engine>, graph: String, node: String, balance: f32) {
    engine.send(Msg::Balance {
        graph,
        node,
        balance: balance.clamp(-1.0, 1.0),
    });
}

#[tauri::command]
pub fn engine_volume(engine: State<Engine>, graph: String, node: String, volume: f32) {
    engine.send(Msg::Volume {
        graph,
        node,
        volume: volume.clamp(0.0, 2.0),
    });
}
