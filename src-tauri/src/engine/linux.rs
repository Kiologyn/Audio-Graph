//! The engine on Linux: the audio server (PulseAudio, or PipeWire through pipewire-pulse) gives
//! the devices that apps and the system see, the sound between devices goes through the engine.
//! - a cable is a virtual device: a null-sink (the cable's input) and a remap-source of its monitor (the output);
//! - a link between devices is the engine's own (engine/streams.rs): a recording stream, a buffer with
//!   the delay and the path volume, a playback stream, as on Windows;
//! - app devices mean moving their streams; default devices are a server setting.
//!
//! Server modules outlive a crash of Audio Graph, so our modules' arguments carry the mark
//! "audiograph": marked modules this run doesn't know about are removed. Previous default
//! devices, app devices and app volumes are journaled before a change and
//! restored after the graph stops, or, if the app is silent, when it opens audio.

pub(super) mod streams;

use super::levels::Levels;
use super::plan::{AppRoute, Plan, Stream as PlanStream, SystemRoute, Target, VirtualCable, LOOPBACK_PREFIX, VIRTUAL_PREFIX};
use super::streams::Streams;
use super::{AppState, AppStatus, DeviceError, StreamStatus};
use crate::apps::linux::{is_app_stream, is_running};
use crate::pulse::{self, Device, Pulse, Stream};
use crate::storage::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The mark in our modules' arguments
const MARK: &str = "audiograph";
/// How often to retry what failed (no device, the server refused)
const RETRY: Duration = Duration::from_secs(2);
/// How long to wait for a created device or stream to appear: PipeWire creates them
/// after answering the module load, PulseAudio right away
const APPEAR: Duration = Duration::from_secs(1);
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum Flow {
    Render,
    Capture,
}

/// The app's device before the graph changed it
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Saved {
    /// Executable file name in lower case
    exe: String,
    flow: Flow,
    /// None: the app followed the default device
    previous: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedDefault {
    flow: Flow,
    previous: Option<String>,
}

/// The app's mixer volume before the graph changed it
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedVolume {
    exe: String,
    previous: f32,
}

#[derive(Serialize, Deserialize, Default)]
struct Journal {
    saved: Vec<Saved>,
    defaults: Vec<SavedDefault>,
    volumes: Vec<SavedVolume>,
}

impl Journal {
    fn is_empty(&self) -> bool {
        self.saved.is_empty() && self.defaults.is_empty() && self.volumes.is_empty()
    }
}

struct LoadedCable {
    label: String,
    source: String,
    sink_module: u32,
    source_module: u32,
}

/// The server state at the start of a tick
struct Snapshot {
    server: pulse::Server,
    sinks: Vec<Device>,
    sources: Vec<Device>,
    inputs: Vec<Stream>,
    outputs: Vec<Stream>,
}

impl Snapshot {
    fn devices(&self, flow: Flow) -> &[Device] {
        match flow {
            Flow::Render => &self.sinks,
            Flow::Capture => &self.sources,
        }
    }

    fn streams(&self, flow: Flow) -> &[Stream] {
        match flow {
            Flow::Render => &self.inputs,
            Flow::Capture => &self.outputs,
        }
    }

    fn default(&self, flow: Flow) -> Option<&str> {
        match flow {
            Flow::Render => self.server.default_sink.as_deref(),
            Flow::Capture => self.server.default_source.as_deref(),
        }
    }

    fn device(&self, flow: Flow, name: &str) -> Option<&Device> {
        self.devices(flow).iter().find(|d| d.name == name)
    }

    fn name_of(&self, flow: Flow, index: u32) -> Option<&str> {
        self.devices(flow).iter().find(|d| d.index == index).map(|d| d.name.as_str())
    }
}

pub struct Backend {
    pulse: Option<Pulse>,
    journal_path: PathBuf,
    journal: Journal,
    /// Created cables: output device name → modules
    cables: BTreeMap<String, LoadedCable>,
    /// The sound between devices
    streams: Streams,
    /// What failed and why: don't retry more often than RETRY
    failed: HashMap<String, (Instant, String)>,
    /// Default devices the user chose (not our cables)
    user_default: HashMap<Flow, String>,
    /// Since when an app's sound fails to move: a short refusal is not an error
    move_failing: HashMap<(String, Flow), Instant>,
    /// Modules of previous runs are already removed
    clean: bool,
}

/// The device's label in the system: without quotes and backslashes that would break module arguments
fn description(label: &str) -> String {
    let label: String = label.chars().filter(|c| !matches!(c, '"' | '\'' | '\\') && !c.is_control()).collect();
    let label = label.trim();
    if label.is_empty() {
        "Audio Graph".to_string()
    } else {
        format!("{label} (Audio Graph)")
    }
}

fn is_virtual(name: &str) -> bool {
    name.starts_with(VIRTUAL_PREFIX)
}

impl Backend {
    pub fn new(data_dir: &Path, levels: Arc<Levels>) -> Self {
        let journal_path = data_dir.join("pulse-routing.json");
        let journal = fs::read_to_string(&journal_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Journal>(&t).ok())
            .unwrap_or_default();
        Self {
            pulse: None,
            journal_path,
            journal,
            cables: BTreeMap::new(),
            streams: Streams::new(levels),
            failed: HashMap::new(),
            user_default: HashMap::new(),
            move_failing: HashMap::new(),
            clean: false,
        }
    }

    /// The server connection; a broken one is reopened
    fn pulse(&mut self) -> Result<&mut Pulse, String> {
        if !self.pulse.as_ref().is_some_and(Pulse::is_ready) {
            self.pulse = None;
            self.pulse = Some(Pulse::connect("Audio Graph")?);
        }
        Ok(self.pulse.as_mut().unwrap())
    }

    pub fn defaults(&mut self) -> super::plan::Defaults {
        let server = self.pulse().and_then(|p| p.server()).unwrap_or_default();
        super::plan::Defaults {
            render: server.default_sink,
            capture: server.default_source,
        }
    }

    fn write_journal(&self) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(&self.journal).map_err(|e| e.to_string())?;
        write_atomic(&self.journal_path, &json).map_err(|e| tr!("error_journal_write", e = e))
    }

    fn snapshot(&mut self) -> Result<Snapshot, String> {
        let p = self.pulse()?;
        Ok(Snapshot {
            server: p.server()?,
            sinks: p.sinks()?,
            sources: p.sources()?,
            inputs: p.sink_inputs()?,
            outputs: p.source_outputs()?,
        })
    }

    /// Devices changed: what failed may try again without waiting for its retry pause
    pub fn retry_now(&mut self) {
        self.failed.clear();
        self.streams.retry_now();
    }

    pub fn apply(&mut self, plan: &Plan) -> (Vec<StreamStatus>, Vec<AppStatus>) {
        // nothing runs and nothing to restore: leave the server alone
        let idle = plan.virtual_cables.is_empty() && plan.streams.is_empty() && plan.routes.is_empty() && plan.system.is_none();
        if idle && self.clean && self.cables.is_empty() && self.streams.is_empty() && self.journal.is_empty() {
            return (Vec::new(), Vec::new());
        }
        match self.try_apply(plan) {
            Ok(result) => result,
            Err(e) => {
                // the server is unavailable or the connection broke: start over on the next tick
                self.pulse = None;
                let message = tr!("error_audio_server_unavailable", error = e);
                unavailable(plan, &message)
            }
        }
    }

    fn try_apply(&mut self, plan: &Plan) -> Result<(Vec<StreamStatus>, Vec<AppStatus>), String> {
        // a long tick goes to the log split by steps: it shows where the server was waited for
        let started = Instant::now();
        let mut steps: Vec<(&str, Duration)> = Vec::new();
        let mut mark = |name: &'static str, at: &mut Instant| {
            steps.push((name, at.elapsed()));
            *at = Instant::now();
        };
        let mut at = Instant::now();
        self.tidy()?;
        mark("modules", &mut at);
        let mut cable_errors = Vec::new();
        let mut wanted_cables: BTreeMap<&str, &VirtualCable> = BTreeMap::new();
        for cable in &plan.virtual_cables {
            wanted_cables.entry(cable.sink.as_str()).or_insert(cable);
        }
        for cable in wanted_cables.values() {
            match self.ensure_cable(cable) {
                Ok(()) => {}
                Err(message) => cable_errors.push(StreamStatus {
                    sink: cable.sink.clone(),
                    sources: Vec::new(),
                    errors: vec![DeviceError { device: cable.sink.clone(), message }],
                }),
            }
        }
        mark("cables", &mut at);
        let snap = self.snapshot()?;
        self.remember_user_defaults(&snap);
        mark("state", &mut at);

        let mut apps = Vec::new();
        apps.extend(self.apply_system(plan.system.as_ref(), &snap));
        // "default" for apps is the new default device by now
        let snap = Snapshot { server: self.pulse()?.server()?, ..snap };
        mark("default", &mut at);
        apps.extend(self.apply_routes(&plan.routes, &snap));
        mark("apps", &mut at);
        let mut streams = self.apply_links(plan, &snap);
        streams.extend(cable_errors);
        mark("links", &mut at);

        // cables are removed last: apps and links have left them already
        let unwanted: Vec<String> = self.cables.keys().filter(|k| !wanted_cables.contains_key(k.as_str())).cloned().collect();
        for sink in unwanted {
            self.unload_cable(&sink);
        }
        self.guard_defaults(plan.system.as_ref())?;
        mark("cleanup", &mut at);
        if started.elapsed() > Duration::from_secs(1) {
            let detail: Vec<String> = steps.iter().map(|(name, d)| format!("{name} {} ms", d.as_millis())).collect();
            let line = format!("long engine tick ({} ms): {}", started.elapsed().as_millis(), detail.join(", "));
            #[cfg(test)]
            eprintln!("{line}");
            crate::log::write(line);
        }

        // a request error may have been a broken connection
        if !self.pulse()?.is_ready() {
            return Err(tr!("error_audio_server_lost"));
        }
        Ok((streams, apps))
    }

    /// Forgets modules that are gone (the server restarted), and removes our modules this run doesn't
    /// know about; the loopbacks earlier versions made for links are among those
    fn tidy(&mut self) -> Result<(), String> {
        let modules = self.pulse()?.modules()?;
        let present: HashSet<u32> = modules.iter().map(|m| m.index).collect();
        let gone: Vec<String> = self
            .cables
            .iter()
            .filter(|(_, c)| !present.contains(&c.sink_module) || !present.contains(&c.source_module))
            .map(|(k, _)| k.clone())
            .collect();
        for sink in gone {
            self.unload_cable(&sink);
        }

        let known: HashSet<u32> = self.cables.values().flat_map(|c| [c.sink_module, c.source_module]).collect();
        // loopbacks and remap-sources first: they hold on to the null-sink
        let mut strays: Vec<&pulse::Module> = modules
            .iter()
            .filter(|m| m.argument.contains(MARK) && !known.contains(&m.index))
            .collect();
        strays.sort_by_key(|m| m.name == "module-null-sink");
        let strays: Vec<u32> = strays.iter().map(|m| m.index).collect();
        if !strays.is_empty() {
            crate::log::write(format!("removing modules of a previous run: {strays:?}"));
        }
        for index in strays {
            let _ = self.pulse()?.unload_module(index);
        }
        self.clean = true;
        Ok(())
    }

    /// Don't retry a failure more often than RETRY: Ok(()) means it may try
    fn may_retry(&self, key: &str) -> Result<(), String> {
        match self.failed.get(key) {
            Some((at, message)) if at.elapsed() < RETRY => Err(message.clone()),
            _ => Ok(()),
        }
    }

    fn fail(&mut self, key: String, message: String) -> String {
        self.failed.insert(key, (Instant::now(), message.clone()));
        message
    }

    /// Creates a cable's virtual devices if they don't exist yet
    fn ensure_cable(&mut self, cable: &VirtualCable) -> Result<(), String> {
        if let Some(loaded) = self.cables.get(&cable.sink) {
            if loaded.label == cable.label {
                return Ok(());
            }
            // the device is named after the node's label: a changed label means recreating it
            self.unload_cable(&cable.sink);
        }
        let key = format!("cable:{}", cable.sink);
        self.may_retry(&key)?;
        let description = description(&cable.label);
        let sink_args = format!("sink_name={} sink_properties='device.description=\"{description}\"'", cable.sink);
        let source_args = format!(
            "source_name={} master={}.monitor source_properties='device.description=\"{description}\"'",
            cable.source, cable.sink
        );
        let sink_module = match self.pulse()?.load_module("module-null-sink", &sink_args) {
            Ok(index) => index,
            Err(e) => {
                let message = tr!("error_create_cable", name = cable.label, error = e);
                return Err(self.fail(key, message));
            }
        };
        // the cable's output is the monitor of its input: the monitor must exist already
        let monitor = format!("{}.monitor", cable.sink);
        self.wait_for("cable monitor", |p| Ok(p.sources()?.iter().any(|d| d.name == monitor)));
        let source_module = match self.pulse()?.load_module("module-remap-source", &source_args) {
            Ok(index) => index,
            Err(e) => {
                let _ = self.pulse()?.unload_module(sink_module);
                let message = tr!("error_create_cable", name = cable.label, error = e);
                return Err(self.fail(key, message));
            }
        };
        self.wait_for("cable output", |p| Ok(p.sources()?.iter().any(|d| d.name == cable.source)));
        self.failed.remove(&key);
        self.cables.insert(
            cable.sink.clone(),
            LoadedCable {
                label: cable.label.clone(),
                source: cable.source.clone(),
                sink_module,
                source_module,
            },
        );
        Ok(())
    }

    /// Waits for what was created to appear on the server (no longer than APPEAR)
    fn wait_for(&mut self, what: &str, mut ready: impl FnMut(&mut Pulse) -> Result<bool, String>) {
        let started = Instant::now();
        while started.elapsed() < APPEAR {
            match self.pulse().and_then(|p| ready(p)) {
                Ok(true) | Err(_) => return,
                Ok(false) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let line = format!("gave up waiting for: {what}");
        #[cfg(test)]
        eprintln!("{line}");
        crate::log::write(line);
    }

    /// Removes a cable in order: streams on its devices first, then the output, then the input.
    /// Streams could be left to the server (it closes them with the device), but then a link
    /// waits for its retry pause. Waits for the devices to disappear: the server unloads
    /// modules with a delay, and a cable with the same name may be needed right away (a rename)
    fn unload_cable(&mut self, sink: &str) {
        let Some(cable) = self.cables.remove(sink) else { return };
        self.streams.close(sink);
        self.streams.close(&format!("{LOOPBACK_PREFIX}{sink}"));
        self.streams.close(&cable.source);
        let Some(p) = self.pulse.as_mut() else { return };
        let _ = p.unload_module(cable.source_module);
        let _ = p.unload_module(cable.sink_module);
        let (sink, source) = (sink.to_string(), cable.source.clone());
        self.wait_for("cable removal", |p| {
            Ok(!p.sinks()?.iter().any(|d| d.name == sink) && !p.sources()?.iter().any(|d| d.name == source))
        });
    }

    fn remember_user_defaults(&mut self, snap: &Snapshot) {
        for flow in [Flow::Render, Flow::Capture] {
            if let Some(name) = snap.default(flow).filter(|n| !is_virtual(n)) {
                self.user_default.insert(flow, name.to_string());
            }
        }
    }

    fn set_default(&mut self, flow: Flow, name: &str) -> Result<(), String> {
        let p = self.pulse()?;
        match flow {
            Flow::Render => p.set_default_sink(name),
            Flow::Capture => p.set_default_source(name),
        }
    }

    /// Default devices from the "System" node, or the previous ones without it
    fn apply_system(&mut self, system: Option<&SystemRoute>, snap: &Snapshot) -> Option<AppStatus> {
        let wanted = [
            (Flow::Render, system.and_then(|s| s.render.clone())),
            (Flow::Capture, system.and_then(|s| s.capture.clone())),
        ];
        let mut error: Option<String> = None;
        for (flow, want) in wanted {
            match want {
                Some(id) => {
                    let current = snap.default(flow).map(str::to_string);
                    if current.as_deref() == Some(id.as_str()) {
                        continue;
                    }
                    if !self.journal.defaults.iter().any(|d| d.flow == flow) {
                        // the previous one is what the user chose, not a cable the server picked by itself
                        let previous = current.filter(|c| !is_virtual(c)).or_else(|| self.user_default.get(&flow).cloned());
                        self.journal.defaults.push(SavedDefault { flow, previous });
                        if let Err(e) = self.write_journal() {
                            error.get_or_insert(e);
                            continue;
                        }
                    }
                    if let Err(e) = self.set_default(flow, &id) {
                        error.get_or_insert(tr!("error_set_default_device", error = e));
                    }
                }
                None => {
                    let Some(i) = self.journal.defaults.iter().position(|d| d.flow == flow) else { continue };
                    let saved = self.journal.defaults.remove(i);
                    // the previous device may be gone: then the server picks one itself
                    if let Some(previous) = saved.previous.filter(|p| snap.device(flow, p).is_some()) {
                        let _ = self.set_default(flow, &previous);
                    }
                    let _ = self.write_journal();
                }
            }
        }
        system.map(|s| AppStatus {
            node: s.node.clone(),
            state: if error.is_some() { AppState::Failed } else { AppState::Routed },
            error,
        })
    }

    /// Our cables must not become default devices by themselves: PipeWire picks
    /// a new device if the user never chose one explicitly
    fn guard_defaults(&mut self, system: Option<&SystemRoute>) -> Result<(), String> {
        let server = self.pulse()?.server()?;
        for (flow, current, wanted) in [
            (Flow::Render, server.default_sink, system.and_then(|s| s.render.clone())),
            (Flow::Capture, server.default_source, system.and_then(|s| s.capture.clone())),
        ] {
            let Some(current) = current.filter(|c| is_virtual(c)) else { continue };
            if wanted.as_deref() == Some(current.as_str()) {
                continue;
            }
            if let Some(user) = self.user_default.get(&flow).cloned() {
                let _ = self.set_default(flow, &user);
            }
        }
        Ok(())
    }

    fn move_stream(&mut self, flow: Flow, index: u32, device: &str) -> Result<(), String> {
        let p = self.pulse()?;
        match flow {
            Flow::Render => p.move_sink_input(index, device),
            Flow::Capture => p.move_source_output(index, device),
        }
        .map_err(|e| tr!("error_move_app_audio", error = e))
    }

    /// App devices per the plan; apps not in the plan get the previous ones
    fn apply_routes(&mut self, routes: &[AppRoute], snap: &Snapshot) -> Vec<AppStatus> {
        let streams_of = |flow: Flow, exe: &str| -> Vec<Stream> {
            snap.streams(flow)
                .iter()
                .filter(|s| is_app_stream(s) && s.binary.as_deref().is_some_and(|b| b.to_lowercase() == exe))
                .cloned()
                .collect()
        };
        let mut wanted: HashSet<(String, Flow)> = HashSet::new();
        let mut statuses = Vec::new();

        for route in routes {
            let exe = route.app.exe.to_lowercase();
            let targets = [(Flow::Render, &route.render), (Flow::Capture, &route.capture)];
            let mut relevant = 0;
            for (flow, target) in &targets {
                if target.is_some() {
                    wanted.insert((exe.clone(), *flow));
                    relevant += streams_of(*flow, &exe).len();
                }
            }
            if relevant == 0 {
                // the device is assigned when the app opens audio
                statuses.push(AppStatus {
                    node: route.node.clone(),
                    state: if is_running(&exe) { AppState::NoAudio } else { AppState::NotRunning },
                    error: None,
                });
                continue;
            }
            let mut error = None;
            for (flow, target) in targets {
                let Some(target) = target else { continue };
                let name = match target {
                    Target::Default => snap.default(flow).map(str::to_string),
                    Target::Device(id) => Some(id.clone()),
                };
                let Some(device) = name.as_deref().and_then(|n| snap.device(flow, n)) else {
                    error.get_or_insert(tr!("error_device_not_found"));
                    continue;
                };
                let (target_index, target_name) = (device.index, device.name.clone());
                let mut move_error = None;
                for stream in streams_of(flow, &exe).iter().filter(|s| s.device != target_index) {
                    if !self.journal.saved.iter().any(|s| s.exe == exe && s.flow == flow) {
                        // it followed the default device, so it returns to it
                        let current = snap.name_of(flow, stream.device);
                        let previous = current.filter(|c| Some(*c) != snap.default(flow) && !is_virtual(c)).map(str::to_string);
                        self.journal.saved.push(Saved { exe: exe.clone(), flow, previous });
                        if let Err(e) = self.write_journal() {
                            error.get_or_insert(e);
                            continue;
                        }
                    }
                    if let Err(e) = self.move_stream(flow, stream.index, &target_name) {
                        move_error.get_or_insert(e);
                    }
                }
                // a device PipeWire has just created doesn't accept streams yet: the next tick
                // retries, and it becomes an error if it keeps failing longer than RETRY
                let key = (exe.clone(), flow);
                match move_error {
                    None => {
                        self.move_failing.remove(&key);
                    }
                    Some(e) => {
                        if self.move_failing.entry(key).or_insert_with(Instant::now).elapsed() >= RETRY {
                            error.get_or_insert(e);
                        }
                    }
                }
            }
            statuses.push(AppStatus {
                node: route.node.clone(),
                state: if error.is_some() { AppState::Failed } else { AppState::Routed },
                error,
            });
        }

        // restoring previous devices
        let mut kept = Vec::new();
        let mut changed = false;
        for saved in std::mem::take(&mut self.journal.saved) {
            if wanted.contains(&(saved.exe.clone(), saved.flow)) {
                kept.push(saved);
                continue;
            }
            let streams = streams_of(saved.flow, &saved.exe);
            if streams.is_empty() {
                // the app is silent: restore when it opens audio
                kept.push(saved);
                continue;
            }
            let target = saved
                .previous
                .as_deref()
                .filter(|p| snap.device(saved.flow, p).is_some())
                .or(snap.default(saved.flow))
                .and_then(|n| snap.device(saved.flow, n))
                .map(|d| (d.index, d.name.clone()));
            if let Some((index, name)) = target {
                for stream in streams.iter().filter(|s| s.device != index) {
                    let _ = self.move_stream(saved.flow, stream.index, &name);
                }
            }
            changed = true;
        }
        self.journal.saved = kept;

        // earlier versions changed app volumes while a graph ran: what their journal holds goes back
        let mut kept = Vec::new();
        for saved in std::mem::take(&mut self.journal.volumes) {
            let streams = streams_of(Flow::Render, &saved.exe);
            if streams.is_empty() {
                kept.push(saved);
                continue;
            }
            for s in streams.iter().filter(|s| s.volume_writable) {
                let _ = self.pulse().and_then(|p| p.set_sink_input_volume(s.index, &pulse::mixer(s.volume.len(), saved.previous)));
            }
            changed = true;
        }
        self.journal.volumes = kept;
        if changed {
            let _ = self.write_journal();
        }
        statuses
    }

    /// The engine's links per the plan. A stream to a device that isn't there couldn't start:
    /// its link waits, with a clear error right away
    fn apply_links(&mut self, plan: &Plan, snap: &Snapshot) -> Vec<StreamStatus> {
        let present: Vec<PlanStream> = plan
            .streams
            .iter()
            .map(|stream| {
                let mut stream = stream.clone();
                let sink = snap.device(Flow::Render, &stream.sink).is_some();
                stream.sources.retain(|s| {
                    let (flow, name) = source_device(&s.device);
                    sink && snap.device(flow, name).is_some()
                });
                stream
            })
            .collect();
        let statuses = self.streams.apply(&present);
        plan.streams
            .iter()
            .zip(statuses)
            .map(|(stream, status)| {
                let devices = stream.sources.iter().map(|s| source_device(&s.device)).chain([(Flow::Render, stream.sink.as_str())]);
                let mut missing: Vec<&str> = devices.filter(|(flow, name)| snap.device(*flow, name).is_none()).map(|(_, name)| name).collect();
                // the layers of one source
                missing.dedup();
                let missing = missing.into_iter().map(|device| DeviceError { device: device.to_string(), message: tr!("error_device_not_found") });
                StreamStatus {
                    sink: stream.sink.clone(),
                    sources: stream.sources.iter().map(|s| s.device.clone()).collect(),
                    errors: missing.chain(status.errors).collect(),
                }
            })
            .collect()
    }

    /// App exit: the plan is already empty and applied, only the server is left to release
    pub fn shutdown(&mut self) {
        self.streams.shutdown();
        let sinks: Vec<String> = self.cables.keys().cloned().collect();
        for sink in sinks {
            self.unload_cable(&sink);
        }
    }
}

/// The device a link's source depends on: a loopback's is the output device it listens to
fn source_device(source: &str) -> (Flow, &str) {
    match source.strip_prefix(LOOPBACK_PREFIX) {
        Some(sink) => (Flow::Render, sink),
        None => (Flow::Capture, source),
    }
}

/// The server is unavailable: everything the graph should do doesn't work
fn unavailable(plan: &Plan, message: &str) -> (Vec<StreamStatus>, Vec<AppStatus>) {
    let streams = plan
        .streams
        .iter()
        .map(|s| StreamStatus {
            sink: s.sink.clone(),
            sources: s.sources.iter().map(|x| x.device.clone()).collect(),
            errors: vec![DeviceError { device: s.sink.clone(), message: message.to_string() }],
        })
        .collect();
    let apps = plan
        .routes
        .iter()
        .map(|r| r.node.clone())
        .chain(plan.system.iter().map(|s| s.node.clone()))
        .map(|node| AppStatus { node, state: AppState::Failed, error: Some(message.to_string()) })
        .collect();
    (streams, apps)
}

/// Tests on a real audio server (PulseAudio or PipeWire from PULSE_SERVER).
/// They create their own test devices agtest.* and remove them; they need pactl, pacat and parec:
/// cargo test engine::linux -- --ignored --nocapture --test-threads=1
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::plan::Gain;
    use crate::graphs::AppRef;
    use std::io::{Read, Write};
    use std::process::{Child, Command, Stdio};

    fn pactl(args: &[&str]) -> String {
        let out = Command::new("pactl").args(args).output().expect("pactl");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Test devices: output agtest.out and input agtest.mic.src (played into through agtest.mic)
    struct Devices(Vec<String>);

    impl Devices {
        fn new() -> Self {
            let load = |args: &[&str]| pactl(&[&["load-module"], args].concat());
            let modules = vec![
                load(&["module-null-sink", "sink_name=agtest.out"]),
                load(&["module-null-sink", "sink_name=agtest.mic"]),
                load(&["module-remap-source", "source_name=agtest.mic.src", "master=agtest.mic.monitor"]),
            ];
            assert!(modules.iter().all(|m| m.parse::<u32>().is_ok()), "test devices: {modules:?}");
            std::thread::sleep(Duration::from_millis(300));
            // the server remembers a device's volume by its name, also from an earlier test
            pactl(&["set-sink-volume", "agtest.out", "100%"]);
            pactl(&["set-sink-volume", "agtest.mic", "100%"]);
            pactl(&["set-source-volume", "agtest.mic.src", "100%"]);
            Self(modules)
        }
    }

    impl Drop for Devices {
        fn drop(&mut self) {
            for m in self.0.iter().rev() {
                pactl(&["unload-module", m]);
            }
        }
    }

    fn backend() -> (Backend, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ag-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (Backend::new(&dir, Default::default()), dir)
    }

    /// A few engine ticks: PipeWire finishes what it creates after answering
    fn ticks(backend: &mut Backend, plan: &Plan) -> (Vec<StreamStatus>, Vec<AppStatus>) {
        let mut result = backend.apply(plan);
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(250));
            result = backend.apply(plan);
        }
        result
    }

    fn our_modules() -> Vec<String> {
        pactl(&["list", "short", "modules"]).lines().filter(|l| l.contains(MARK)).map(str::to_string).collect()
    }

    /// The average level (RMS) of what sounds on the input device while play_into plays
    /// a sine with amplitude 0.5 into the output device
    fn tone_rms(play_into: &str, record_from: &str) -> f32 {
        let [left, right] = tone_rms_channels(play_into, record_from);
        ((left * left + right * right) / 2.0).sqrt()
    }

    /// The same per channel: left and right
    fn tone_rms_channels(play_into: &str, record_from: &str) -> [f32; 2] {
        let mut rec = Command::new("parec")
            .args(["--device", record_from, "--raw", "--format=s16le", "--rate=48000", "--channels=2", "--latency-msec=20"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("parec");
        let mut play = Command::new("pacat")
            .args(["--device", play_into, "--raw", "--format=s16le", "--rate=48000", "--channels=2"])
            .stdin(Stdio::piped())
            .spawn()
            .expect("pacat");
        let mut tone = Vec::new();
        for i in 0..48_000 * 2 {
            let v = ((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5 * 32767.0) as i16;
            tone.extend_from_slice(&v.to_le_bytes());
            tone.extend_from_slice(&v.to_le_bytes());
        }
        let mut stdin = play.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&tone);
        });
        // one and a half seconds of recording, measured over the last half: the start is the link's latency
        let mut buf = vec![0u8; 48_000 * 4 * 3 / 2];
        rec.stdout.as_mut().unwrap().read_exact(&mut buf).unwrap();
        let _ = rec.kill();
        let _ = writer.join();
        let _ = play.wait();
        let samples: Vec<f32> = buf[buf.len() / 2..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let channel = |ch: usize| {
            let values: Vec<f32> = samples.iter().skip(ch).step_by(2).copied().collect();
            (values.iter().map(|s| s * s).sum::<f32>() / values.len() as f32).sqrt()
        };
        [channel(0), channel(1)]
    }

    fn cable() -> VirtualCable {
        VirtualCable {
            node: "t1".into(),
            label: "Test mix".into(),
            sink: "audiograph.t1".into(),
            source: "audiograph.t1.out".into(),
        }
    }

    #[test]
    #[ignore]
    fn cable_links_and_gain() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        // microphone → cable → output; the cable's 50% volume applies to the sound taken from it
        let plan = Plan {
            virtual_cables: vec![cable()],
            streams: vec![
                PlanStream::through("audiograph.t1", false, &[("agtest.mic.src", [1.0; 2], 0.0)]),
                PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [0.5; 2], 0.0)]),
            ],
            ..Plan::default()
        };
        let (streams, _) = ticks(&mut backend, &plan);
        println!("streams: {streams:?}");
        assert!(streams.iter().all(|s| s.errors.is_empty()), "{streams:?}");
        let sinks = pactl(&["list", "short", "sinks"]);
        assert!(sinks.contains("audiograph.t1"), "{sinks}");
        let description = pactl(&["list", "sinks"]);
        assert!(description.contains("Test mix (Audio Graph)"));
        // the second tick changes nothing
        let before = our_modules();
        backend.apply(&plan);
        assert_eq!(our_modules(), before);
        println!("our modules: {before:#?}");

        let rms = tone_rms("agtest.mic", "agtest.out.monitor");
        println!("output RMS: {rms:.3} (0.354 without volume, 0.177 at 50%)");
        assert!((rms - 0.177).abs() < 0.03, "RMS {rms}");

        // volume on the fly
        let louder = Plan {
            streams: vec![
                plan.streams[0].clone(),
                PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [1.0; 2], 0.0)]),
            ],
            ..plan.clone()
        };
        ticks(&mut backend, &louder);
        let rms = tone_rms("agtest.mic", "agtest.out.monitor");
        println!("RMS at 100%: {rms:.3}");
        assert!((rms - 0.354).abs() < 0.04, "RMS {rms}");

        // balance to the left: the right channel a quarter as loud
        let balanced = Plan {
            streams: vec![
                plan.streams[0].clone(),
                PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [1.0, 0.25], 0.0)]),
            ],
            ..plan.clone()
        };
        ticks(&mut backend, &balanced);
        let [left, right] = tone_rms_channels("agtest.mic", "agtest.out.monitor");
        println!("RMS with the balance to the left: {left:.3} {right:.3}");
        assert!((left - 0.354).abs() < 0.04 && (right / left - 0.25).abs() < 0.03, "RMS {left} {right}");

        // the engine's streams stay at 100%: the volume is in the sound they carry
        let list = pactl(&["list", "sink-inputs"]);
        let ours = list.split("Sink Input #").filter(|b| b.contains("audiograph.link = \"1\""));
        let volumes: Vec<&str> = ours.flat_map(|b| b.lines().find(|l| l.trim().starts_with("Volume:"))).collect();
        assert!(!volumes.is_empty() && volumes.iter().all(|v| v.contains("100%") && !v.contains(" 50%")), "{volumes:?}");

        // stop: no cable, no links
        ticks(&mut backend, &Plan::default());
        assert!(our_modules().is_empty(), "{:?}", our_modules());
        assert!(!pactl(&["list", "short", "sinks"]).contains("audiograph.t1"));
        assert_eq!(link_streams(), 0);
    }

    /// How many of the engine's playback and recording streams the server has
    fn link_streams() -> usize {
        let streams = pactl(&["list", "sink-inputs"]) + &pactl(&["list", "source-outputs"]);
        streams.matches("audiograph.link = \"1\"").count()
    }

    #[test]
    #[ignore]
    fn layers_of_one_source_share_the_device_streams() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let plan = |sources: &[(Gain, f32)]| {
            let sources: Vec<(&str, Gain, f32)> = sources.iter().map(|&(gain, delay)| ("agtest.mic.src", gain, delay)).collect();
            Plan { streams: vec![PlanStream::through("agtest.out", true, &sources)], ..Plan::default() }
        };
        ticks(&mut backend, &plan(&[([1.0, 0.0], 0.0), ([0.0, 1.0], 0.2)]));
        // one recording and one playback stream, whatever the links between them
        assert_eq!(link_streams(), 2);
        let [left, right] = tone_rms_channels("agtest.mic", "agtest.out.monitor");
        assert!(left > 0.2 && right > 0.2, "RMS {left} {right}");
        ticks(&mut backend, &plan(&[([1.0, 1.0], 0.0)]));
        assert_eq!(link_streams(), 2);
        ticks(&mut backend, &Plan::default());
        assert_eq!(link_streams(), 0);
        assert!(our_modules().is_empty(), "{:?}", our_modules());
    }

    /// Plays a tone into the first device and records the second one for this long: the samples of
    /// the left channel. Recorded with the engine's own stream: parec leaves out the silence
    /// PulseAudio sends as holes, and the time of what it records would be lost
    fn tone_through(play_into: &str, record_from: &str, seconds: f32) -> Vec<f32> {
        let frames = (seconds * 48_000.0) as usize;
        let device = record_from.to_string();
        let recorder = std::thread::spawn(move || {
            let mut stream = pulse::AudioStream::record(&device, 48_000, 2, Duration::from_millis(10)).unwrap();
            let mut left = Vec::with_capacity(frames);
            while left.len() < frames {
                stream.wait(Duration::from_millis(100)).unwrap();
                stream.read(|samples| left.extend(samples.iter().step_by(2))).unwrap();
            }
            left.truncate(frames);
            left
        });
        std::thread::sleep(Duration::from_millis(300));
        let mut play = Command::new("pacat")
            .args(["--device", play_into, "--raw", "--format=float32le", "--rate=48000", "--channels=2", "--latency-msec=20"])
            .stdin(Stdio::piped())
            .spawn()
            .expect("pacat");
        let mut tone = Vec::with_capacity(frames * 8);
        for i in 0..frames {
            let v = ((i as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 0.5) as f32;
            tone.extend_from_slice(&v.to_le_bytes());
            tone.extend_from_slice(&v.to_le_bytes());
        }
        let mut stdin = play.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&tone);
        });
        let left = recorder.join().unwrap();
        let _ = play.kill();
        let _ = play.wait();
        let _ = writer.join();
        left
    }

    /// PulseAudio's null-sink plays 2 s of silence ahead when its first stream comes:
    /// for that long it takes nothing from the stream, and the sound of a link is dropped
    fn settle() {
        std::thread::sleep(Duration::from_millis(2100));
    }

    fn onset(samples: &[f32]) -> Option<f32> {
        samples.iter().position(|s| s.abs() > 0.05).map(|i| i as f32 / 48_000.0)
    }

    /// The delay in the link's buffer, also past what a server loopback could hold (about 2 s)
    #[test]
    #[ignore]
    fn delay_is_held_in_the_link() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let plan = |delay| Plan {
            streams: vec![PlanStream::through("agtest.out", true, &[("agtest.mic.src", [1.0; 2], delay)])],
            ..Plan::default()
        };
        ticks(&mut backend, &plan(0.0));
        settle();
        let plain = onset(&tone_through("agtest.mic", "agtest.out.monitor", 1.5)).expect("no sound");
        println!("without a delay the sound comes after {plain:.3} s");
        for delay in [0.25, 5.0] {
            ticks(&mut backend, &plan(delay));
            // the link's reserve gathers from the moment the delay was set: what is playing now is silence
            std::thread::sleep(Duration::from_secs_f32(delay + 0.3));
            let heard = onset(&tone_through("agtest.mic", "agtest.out.monitor", delay + 1.5)).expect("no sound");
            println!("delay {delay}: the sound comes after {heard:.3} s, {:.3} s more", heard - plain);
            assert!((heard - plain - delay).abs() < 0.06, "{heard} {plain}");
        }
        ticks(&mut backend, &Plan::default());
    }

    /// A sine through a cable for a while: a click in it stands out of what the sine's own curve predicts.
    /// AG_LOAD=n adds n busy threads; AG_SECONDS sets the length
    #[test]
    #[ignore]
    fn sound_is_continuous() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let plan = Plan {
            virtual_cables: vec![cable()],
            streams: vec![
                PlanStream::through("audiograph.t1", false, &[("agtest.mic.src", [1.0; 2], 0.0)]),
                PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [1.0; 2], 0.0)]),
            ],
            ..Plan::default()
        };
        ticks(&mut backend, &plan);
        settle();
        let number = |name: &str, default: f32| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let load: Vec<_> = (0..number("AG_LOAD", 0.0) as usize)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut x = 1u64;
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    x
                })
            })
            .collect();
        let seconds = number("AG_SECONDS", 20.0);
        let samples = tone_through("agtest.mic", "agtest.out.monitor", seconds);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for thread in load {
            let _ = thread.join();
        }
        let start = samples.iter().position(|s| s.abs() > 0.05).expect("no sound") + 4800;
        // the tone's end may fall inside the recording
        let sound = &samples[start..samples.len() - 48_000];
        let k = 2.0 * (440.0 * std::f32::consts::TAU / 48_000.0).cos();
        let clicks: Vec<usize> = sound.windows(3).enumerate().filter(|(_, w)| (w[2] - k * w[1] + w[0]).abs() > 0.01).map(|(i, _)| i).collect();
        // a run of clicks in one place is one event
        let mut events: Vec<usize> = Vec::new();
        for &i in &clicks {
            if events.last().is_none_or(|last| i - last > 480) {
                events.push(i);
            }
        }
        let silence = sound.windows(4800).step_by(480).filter(|w| w.iter().all(|s| s.abs() < 0.01)).count();
        println!("{:.1} s of sound: {} clicks, first at {:?} s, {silence} silent windows", sound.len() as f32 / 48_000.0, events.len(), events.iter().take(8).map(|i| *i as f32 / 48_000.0).collect::<Vec<_>>());
        ticks(&mut backend, &Plan::default());
        assert!(events.len() <= (seconds / 10.0) as usize + 1, "{} clicks", events.len());
    }

    /// A link's device goes away and comes back under a running graph: the sound doesn't go anywhere
    /// else meanwhile, and the link returns by itself
    #[test]
    #[ignore]
    fn link_follows_its_device() {
        let _out = Devices::new();
        let (mut backend, _) = backend();
        let plan = Plan {
            streams: vec![PlanStream::through("agtest.gone", true, &[("agtest.mic.src", [1.0; 2], 0.0)])],
            ..Plan::default()
        };
        // not there yet: a clear error, no stream
        let (streams, _) = ticks(&mut backend, &plan);
        assert_eq!(streams[0].errors.len(), 1, "{streams:?}");
        assert_eq!(link_streams(), 0);
        let module = pactl(&["load-module", "module-null-sink", "sink_name=agtest.gone"]);
        std::thread::sleep(Duration::from_millis(300));
        let (streams, _) = ticks(&mut backend, &plan);
        assert!(streams[0].errors.is_empty(), "{streams:?}");
        assert_eq!(link_streams(), 2);
        settle();
        let rms = tone_rms("agtest.mic", "agtest.gone.monitor");
        assert!((rms - 0.354).abs() < 0.04, "RMS {rms}");

        pactl(&["unload-module", &module]);
        std::thread::sleep(Duration::from_millis(500));
        let (streams, _) = ticks(&mut backend, &plan);
        println!("gone: {streams:?}");
        assert_eq!(streams[0].errors.len(), 1, "{streams:?}");
        // only the recording stays, and it plays nowhere
        assert!(!pactl(&["list", "sink-inputs"]).contains("audiograph.link = \"1\""));

        let module = pactl(&["load-module", "module-null-sink", "sink_name=agtest.gone"]);
        std::thread::sleep(Duration::from_millis(300));
        backend.retry_now();
        let (streams, _) = ticks(&mut backend, &plan);
        assert!(streams[0].errors.is_empty(), "{streams:?}");
        settle();
        let rms = tone_rms("agtest.mic", "agtest.gone.monitor");
        assert!((rms - 0.354).abs() < 0.04, "RMS {rms}");
        ticks(&mut backend, &Plan::default());
        assert_eq!(link_streams(), 0);
        pactl(&["unload-module", &module]);
    }

    /// What plays on a device reaches another one through its monitor, though no link plays it there.
    /// A missing device is reported under the output device's own name, as the interface looks for it
    #[test]
    #[ignore]
    fn output_passes_its_device_on() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let stream = |device: &str| PlanStream::through("agtest.out", true, &[(&format!("{LOOPBACK_PREFIX}{device}"), [0.5; 2], 0.0)]);
        let plan = Plan { streams: vec![stream("agtest.mic")], ..Plan::default() };
        let (streams, _) = ticks(&mut backend, &plan);
        assert!(streams[0].errors.is_empty(), "{streams:?}");
        assert_eq!(link_streams(), 2);
        settle();
        let rms = tone_rms("agtest.mic", "agtest.out.monitor");
        println!("output RMS: {rms:.3} (0.177 at 50%)");
        assert!((rms - 0.177).abs() < 0.03, "RMS {rms}");

        let plan = Plan { streams: vec![stream("agtest.gone")], ..Plan::default() };
        let (streams, _) = ticks(&mut backend, &plan);
        assert_eq!(streams[0].errors.len(), 1, "{streams:?}");
        assert_eq!(streams[0].errors[0].device, "agtest.gone");
        assert_eq!(link_streams(), 0);
    }

    /// Nothing plays on the device, yet its monitor keeps giving sound: the pauses stay in a link
    #[test]
    #[ignore]
    fn silent_monitor_keeps_time() {
        use crate::engine::link::{CHANNELS, RATE};
        use crate::engine::ring::ring;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Mutex;
        let _devices = Devices::new();
        settle();
        let (producer, reader) = ring(RATE * CHANNELS * 8);
        let links = Mutex::new(vec![((String::new(), 0), producer)]);
        let (stop, working) = (AtomicBool::new(false), AtomicBool::new(false));
        let device = format!("{LOOPBACK_PREFIX}agtest.out");
        let frames = std::thread::scope(|s| {
            let thread = s.spawn(|| super::streams::capture_thread(&device, &links, &stop, &working));
            // PulseAudio's null-sink starts up to 2 s late, then it runs evenly
            std::thread::sleep(Duration::from_secs(3));
            let start = reader.len();
            std::thread::sleep(Duration::from_secs(2));
            let frames = (reader.len() - start) / CHANNELS;
            stop.store(true, Ordering::Relaxed);
            thread.join().unwrap().unwrap();
            frames
        });
        println!("{frames} frames in 2 s");
        assert!(frames > RATE * 3 / 2, "{frames}");
    }

    /// An app and the default microphone can listen to an output's monitor
    #[test]
    #[ignore]
    fn monitor_is_a_recording_device_for_apps() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let _recorder = spawn("parec", &["--device", "agtest.mic.src", "--raw", "--format=s16le", "--rate=48000", "--channels=2"]);
        std::thread::sleep(Duration::from_millis(700));
        let before = pactl(&["get-default-source"]);
        let plan = Plan {
            routes: vec![AppRoute {
                node: "app".into(),
                app: AppRef { exe: "pacat".into(), path: String::new(), name: "pacat".into() },
                render: None,
                capture: Some(Target::Device("agtest.out.monitor".into())),
            }],
            system: Some(SystemRoute { node: "sys".into(), render: None, capture: Some("agtest.out.monitor".into()) }),
            ..Plan::default()
        };
        let (_, apps) = ticks(&mut backend, &plan);
        println!("during: {:?} {apps:?}, default {}", parec_source(), pactl(&["get-default-source"]));
        assert!(apps.iter().all(|a| a.error.is_none()), "{apps:?}");
        assert_eq!(parec_source().as_deref(), Some("agtest.out.monitor"));
        assert_eq!(pactl(&["get-default-source"]), "agtest.out.monitor");
        ticks(&mut backend, &Plan::default());
        assert_eq!(parec_source().as_deref(), Some("agtest.mic.src"));
        assert_eq!(pactl(&["get-default-source"]), before);
    }

    /// A noise gate on a real server: the tone's peak is 0.5, about -6 dB
    #[test]
    #[ignore]
    fn a_gate_mutes_what_is_quieter() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let plan = |gate| {
            let mut stream = PlanStream::through("agtest.out", true, &[("agtest.mic.src", [1.0; 2], 0.0)]);
            stream.stages[1].gate = gate;
            Plan { streams: vec![stream], ..Plan::default() }
        };
        ticks(&mut backend, &plan(-20.0));
        settle();
        let rms = tone_rms("agtest.mic", "agtest.out.monitor");
        println!("gate -20 dB: {rms:.3}");
        assert!((rms - 0.354).abs() < 0.04, "RMS {rms}");
        ticks(&mut backend, &plan(-3.0));
        std::thread::sleep(Duration::from_millis(500));
        let rms = tone_rms("agtest.mic", "agtest.out.monitor");
        println!("gate -3 dB: {rms:.3}");
        assert!(rms < 0.001, "RMS {rms}");
    }

    /// The sliders of nodes are the server's: what is set is what pactl sees, and the other way round.
    /// The default device is found, what isn't there gives nothing
    #[test]
    #[ignore]
    fn system_sliders_are_the_servers() {
        use crate::system_volume::{system_volumes, write_volume, Target};
        let _devices = Devices::new();
        let sink = Target::Device { flow: crate::devices::Flow::Render, id: Some("agtest.out".into()) };
        let source = Target::Device { flow: crate::devices::Flow::Capture, id: Some("agtest.mic.src".into()) };
        let near = |value: Option<f32>, level: f32| value.is_some_and(|v| (v - level).abs() < 0.01);

        write_volume(&sink, 0.5).unwrap();
        write_volume(&source, 0.25).unwrap();
        assert!(pactl(&["get-sink-volume", "agtest.out"]).contains("50%"));
        assert!(pactl(&["get-source-volume", "agtest.mic.src"]).contains("25%"));
        let read = system_volumes(vec![sink.clone(), source.clone()]);
        assert!(near(read[0], 0.5) && near(read[1], 0.25), "{read:?}");

        pactl(&["set-sink-volume", "agtest.out", "75%"]);
        assert!(near(system_volumes(vec![sink.clone()])[0], 0.75));

        // the balance stays
        pactl(&["set-sink-volume", "agtest.out", "100%", "50%"]);
        write_volume(&sink, 0.5).unwrap();
        let shown = pactl(&["get-sink-volume", "agtest.out"]);
        assert!(shown.contains("50%") && shown.contains("25%"), "{shown}");

        pactl(&["set-default-sink", "agtest.out"]);
        let default = Target::Device { flow: crate::devices::Flow::Render, id: None };
        assert!(near(system_volumes(vec![default])[0], 0.5));

        let gone = Target::Device { flow: crate::devices::Flow::Render, id: Some("agtest.gone".into()) };
        assert_eq!(system_volumes(vec![gone, Target::App { exe: "pacat".into() }]), vec![None, None]);

        let _player = spawn("pacat", &["--device", "agtest.mic", "--raw", "--format=s16le", "--rate=48000", "--channels=2", "/dev/zero"]);
        std::thread::sleep(Duration::from_millis(700));
        let app = Target::App { exe: "pacat".into() };
        assert!(near(system_volumes(vec![app.clone()])[0], 1.0));
        write_volume(&app, 0.5).unwrap();
        assert!(near(system_volumes(vec![app])[0], 0.5));
        assert_eq!(pacat_stream().map(|s| s.1).as_deref(), Some("50%"));
    }

    /// What a running graph sets goes back when it stops; a stopped graph sets nothing
    #[test]
    #[ignore]
    fn a_graph_sets_a_slider_while_it_runs() {
        use crate::graphs::{Graph, GraphNode};
        let _devices = Devices::new();
        use crate::system_volume::VolumeMemory;
        let (_, dir) = backend();
        pactl(&["set-sink-volume", "agtest.out", "80%"]);
        let node: GraphNode = serde_json::from_value(serde_json::json!({
            "id": "o", "kind": "output", "label": "o", "position": {"x": 0, "y": 0},
            "sink": {"type": "device", "id": "agtest.out", "name": "o"}, "systemVolume": 0.4
        }))
        .unwrap();
        let graph: Graph = serde_json::from_value(serde_json::json!({"id": "g", "name": "g", "createdAt": 0, "updatedAt": 0, "nodes": [node]})).unwrap();
        let memory = VolumeMemory::new(&dir);
        memory.follow(&[]);
        assert!(pactl(&["get-sink-volume", "agtest.out"]).contains("80%"));
        memory.follow(&[&graph]);
        assert!(pactl(&["get-sink-volume", "agtest.out"]).contains("40%"));
        // the slider moves on its own while the graph runs, and the graph lets it
        pactl(&["set-sink-volume", "agtest.out", "60%"]);
        memory.follow(&[&graph]);
        assert!(pactl(&["get-sink-volume", "agtest.out"]).contains("60%"));
        memory.follow(&[]);
        assert!(pactl(&["get-sink-volume", "agtest.out"]).contains("80%"));
    }

    /// A process that is closed with the test, even if the test fails
    struct Spawned(Child);

    impl Drop for Spawned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// pacat or parec; their output must not hold the tests' output
    fn spawn(program: &str, args: &[&str]) -> Spawned {
        Spawned(
            Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect(program),
        )
    }

    fn play_silence() -> Spawned {
        spawn("pacat", &["--raw", "--format=s16le", "--rate=48000", "--channels=2", "/dev/zero"])
    }

    /// The device and volume of the pacat stream: (device name, volume in percent)
    fn pacat_stream() -> Option<(String, String)> {
        let sinks = pactl(&["list", "short", "sinks"]);
        let list = pactl(&["list", "sink-inputs"]);
        let block = list.split("Sink Input #").find(|b| b.contains("application.process.binary = \"pacat\""))?;
        let sink: u32 = block.lines().find_map(|l| l.trim().strip_prefix("Sink: "))?.parse().ok()?;
        let name = sinks.lines().find(|l| l.starts_with(&format!("{sink}\t")))?.split('\t').nth(1)?.to_string();
        let volume = block.lines().find(|l| l.trim().starts_with("Volume:"))?.split('/').nth(1)?.trim().to_string();
        Some((name, volume))
    }

    #[test]
    #[ignore]
    fn app_routing_and_restore() {
        let _devices = Devices::new();
        let (mut backend, dir) = backend();
        let _player = play_silence();
        std::thread::sleep(Duration::from_millis(700));
        let before = pacat_stream();
        println!("before: {before:?}");

        let plan = Plan {
            routes: vec![AppRoute {
                node: "app".into(),
                app: AppRef { exe: "pacat".into(), path: String::new(), name: "pacat".into() },
                render: Some(Target::Device("agtest.out".into())),
                capture: None,
            }],
            ..Plan::default()
        };
        let (_, apps) = ticks(&mut backend, &plan);
        println!("status: {apps:?}");
        assert_eq!(apps[0].state, AppState::Routed, "{apps:?}");
        let during = pacat_stream();
        println!("during: {during:?}, journal: {}", fs::read_to_string(dir.join("pulse-routing.json")).unwrap_or_default());
        assert_eq!(during, Some(("agtest.out".to_string(), "100%".to_string())));

        ticks(&mut backend, &Plan::default());
        let after = pacat_stream();
        println!("after: {after:?}");
        assert_eq!(after, before);
        assert!(backend.journal.is_empty());
    }

    /// The recording device of pacat's recording stream (parec is the same file)
    fn parec_source() -> Option<String> {
        let sources = pactl(&["list", "short", "sources"]);
        let list = pactl(&["list", "source-outputs"]);
        let block = list.split("Source Output #").find(|b| b.contains("application.process.binary = \"pacat\""))?;
        let source: u32 = block.lines().find_map(|l| l.trim().strip_prefix("Source: "))?.parse().ok()?;
        Some(sources.lines().find(|l| l.starts_with(&format!("{source}\t")))?.split('\t').nth(1)?.to_string())
    }

    #[test]
    #[ignore]
    fn app_with_chosen_devices_returns_to_them() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        // the app chose its devices itself: output agtest.mic, input agtest.out.monitor
        let _player = spawn("pacat", &["--device", "agtest.mic", "--raw", "--format=s16le", "--rate=48000", "--channels=2", "/dev/zero"]);
        let _recorder = spawn("parec", &["--device", "agtest.out.monitor", "--raw", "--format=s16le", "--rate=48000", "--channels=2"]);
        std::thread::sleep(Duration::from_millis(700));
        println!("before: {:?} / {:?}", pacat_stream(), parec_source());
        assert_eq!(pacat_stream().map(|s| s.0).as_deref(), Some("agtest.mic"));

        let plan = Plan {
            routes: vec![AppRoute {
                node: "app".into(),
                app: AppRef { exe: "pacat".into(), path: String::new(), name: "pacat".into() },
                render: Some(Target::Device("agtest.out".into())),
                capture: Some(Target::Device("agtest.mic.src".into())),
            }],
            ..Plan::default()
        };
        let (_, apps) = ticks(&mut backend, &plan);
        println!("during: {:?} / {:?}, {apps:?}", pacat_stream(), parec_source());
        assert_eq!(pacat_stream().map(|s| s.0).as_deref(), Some("agtest.out"));
        assert_eq!(parec_source().as_deref(), Some("agtest.mic.src"));

        ticks(&mut backend, &Plan::default());
        println!("after: {:?} / {:?}", pacat_stream(), parec_source());
        assert_eq!(pacat_stream().map(|s| s.0).as_deref(), Some("agtest.mic"));
        assert_eq!(parec_source().as_deref(), Some("agtest.out.monitor"));
        assert!(backend.journal.is_empty());
    }

    #[test]
    #[ignore]
    fn system_node_defaults_and_restore() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let (sink, source) = (pactl(&["get-default-sink"]), pactl(&["get-default-source"]));
        let plan = Plan {
            virtual_cables: vec![cable()],
            system: Some(SystemRoute {
                node: "system".into(),
                render: Some("audiograph.t1".into()),
                capture: Some("audiograph.t1.out".into()),
            }),
            ..Plan::default()
        };
        let (_, apps) = ticks(&mut backend, &plan);
        println!("status: {apps:?}");
        assert_eq!(apps[0].state, AppState::Routed);
        assert_eq!(pactl(&["get-default-sink"]), "audiograph.t1");
        assert_eq!(pactl(&["get-default-source"]), "audiograph.t1.out");

        ticks(&mut backend, &Plan::default());
        assert_eq!((pactl(&["get-default-sink"]), pactl(&["get-default-source"])), (sink, source));
        assert!(our_modules().is_empty());
    }

    #[test]
    #[ignore]
    fn renamed_cable_is_recreated_at_once() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let plan = |label: &str| Plan {
            virtual_cables: vec![VirtualCable { label: label.into(), ..cable() }],
            streams: vec![PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [1.0; 2], 0.0)])],
            ..Plan::default()
        };
        ticks(&mut backend, &plan("First"));
        for label in ["Second", "Third"] {
            // there must be no errors already in the first tick after a rename
            let (streams, _) = backend.apply(&plan(label));
            println!("{label}: {streams:?}");
            assert!(streams.iter().all(|s| s.errors.is_empty()), "{streams:?}");
            let sinks = pactl(&["list", "sinks"]);
            assert!(sinks.contains(&format!("{label} (Audio Graph)")), "{label}");
        }
        ticks(&mut backend, &Plan::default());
        assert!(our_modules().is_empty());
    }

    /// Cable renames while an app plays through it and links run: after each one
    /// the engine reaches the plan within a few ticks. The server may stall (PulseAudio in WSLg
    /// delays answers by seconds under frequent device creation): then the engine reports
    /// it and catches up when the server answers
    #[test]
    #[ignore]
    fn renames_under_load_converge() {
        let _devices = Devices::new();
        let (mut backend, _) = backend();
        let _player = play_silence();
        std::thread::sleep(Duration::from_millis(500));
        let plan = |label: &str| Plan {
            virtual_cables: vec![VirtualCable { label: label.into(), ..cable() }],
            streams: vec![
                PlanStream::through("audiograph.t1", false, &[("agtest.mic.src", [1.0; 2], 0.0)]),
                PlanStream::through("agtest.out", true, &[("audiograph.t1.out", [0.7; 2], 0.0)]),
            ],
            routes: vec![AppRoute {
                node: "app".into(),
                app: AppRef { exe: "pacat".into(), path: String::new(), name: "pacat".into() },
                render: Some(Target::Device("audiograph.t1".into())),
                capture: None,
            }],
            ..Plan::default()
        };
        ticks(&mut backend, &plan("Start"));
        for i in 0..12 {
            let label = format!("Name {i}");
            let started = Instant::now();
            let mut done = false;
            let mut last = Vec::new();
            while !done && started.elapsed() < Duration::from_secs(20) {
                let (streams, apps) = backend.apply(&plan(&label));
                last = streams.iter().flat_map(|s| s.errors.iter().map(|e| e.message.clone())).chain(apps.iter().filter_map(|a| a.error.clone())).collect();
                let renamed = pactl(&["list", "sinks"]).contains(&format!("{label} (Audio Graph)"));
                done = last.is_empty() && renamed && pacat_stream().map(|s| s.0).as_deref() == Some("audiograph.t1");
                if !done {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            println!("{label}: done after {:?}", started.elapsed());
            assert!(done, "{label}: didn't converge in 20 s, errors {last:?}");
        }
        ticks(&mut backend, &Plan::default());
        assert!(our_modules().is_empty());
    }

    #[test]
    #[ignore]
    fn stray_modules_of_a_crashed_run_are_removed() {
        // "the previous run crashed": its cable is left on the server
        let stray = pactl(&["load-module", "module-null-sink", "sink_name=audiograph.stray"]);
        assert!(stray.parse::<u32>().is_ok());
        let (mut backend, _) = backend();
        backend.apply(&Plan { virtual_cables: vec![cable()], ..Plan::default() });
        let left = our_modules();
        println!("after the first tick: {left:#?}");
        assert!(!left.iter().any(|m| m.contains("audiograph.stray")));
        backend.apply(&Plan::default());
        assert!(our_modules().is_empty());
    }
}
