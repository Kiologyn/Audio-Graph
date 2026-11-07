//! The system's own volume sliders of devices and apps. A graph can set them for the time it runs:
//! the value before is written down first and goes back when the last graph that changed it stops,
//! after a crash too. A stopped graph touches nothing. The interface reads the sliders regularly.

use crate::devices::Flow;
use crate::graphs::{DeviceRef, Graph, GraphNode, NodeKind};
use crate::storage::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Target {
    /// A device; without an id the system's default one
    Device { flow: Flow, id: Option<String> },
    /// An app by its executable file name, in lower case
    App { exe: String },
}

/// The system slider behind a node, if it has one and something is chosen
pub fn node_target(node: &GraphNode) -> Option<Target> {
    let device = |flow: Flow, device: &Option<DeviceRef>| match device.as_ref()? {
        DeviceRef::Default => Some(Target::Device { flow, id: None }),
        DeviceRef::Device { id, .. } => Some(Target::Device { flow, id: Some(id.clone()) }),
    };
    match node.kind {
        NodeKind::Input => device(Flow::Capture, &node.source),
        NodeKind::Output => device(Flow::Render, &node.sink),
        NodeKind::App => Some(Target::App { exe: node.app.as_ref()?.exe.to_lowercase() }),
        NodeKind::Cable | NodeKind::System => None,
    }
}

/// The default device by its id: it may change while the graph runs, the volume goes back to the same one
fn resolved(target: &Target) -> Target {
    match target {
        Target::Device { flow, id: None } => {
            let (render, capture) = crate::devices::defaults();
            Target::Device { flow: *flow, id: if *flow == Flow::Render { render } else { capture } }
        }
        other => other.clone(),
    }
}

fn read_one(target: &Target) -> Option<f32> {
    platform::read(std::slice::from_ref(target)).pop().flatten()
}

/// A slider a graph changed: what it was before, and the running graphs that changed it
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Saved {
    target: Target,
    previous: f32,
    graphs: Vec<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What graphs did to the system's sliders
pub struct VolumeMemory {
    path: PathBuf,
    saved: Mutex<Vec<Saved>>,
    /// Nodes whose value has reached the system in this run: later the sliders move on their own
    applied: Mutex<HashMap<(String, String), Target>>,
}

impl VolumeMemory {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("system-volumes.json");
        let saved = fs::read(&path).ok().and_then(|text| serde_json::from_slice(&text).ok()).unwrap_or_default();
        Self { path, saved: Mutex::new(saved), applied: Mutex::new(HashMap::new()) }
    }

    fn write(&self, saved: &[Saved]) {
        let written = serde_json::to_vec_pretty(saved).map_err(|e| e.to_string()).and_then(|text| write_atomic(&self.path, &text).map_err(|e| e.to_string()));
        if let Err(e) = written {
            crate::log::write(format!("failed to write the system volumes: {e}"));
        }
    }

    /// A running graph sets a slider; false if there is nothing to set yet
    pub fn set(&self, graph: &str, target: &Target, level: f32) -> bool {
        self.set_with(graph, &resolved(target), level, read_one, |t, v| platform::write(t, v).is_ok())
    }

    fn set_with(&self, graph: &str, target: &Target, level: f32, read: impl Fn(&Target) -> Option<f32>, write: impl Fn(&Target, f32) -> bool) -> bool {
        let Some(current) = read(target) else { return false };
        let mut saved = lock(&self.saved);
        match saved.iter_mut().find(|s| s.target == *target) {
            Some(entry) if entry.graphs.iter().any(|g| g == graph) => {}
            Some(entry) => {
                entry.graphs.push(graph.to_string());
                self.write(&saved);
            }
            None => {
                saved.push(Saved { target: target.clone(), previous: current, graphs: vec![graph.to_string()] });
                self.write(&saved);
            }
        }
        write(target, level.clamp(0.0, 1.0))
    }

    /// Every engine tick: the values of running graphs reach the system once per run, an app's
    /// as soon as it opens sound, and what stopped graphs changed goes back
    pub fn follow(&self, running: &[&Graph]) {
        let mut applied = lock(&self.applied);
        applied.retain(|(graph, _), _| running.iter().any(|g| g.id == *graph));
        for graph in running {
            for node in &graph.nodes {
                let (Some(level), Some(target)) = (node.system_volume, node_target(node)) else { continue };
                let key = (graph.id.clone(), node.id.clone());
                if applied.get(&key) != Some(&target) && self.set(&graph.id, &target, level) {
                    applied.insert(key, target);
                }
            }
        }
        let ids: Vec<&str> = running.iter().map(|g| g.id.as_str()).collect();
        self.restore_with(&ids, read_one, |t, v| platform::write(t, v).is_ok());
    }

    fn restore_with(&self, running: &[&str], read: impl Fn(&Target) -> Option<f32>, write: impl Fn(&Target, f32) -> bool) {
        let mut saved = lock(&self.saved);
        if saved.is_empty() {
            return;
        }
        let before = saved.clone();
        saved.retain_mut(|entry| {
            entry.graphs.retain(|g| running.contains(&g.as_str()));
            // a device that is gone or an app without sound gets its volume when it is back
            !entry.graphs.is_empty() || read(&entry.target).is_none() || !write(&entry.target, entry.previous)
        });
        if *saved != before {
            self.write(&saved);
        }
    }
}

/// The slider positions from 0 to 1; None if there is nothing to read: the device
/// isn't available, the app has no sound open
#[tauri::command(async)]
pub fn system_volumes(targets: Vec<Target>) -> Vec<Option<f32>> {
    platform::read(&targets)
}

#[cfg(all(test, target_os = "linux"))]
pub use platform::write as write_volume;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn app(exe: &str) -> Target {
        Target::App { exe: exe.into() }
    }

    fn memory(name: &str) -> VolumeMemory {
        let dir = std::env::temp_dir().join(format!("ag-volumes-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        VolumeMemory::new(&dir)
    }

    /// A pretend system: the slider of one target
    struct System(RefCell<Option<f32>>);

    impl System {
        fn read(&self) -> impl Fn(&Target) -> Option<f32> + '_ {
            |_| *self.0.borrow()
        }
        fn write(&self) -> impl Fn(&Target, f32) -> bool + '_ {
            |_, v| {
                *self.0.borrow_mut() = Some(v);
                true
            }
        }
    }

    #[test]
    fn the_first_value_goes_back_when_the_graph_stops() {
        let (m, s, t) = (memory("back"), System(RefCell::new(Some(0.7))), app("a"));
        m.set_with("g", &t, 0.3, s.read(), s.write());
        m.set_with("g", &t, 0.2, s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.2));
        m.restore_with(&["g"], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.2));
        m.restore_with(&[], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.7));
        assert!(lock(&m.saved).is_empty());
    }

    #[test]
    fn it_waits_for_the_last_graph_that_changed_the_slider() {
        let (m, s, t) = (memory("last"), System(RefCell::new(Some(0.7))), app("a"));
        m.set_with("a", &t, 0.3, s.read(), s.write());
        m.set_with("b", &t, 0.2, s.read(), s.write());
        m.restore_with(&["b"], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.2));
        m.restore_with(&[], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.7));
    }

    #[test]
    fn what_isnt_there_gets_its_volume_when_it_is_back() {
        let (m, s, t) = (memory("gone"), System(RefCell::new(Some(0.7))), app("a"));
        m.set_with("g", &t, 0.3, s.read(), s.write());
        *s.0.borrow_mut() = None;
        m.restore_with(&[], s.read(), s.write());
        assert_eq!(lock(&m.saved).len(), 1);
        *s.0.borrow_mut() = Some(0.3);
        m.restore_with(&[], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.7));
    }

    #[test]
    fn nothing_to_set_is_left_alone() {
        let (m, s, t) = (memory("none"), System(RefCell::new(None)), app("a"));
        assert!(!m.set_with("g", &t, 0.3, s.read(), s.write()));
        assert!(lock(&m.saved).is_empty());
    }

    /// After a crash the file still knows: the first tick without the graph puts the volume back
    #[test]
    fn a_crash_loses_nothing() {
        let (m, s, t) = (memory("crash"), System(RefCell::new(Some(0.7))), app("a"));
        m.set_with("g", &t, 0.3, s.read(), s.write());
        let again = VolumeMemory::new(m.path.parent().unwrap());
        again.restore_with(&[], s.read(), s.write());
        assert_eq!(*s.0.borrow(), Some(0.7));
    }
}

#[cfg(windows)]
mod platform {
    use super::Target;
    use crate::apps::win::{process_path, session_volumes};
    use crate::com::{wide, ComGuard};
    use crate::devices::Flow;
    use std::collections::HashMap;
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eCapture, eConsole, eRender, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    fn endpoint(flow: Flow, id: Option<&str>) -> Option<IAudioEndpointVolume> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = match id {
                Some(id) => {
                    let id = wide(id);
                    enumerator.GetDevice(PCWSTR(id.as_ptr()))
                }
                None => enumerator.GetDefaultAudioEndpoint(if flow == Flow::Render { eRender } else { eCapture }, eConsole),
            };
            device.ok()?.Activate(CLSCTX_ALL, None).ok()
        }
    }

    /// The volume controls of apps with sound open: exe in lower case → its sessions
    fn apps() -> HashMap<String, Vec<ISimpleAudioVolume>> {
        let mut apps: HashMap<String, Vec<ISimpleAudioVolume>> = HashMap::new();
        for (pid, controls) in session_volumes() {
            if let Some(path) = process_path(pid) {
                let exe = path.rsplit('\\').next().unwrap_or(&path).to_lowercase();
                apps.entry(exe).or_default().extend(controls);
            }
        }
        apps
    }

    pub fn read(targets: &[Target]) -> Vec<Option<f32>> {
        let _com = ComGuard::init();
        let sessions = apps();
        targets
            .iter()
            .map(|target| match target {
                Target::Device { flow, id } => {
                    let volume = endpoint(*flow, id.as_deref())?;
                    unsafe { volume.GetMasterVolumeLevelScalar() }.ok()
                }
                Target::App { exe } => unsafe { sessions.get(exe)?.first()?.GetMasterVolume() }.ok(),
            })
            .collect()
    }

    pub fn write(target: &Target, level: f32) -> Result<(), String> {
        let _com = ComGuard::init();
        match target {
            Target::Device { flow, id } => {
                let volume = endpoint(*flow, id.as_deref()).ok_or_else(|| tr!("error_device_not_found"))?;
                unsafe { volume.SetMasterVolumeLevelScalar(level, std::ptr::null()) }.map_err(|e| e.message())
            }
            Target::App { exe } => {
                for control in apps().get(exe).into_iter().flatten() {
                    unsafe { control.SetMasterVolume(level, std::ptr::null()) }.map_err(|e| e.message())?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Target;
    use crate::apps::linux::is_app_stream;
    use crate::devices::Flow;
    use crate::pulse::{self, Device, Pulse, Stream};

    /// What the server knows about the targets, read once
    struct Server {
        sinks: Vec<Device>,
        sources: Vec<Device>,
        default_sink: Option<String>,
        default_source: Option<String>,
        streams: Vec<Stream>,
    }

    impl Server {
        fn read(pulse: &mut Pulse) -> Result<Self, String> {
            let server = pulse.server()?;
            Ok(Self {
                sinks: pulse.sinks()?,
                sources: pulse.sources()?,
                default_sink: server.default_sink,
                default_source: server.default_source,
                streams: pulse.sink_inputs()?,
            })
        }

        fn device(&self, flow: Flow, id: Option<&str>) -> Option<&Device> {
            let (devices, default) = match flow {
                Flow::Render => (&self.sinks, &self.default_sink),
                Flow::Capture => (&self.sources, &self.default_source),
            };
            let name = id.or(default.as_deref())?;
            devices.iter().find(|d| d.name == name)
        }

        fn app<'a>(&'a self, exe: &'a str) -> impl Iterator<Item = &'a Stream> {
            self.streams
                .iter()
                .filter(move |s| s.volume_writable && is_app_stream(s) && s.binary.as_deref().is_some_and(|b| b.to_lowercase() == exe))
        }
    }

    pub fn read(targets: &[Target]) -> Vec<Option<f32>> {
        let server = Pulse::connect("Audio Graph").and_then(|mut pulse| Server::read(&mut pulse));
        let Ok(server) = server else { return vec![None; targets.len()] };
        targets
            .iter()
            .map(|target| match target {
                Target::Device { flow, id } => server.device(*flow, id.as_deref()).map(|d| pulse::slider_level(&d.volume)),
                Target::App { exe } => server.app(exe).next().map(|s| pulse::slider_level(&s.volume)),
            })
            .collect()
    }

    pub fn write(target: &Target, level: f32) -> Result<(), String> {
        let mut pulse = Pulse::connect("Audio Graph")?;
        let server = Server::read(&mut pulse)?;
        match target {
            Target::Device { flow, id } => {
                let device = server.device(*flow, id.as_deref()).ok_or_else(|| tr!("error_device_not_found"))?;
                let volume = pulse::rescaled(&device.volume, level);
                match flow {
                    Flow::Render => pulse.set_sink_volume(&device.name, &volume),
                    Flow::Capture => pulse.set_source_volume(&device.name, &volume),
                }
            }
            Target::App { exe } => {
                for stream in server.app(exe) {
                    pulse.set_sink_input_volume(stream.index, &pulse::rescaled(&stream.volume, level))?;
                }
                Ok(())
            }
        }
    }
}
