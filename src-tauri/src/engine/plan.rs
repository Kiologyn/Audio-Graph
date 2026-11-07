//! Turns a graph into the engine's work plan. Platform-independent.
//!
//! What the system does and what the engine does itself:
//! - an app's links are the system's per-app output/input device settings: the app
//!   plays straight into the device of the node on its right and hears the node on its left, the engine doesn't touch the sound;
//! - other links (input, output, cables) are engine streams: it reads recording devices,
//!   mixes and plays into an output device. An internal cable is a mix point inside the engine.
//!   What comes out of an output node is everything playing on its device, not only the graph's sound;
//! - links of the "System" node are the system default devices: its output is the sound of apps
//!   playing into the default output device, its input is what they hear as the microphone.
//!
//! A cable is a step of its own in the sound's way, wherever it is carried out: what comes to it is summed,
//! its noise gate listens to that sum, then its volume and delay act on it. What it passes on is what the next
//! node gets. Sound that takes several paths adds up, as two wires into a mixer would (Stage).
//!
//! On Linux cables aren't chosen from a list: every cable node is a virtual device of its own,
//! created while the graph runs (virtualize). The rest of the plan is the same.

use crate::cables::Cable;
use crate::graphs::{AppRef, DeviceRef, Graph, GraphEdge, GraphNode, NodeKind};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Something in the graph that can't be done. The rest of the graph still works.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub nodes: Vec<String>,
    pub edges: Vec<String>,
    pub message: String,
}

/// A device assigned to an app
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Like the system: remove the app's own setting
    Default,
    Device(String),
}

/// System settings for one app
#[derive(Clone, Debug, PartialEq)]
pub struct AppRoute {
    pub node: String,
    pub app: AppRef,
    /// Where the app plays
    pub render: Option<Target>,
    /// Where the app takes its microphone from
    pub capture: Option<Target>,
}

/// Volume of the left and the right channel
pub type Gain = [f32; 2];

/// A node's volume with its balance: minus quiets the right channel, plus the left one
fn balanced(volume: f32, balance: f32) -> Gain {
    let b = balance.clamp(-1.0, 1.0);
    [volume * (1.0 - b.max(0.0)), volume * (1.0 + b.min(0.0))]
}

/// A recording device in a stream
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub device: String,
    /// How much later the sound is played, in seconds. After compile it is the sum of the delays
    /// on the path (cables, the output) as set; apply_delays turns it into what the engine plays (never negative).
    /// All of a path's delay sits here: a delay after a stage only moves the stage's whole result in time
    pub delay: f32,
    /// The stage it goes into: an index in the stream's stages
    pub stage: usize,
}

/// A cable at work inside a stream: what comes to it is summed, sound quieter than the gate is
/// muted, and the rest goes on to the parent stage with the volume. A node on several paths into a
/// device has a stage on each of them, and they all hear the same sound
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    /// The same for as long as the path to it is: the stage keeps its state from plan to plan
    pub id: String,
    /// The node whose level it measures; the first stage is nobody's
    pub node: String,
    /// Where its sound goes: an earlier stage of the stream, the first one goes into the device
    pub parent: usize,
    pub gain: Gain,
    /// In dB
    pub gate: f32,
}

/// What an output device plays: stages, the first being the device's own sum, and the sources going into them
#[derive(Clone, Debug, PartialEq)]
struct Mix {
    stages: Vec<Stage>,
    sources: Vec<Source>,
}

impl Default for Mix {
    fn default() -> Self {
        let root = Stage { id: String::new(), node: String::new(), parent: 0, gain: [1.0; 2], gate: NO_GATE };
        Self { stages: vec![root], sources: Vec::new() }
    }
}

impl Mix {
    /// Adds a stage for a node reached by a link (or for the node itself); returns its index
    fn add_stage(&mut self, parent: usize, link: &str, node: &str, gain: Gain, gate: f32) -> usize {
        let id = format!("{}/{link}", self.stages[parent].id);
        self.stages.push(Stage { id, node: node.to_string(), parent, gain, gate });
        self.stages.len() - 1
    }

    /// Adds what another mix plays: both go into the device
    fn absorb(&mut self, other: Mix) {
        let shift = self.stages.len() - 1;
        let moved = |stage: usize| if stage == 0 { 0 } else { stage + shift };
        self.stages.extend(other.stages.into_iter().skip(1).map(|s| Stage { parent: moved(s.parent), ..s }));
        self.sources.extend(other.sources.into_iter().map(|s| Source { stage: moved(s.stage), ..s }));
    }
}

/// An engine stream: sound of recording devices goes through stages and plays into an output device
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub sink: String,
    pub stages: Vec<Stage>,
    pub sources: Vec<Source>,
    /// The device is an output node's: output delays apply to it and shift with the other outputs.
    /// An output's delay never goes into a cable, otherwise it would reach every output behind the cable
    pub output: bool,
}

impl Stream {
    /// The stream's sources, each with its number among the sources of its device: one device
    /// can come several times, delayed differently, and each time is a link of its own
    pub fn layers(&self) -> impl Iterator<Item = (&Source, usize)> {
        self.sources.iter().enumerate().map(|(i, s)| (s, self.sources[..i].iter().filter(|o| o.device == s.device).count()))
    }

    /// A stream of sources that each go through a stage of their own with this volume
    #[cfg(test)]
    pub fn through(sink: &str, output: bool, sources: &[(&str, Gain, f32)]) -> Self {
        let mut mix = Mix::default();
        for (i, &(device, gain, delay)) in sources.iter().enumerate() {
            let stage = mix.add_stage(0, &i.to_string(), "", gain, NO_GATE);
            mix.sources.push(Source { device: device.to_string(), delay, stage });
        }
        Self { sink: sink.to_string(), stages: mix.stages, sources: mix.sources, output }
    }

    /// The volume a source plays with: the volumes of the stages on its way, multiplied
    #[cfg(test)]
    pub fn gain_of(&self, source: &Source) -> Gain {
        let mut gain = self.stages[0].gain;
        let mut stage = source.stage;
        while stage != 0 {
            gain = [gain[0] * self.stages[stage].gain[0], gain[1] * self.stages[stage].gain[1]];
            stage = self.stages[stage].parent;
        }
        gain
    }
}

/// The system default devices the "System" node sets
#[derive(Clone, Debug, PartialEq)]
pub struct SystemRoute {
    pub node: String,
    pub render: Option<String>,
    pub capture: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub routes: Vec<AppRoute>,
    pub system: Option<SystemRoute>,
    pub streams: Vec<Stream>,
    pub issues: Vec<Issue>,
    /// Virtual cable devices to create (Linux)
    pub virtual_cables: Vec<VirtualCable>,
}

/// A cable the graph creates itself: an output device (the cable's input) and an input device (its output)
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualCable {
    pub node: String,
    /// The node's label: the system shows the devices under this name
    pub label: String,
    pub sink: String,
    pub source: String,
}

/// The prefix of virtual device names: it tells them apart from the system's devices
pub const VIRTUAL_PREFIX: &str = "audiograph.";

/// The most a link delays sound, in seconds. A delay is held in the link's buffer, so only memory
/// limits it: a second takes 384 KB, the longest delay about 460 MB
pub const MAX_DELAY: f32 = 1200.0;

/// A noise gate at this level or below lets everything through
pub const NO_GATE: f32 = -100.0;

/// A stream source "loopback:<output device>" is the sound played into that output device
/// (WASAPI loopback, the sink's monitor on Linux). It is what comes out of an output node,
/// and it stands in for the output of a cable that has only its input chosen
pub const LOOPBACK_PREFIX: &str = "loopback:";

/// The recording device the engine reads a cable's sound from: its output, or without one
/// the loopback of its input. Apps can't listen to a loopback, only the engine can
fn cable_source(cable: &Cable) -> Option<String> {
    device_id(&cable.capture)
        .map(str::to_string)
        .or_else(|| device_id(&cable.render).map(|id| format!("{LOOPBACK_PREFIX}{id}")))
}

/// Device names of a node's virtual cable: the output device and the input device.
/// Matches virtualDeviceIds in src/platform.ts
pub fn virtual_device_names(node_id: &str) -> (String, String) {
    // a device name allows only letters, digits, ".", "-" and "_"
    let id: String = node_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    (format!("{VIRTUAL_PREFIX}{id}"), format!("{VIRTUAL_PREFIX}{id}.out"))
}

/// The graph with every cable node referring to its own virtual cable, these cables
/// (for compile) and the devices to create for them
pub fn virtualize(graph: &Graph) -> (Graph, Vec<Cable>, Vec<VirtualCable>) {
    let mut graph = graph.clone();
    let mut cables = Vec::new();
    let mut devices = Vec::new();
    for node in graph.nodes.iter_mut().filter(|n| n.kind == NodeKind::Cable) {
        let (sink, source) = virtual_device_names(&node.id);
        let id = format!("virtual:{}", node.id);
        let device = |id: &str| Some(DeviceRef::Device { id: id.to_string(), name: node.label.clone() });
        cables.push(Cable {
            id: id.clone(),
            name: node.label.clone(),
            render: device(&sink),
            capture: device(&source),
            created_at: 0,
        });
        devices.push(VirtualCable {
            node: node.id.clone(),
            label: node.label.clone(),
            sink,
            source,
        });
        node.cable = Some(id);
    }
    (graph, cables, devices)
}

/// The current system default devices: the engine opens "default" in input and output nodes
/// as a specific device and reopens it when the system changes it.
/// If the graph replaces them with the "System" node, "default" means the replacement device
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Defaults {
    pub render: Option<String>,
    pub capture: Option<String>,
}

/// What a node is for the plan
enum Role<'a> {
    Input(Option<&'a DeviceRef>),
    Output(Option<&'a DeviceRef>),
    App,
    /// The system default devices
    System,
    /// A cable from the shared list: its input is an output device, its output a recording device
    Cable(&'a Cable),
    /// An internal cable: mixing inside the engine, invisible to the system
    Bus,
    /// The node refers to a removed cable
    Deleted,
}

/// Where the engine does the work of a cable from the list: on its way into the cable, on the sum of what it
/// plays there (everything that leaves the cable is affected, apps listening to it too), or on its way out, on the
/// sum the cable gives (everything that entered is affected, apps playing into it too)
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Into,
    OutOf,
}

struct Compiler<'a> {
    nodes: HashMap<&'a str, &'a GraphNode>,
    cables: HashMap<&'a str, &'a Cable>,
    /// Taking the "System" node replacement into account
    defaults: Defaults,
    issues: Vec<Issue>,
    /// The graph's delay slider range
    delay_limit: f32,
    /// The graph's maximum volume
    volume_limit: f32,
    /// Where each cable from the list does its work (place_cables)
    sides: HashMap<&'a str, Side>,
}

impl<'a> Compiler<'a> {
    fn role(&self, node: &'a GraphNode) -> Role<'a> {
        match node.kind {
            NodeKind::Input => Role::Input(node.source.as_ref()),
            NodeKind::Output => Role::Output(node.sink.as_ref()),
            NodeKind::App => Role::App,
            NodeKind::System => Role::System,
            NodeKind::Cable => match &node.cable {
                None => Role::Bus,
                Some(id) => self.cables.get(id.as_str()).map_or(Role::Deleted, |c| Role::Cable(c)),
            },
        }
    }

    /// What a node does to the sound passing through the engine: a cable's volume and balance.
    /// The sliders of devices and apps are the system's, it applies them itself
    fn gain(&self, node: &GraphNode) -> Gain {
        if node.kind == NodeKind::Cable {
            balanced(self.volume(node), node.balance)
        } else {
            [1.0; 2]
        }
    }

    /// A node's volume within the graph's maximum: a limit lowered later acts on what was set before
    fn volume(&self, node: &GraphNode) -> f32 {
        node.volume.min(self.volume_limit)
    }

    /// The system carries out the links of apps and "System", not the engine
    fn is_routed(&self, id: &str) -> bool {
        matches!(self.nodes[id].kind, NodeKind::App | NodeKind::System)
    }

    /// A node's delay within the graph's slider range. An output's is how much earlier it sounds
    /// than the rest, always a minus; a cable only holds sound back. Other nodes have none,
    /// whatever files of earlier versions say
    fn delay(&self, node: &GraphNode) -> f32 {
        match node.kind {
            NodeKind::Output => node.delay.clamp(-self.delay_limit, 0.0),
            NodeKind::Cable => node.delay.clamp(0.0, self.delay_limit),
            NodeKind::Input | NodeKind::App | NodeKind::System => 0.0,
        }
    }

    /// A cable's noise gate in dB
    fn gate(&self, node: &GraphNode) -> f32 {
        if node.kind == NodeKind::Cable {
            node.gate.clamp(NO_GATE, 0.0)
        } else {
            NO_GATE
        }
    }

    /// Where each cable from the list does its work (Side). The engine can only touch the sound it carries
    /// itself: its way into the cable is taken unless an app or "System" plays into the cable, then its
    /// way out of it. What the system carries between the cable and such a node on the other side
    /// passes the cable's settings by: that is an issue
    fn place_cables(&mut self, nodes: &'a [GraphNode], edges: &[&'a GraphEdge]) {
        for node in nodes {
            let Role::Cable(cable) = self.role(node) else { continue };
            let (direct_in, mut engine_in): (Vec<&GraphEdge>, Vec<&GraphEdge>) =
                edges.iter().copied().filter(|e| e.target == node.id).partition(|e| self.is_routed(&e.source));
            // nothing can be played into a cable without an input: that is reported where the sound would go
            if device_id(&cable.render).is_none() {
                engine_in.clear();
            }
            let (direct_out, engine_out): (Vec<&GraphEdge>, Vec<&GraphEdge>) =
                edges.iter().copied().filter(|e| e.source == node.id).partition(|e| self.is_routed(&e.target));
            let (side, undelayed) = if !engine_in.is_empty() && direct_in.is_empty() {
                (Some(Side::Into), Vec::new())
            } else if !engine_out.is_empty() && direct_out.is_empty() {
                (Some(Side::OutOf), Vec::new())
            } else if !engine_in.is_empty() {
                (Some(Side::Into), direct_in)
            } else if !engine_out.is_empty() {
                (Some(Side::OutOf), direct_out)
            } else {
                (None, [direct_in, direct_out].concat())
            };
            if let Some(side) = side {
                self.sides.insert(node.id.as_str(), side);
            }
            if !undelayed.is_empty() {
                let mut ids = vec![node.id.as_str()];
                ids.extend(undelayed.iter().map(|e| if e.source == node.id { e.target.as_str() } else { e.source.as_str() }));
                let links: Vec<&str> = undelayed.iter().map(|e| e.id.as_str()).collect();
                if self.gain(node) != [1.0; 2] {
                    self.issue(&ids, &links, tr!("issue_cable_volume_direct"));
                }
                if self.delay(node) != 0.0 {
                    self.issue(&ids, &links, tr!("issue_cable_delay_direct"));
                }
                if self.gate(node) != NO_GATE {
                    self.issue(&ids, &links, tr!("issue_cable_gate_direct"));
                }
            }
        }
    }

    fn issue(&mut self, nodes: &[&str], edges: &[&str], message: impl Into<String>) {
        let issue = Issue {
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            edges: edges.iter().map(|s| s.to_string()).collect(),
            message: message.into(),
        };
        // the same node turns up on the paths of several links
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }

    /// The device the engine reads the node's sound from
    fn source_device(&mut self, node: &'a GraphNode) -> Option<String> {
        match self.role(node) {
            Role::Input(None) => self.issue(&[&node.id], &[], tr!("issue_no_input_device")),
            Role::Input(Some(DeviceRef::Device { id, .. })) => return Some(id.clone()),
            Role::Input(Some(DeviceRef::Default)) => match &self.defaults.capture {
                Some(id) => return Some(id.clone()),
                None => self.issue(&[&node.id], &[], tr!("issue_no_default_input")),
            },
            Role::Cable(cable) => match cable_source(cable) {
                Some(id) => return Some(id),
                None => self.issue(&[&node.id], &[], tr!(
                    "issue_cable_no_output",
                    cable = cable.name
                )),
            },
            Role::Output(_) => return self.sink_device(node).map(|id| format!("{LOOPBACK_PREFIX}{id}")),
            Role::Deleted => self.issue(&[&node.id], &[], tr!("issue_cable_deleted")),
            Role::App | Role::System | Role::Bus => {}
        }
        None
    }

    /// The device the engine plays the node's sound into
    fn sink_device(&mut self, node: &'a GraphNode) -> Option<String> {
        match self.role(node) {
            Role::Output(None) => self.issue(&[&node.id], &[], tr!("issue_no_output_device")),
            Role::Output(Some(DeviceRef::Device { id, .. })) => return Some(id.clone()),
            Role::Output(Some(DeviceRef::Default)) => match &self.defaults.render {
                Some(id) => return Some(id.clone()),
                None => self.issue(&[&node.id], &[], tr!("issue_no_default_output")),
            },
            Role::Cable(cable) => match &cable.render {
                Some(DeviceRef::Device { id, .. }) => return Some(id.clone()),
                _ => self.issue(&[&node.id], &[], tr!("issue_cable_no_input", cable = cable.name)),
            },
            Role::Deleted => self.issue(&[&node.id], &[], tr!("issue_cable_deleted")),
            Role::Input(_) | Role::App | Role::System | Role::Bus => {}
        }
        None
    }

    /// Where to send an app's sound: the device of the node the link goes to
    fn app_render(&mut self, edge: &GraphEdge, target: &'a GraphNode) -> Option<Target> {
        let ids = [edge.source.as_str(), target.id.as_str()];
        match self.role(target) {
            Role::Output(Some(DeviceRef::Default)) => return Some(Target::Default),
            Role::Output(_) | Role::Cable(_) | Role::Deleted => return self.sink_device(target).map(Target::Device),
            Role::Bus => self.issue(
                &ids,
                &[&edge.id],
                tr!(
                    "issue_app_to_internal_cable"
                ),
            ),
            Role::App | Role::System => {
                self.issue(
                    &ids,
                    &[&edge.id],
                    tr!(
                        "issue_device_to_device"
                    ),
                )
            }
            Role::Input(_) => {}
        }
        None
    }

    /// Where an app takes its microphone from: the device of the node the link comes from
    fn app_capture(&mut self, edge: &GraphEdge, source: &'a GraphNode) -> Option<Target> {
        let ids = [source.id.as_str(), edge.target.as_str()];
        match self.role(source) {
            Role::Input(Some(DeviceRef::Default)) => return Some(Target::Default),
            // the system gives an app only a recording device, not a loopback of the cable's input
            Role::Cable(cable) if device_id(&cable.capture).is_none() => self.issue(
                &ids,
                &[&edge.id],
                tr!("issue_cable_no_output_to_app", cable = cable.name),
            ),
            Role::Input(_) | Role::Cable(_) | Role::Deleted => return self.source_device(source).map(Target::Device),
            Role::Bus => self.issue(
                &ids,
                &[&edge.id],
                tr!(
                    "issue_internal_cable_to_app"
                ),
            ),
            // an output's monitor is a recording device like any other for the server
            Role::Output(_) if cfg!(target_os = "linux") => {
                return self.sink_device(source).map(|id| Target::Device(format!("{id}.monitor")));
            }
            // on Windows only the engine can listen to an output device
            Role::Output(_) => self.issue(&ids, &[&edge.id], tr!("issue_output_to_app")),
            // a link between two apps is reported by the source side
            Role::App | Role::System => {}
        }
        None
    }

    /// What comes to a node through engine links, into one of a mix's stages: recording devices, and on the way
    /// to them a stage for every cable that works there. Delay is what the path has gathered so far.
    /// Each path is followed, and what takes several paths sounds on each of them
    fn collect_sources(&mut self, node: &'a GraphNode, engine_edges: &[&'a GraphEdge], path: &mut HashSet<&'a str>, stage: usize, delay: f32, out: &mut Mix) {
        for edge in engine_edges.iter().filter(|e| e.target == node.id) {
            let source = self.nodes[edge.source.as_str()];
            if matches!(self.role(source), Role::Bus) {
                if path.insert(source.id.as_str()) {
                    let inner = out.add_stage(stage, &edge.id, &source.id, self.gain(source), self.gate(source));
                    self.collect_sources(source, engine_edges, path, inner, delay + self.delay(source), out);
                    path.remove(source.id.as_str());
                }
            } else if let Some(device) = self.source_device(source) {
                let working_here = self.sides.get(source.id.as_str()) == Some(&Side::OutOf);
                let stage = if working_here { out.add_stage(stage, &edge.id, &source.id, self.gain(source), self.gate(source)) } else { stage };
                let held = if working_here { self.delay(source) } else { 0.0 };
                out.sources.push(Source { device, delay: delay + held, stage });
            }
        }
    }
}

fn device_id(device: &Option<DeviceRef>) -> Option<&str> {
    match device {
        Some(DeviceRef::Device { id, .. }) => Some(id),
        _ => None,
    }
}

/// Sound between devices: recording device → output device, and what the devices pass on themselves
type DeviceLinks = HashMap<String, BTreeSet<String>>;

/// The links that exist without the graph: a cable's input → its output
fn cable_links(cables: &[Cable]) -> DeviceLinks {
    let mut links = DeviceLinks::new();
    for c in cables {
        if let (Some(render), Some(capture)) = (device_id(&c.render), device_id(&c.capture)) {
            links.entry(render.to_string()).or_default().insert(capture.to_string());
        }
    }
    links
}

/// What a device plays reaches its loopback
fn link_loopback(links: &mut DeviceLinks, source: &str) {
    if let Some(device) = source.strip_prefix(LOOPBACK_PREFIX) {
        links.entry(device.to_string()).or_default().insert(source.to_string());
    }
}

/// Takes out of the streams the loopbacks whose sound would come back to their own device, and
/// returns them with the devices they went into. In a graph such a loop has no circle of arrows
/// when two nodes share a device
fn cut_device_loops(streams: &mut BTreeMap<String, Mix>, cables: &[Cable]) -> BTreeSet<(String, String)> {
    let mut links = cable_links(cables);
    for (sink, mix) in streams.iter() {
        for source in &mix.sources {
            link_loopback(&mut links, &source.device);
            links.entry(source.device.clone()).or_default().insert(sink.clone());
        }
    }
    let mut looped = BTreeSet::new();
    for (sink, mix) in streams.iter_mut() {
        mix.sources.retain(|source| {
            let closes = source.device.starts_with(LOOPBACK_PREFIX) && reaches(&links, sink, &source.device);
            if closes {
                looped.insert((source.device.clone(), sink.clone()));
            }
            !closes
        });
    }
    looped
}

/// Links that lie on loops, grouped by loop (strongly connected components)
fn cycles<'a>(nodes: &[&'a str], edges: &[&'a GraphEdge]) -> Vec<(Vec<&'a str>, Vec<&'a str>)> {
    // Tarjan's algorithm; graphs are small, recursion is enough
    struct Tarjan<'a, 'b> {
        edges: &'b [&'a GraphEdge],
        index: HashMap<&'a str, usize>,
        low: HashMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on_stack: HashSet<&'a str>,
        component: HashMap<&'a str, usize>,
        count: usize,
    }
    impl<'a, 'b> Tarjan<'a, 'b> {
        fn visit(&mut self, v: &'a str) {
            let i = self.index.len();
            self.index.insert(v, i);
            self.low.insert(v, i);
            self.stack.push(v);
            self.on_stack.insert(v);
            let next: Vec<&'a str> = self.edges.iter().filter(|e| e.source == v).map(|e| e.target.as_str()).collect();
            for w in next {
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let lw = self.low[w];
                    let lv = self.low.get_mut(v).unwrap();
                    *lv = (*lv).min(lw);
                } else if self.on_stack.contains(w) {
                    let iw = self.index[w];
                    let lv = self.low.get_mut(v).unwrap();
                    *lv = (*lv).min(iw);
                }
            }
            if self.low[v] == self.index[v] {
                while let Some(w) = self.stack.pop() {
                    self.on_stack.remove(w);
                    self.component.insert(w, self.count);
                    if w == v {
                        break;
                    }
                }
                self.count += 1;
            }
        }
    }

    let mut t = Tarjan {
        edges,
        index: HashMap::new(),
        low: HashMap::new(),
        stack: Vec::new(),
        on_stack: HashSet::new(),
        component: HashMap::new(),
        count: 0,
    };
    for &n in nodes {
        if !t.index.contains_key(n) {
            t.visit(n);
        }
    }
    // a link lies on a loop if both ends are in one component (a node's link to itself included)
    let mut groups: BTreeMap<usize, (Vec<&'a str>, Vec<&'a str>)> = BTreeMap::new();
    for e in edges {
        let c = t.component[e.source.as_str()];
        if c == t.component[e.target.as_str()] {
            let group = groups.entry(c).or_default();
            group.1.push(e.id.as_str());
            for n in [e.source.as_str(), e.target.as_str()] {
                if !group.0.contains(&n) {
                    group.0.push(n);
                }
            }
        }
    }
    groups.into_values().collect()
}

pub fn compile(graph: &Graph, cables: &[Cable], defaults: &Defaults) -> Plan {
    let mut c = Compiler {
        nodes: graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect(),
        cables: cables.iter().map(|c| (c.id.as_str(), c)).collect(),
        defaults: defaults.clone(),
        issues: Vec::new(),
        delay_limit: graph.delay_limit(),
        volume_limit: graph.volume_limit(),
        sides: HashMap::new(),
    };
    let node_ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut edges: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|e| c.nodes.contains_key(e.source.as_str()) && c.nodes.contains_key(e.target.as_str()))
        .collect();

    // loops: sound would go round in circles and howl. Such links aren't carried out
    let mut looped: HashSet<&str> = HashSet::new();
    for (nodes, loop_edges) in cycles(&node_ids, &edges) {
        c.issue(
            &nodes,
            &loop_edges,
            tr!("issue_loop"),
        );
        looped.extend(loop_edges);
    }
    edges.retain(|e| !looped.contains(e.id.as_str()));

    // apps and "System": each has one output device and one input device.
    // "System" goes first: it decides what "default" means for apps
    let mut routes = Vec::new();
    let mut system: Option<SystemRoute> = None;
    let mut seen_apps: HashMap<String, &str> = HashMap::new();
    let ordered = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::System)
        .chain(graph.nodes.iter().filter(|n| n.kind == NodeKind::App));
    for node in ordered {
        let outgoing: Vec<&GraphEdge> = edges.iter().copied().filter(|e| e.source == node.id).collect();
        let incoming: Vec<&GraphEdge> = edges.iter().copied().filter(|e| e.target == node.id).collect();
        if outgoing.is_empty() && incoming.is_empty() {
            continue;
        }
        let is_system = node.kind == NodeKind::System;
        if !is_system && node.app.is_none() {
            c.issue(&[&node.id], &[], tr!("app_none_selected"));
            continue;
        }

        let render = match outgoing.as_slice() {
            [] => None,
            [edge] => {
                let target = c.nodes[edge.target.as_str()];
                let render = c.app_render(edge, target);
                if render.is_some() && target.kind == NodeKind::Output && c.delay(target) != 0.0 {
                    c.issue(&[&target.id, &node.id], &[&edge.id], tr!("issue_delay_direct"));
                }
                render
            }
            many => {
                let ids: Vec<&str> = many.iter().map(|e| e.id.as_str()).collect();
                c.issue(
                    &[&node.id],
                    &ids,
                    if is_system {
                        tr!(
                            "issue_default_output_fan_out"
                        )
                    } else {
                        tr!(
                            "issue_app_fan_out"
                        )
                    },
                );
                None
            }
        };
        let capture = match incoming.as_slice() {
            [] => None,
            [edge] => {
                c.app_capture(edge, c.nodes[edge.source.as_str()])
            }
            many => {
                let ids: Vec<&str> = many.iter().map(|e| e.id.as_str()).collect();
                c.issue(
                    &[&node.id],
                    &ids,
                    if is_system {
                        tr!(
                            "issue_default_input_fan_in"
                        )
                    } else {
                        tr!(
                            "issue_app_fan_in"
                        )
                    },
                );
                None
            }
        };

        if is_system {
            if let Some(first) = &system {
                c.issue(&[&first.node, &node.id], &[], tr!(
                    "issue_system_node_repeated"
                ));
                continue;
            }
            // "System → default output" changes nothing
            let device = |t: Option<Target>| match t {
                Some(Target::Device(id)) => Some(id),
                _ => None,
            };
            let route = SystemRoute {
                node: node.id.clone(),
                render: device(render),
                capture: device(capture),
            };
            // from here on "default" in nodes means the replacement devices
            if let Some(id) = &route.render {
                c.defaults.render = Some(id.clone());
            }
            if let Some(id) = &route.capture {
                c.defaults.capture = Some(id.clone());
            }
            system = Some(route);
            continue;
        }

        if render.is_none() && capture.is_none() {
            continue;
        }
        // the system stores the setting per app, not per node
        let app = node.app.as_ref().unwrap();
        let key = app.exe.to_lowercase();
        if let Some(first) = seen_apps.get(&key) {
            c.issue(&[first, &node.id], &[], tr!(
                "issue_app_repeated",
                app = app.name
            ));
            continue;
        }
        seen_apps.insert(key, &node.id);
        routes.push(AppRoute {
            node: node.id.clone(),
            app: app.clone(),
            render,
            capture,
        });
    }

    // engine streams: links without apps. Sound is gathered for every output device
    let engine_edges: Vec<&GraphEdge> = edges
        .iter()
        .copied()
        .filter(|e| !c.is_routed(&e.source) && !c.is_routed(&e.target))
        .collect();
    c.place_cables(&graph.nodes, &edges);
    // what is mixed per output device
    let mut streams: BTreeMap<String, Mix> = BTreeMap::new();
    let mut outputs = BTreeSet::new();
    for node in &graph.nodes {
        let is_sink = matches!(c.role(node), Role::Output(_) | Role::Cable(_) | Role::Deleted);
        if !is_sink || !engine_edges.iter().any(|e| e.target == node.id) {
            continue;
        }
        // an output delays the sound it plays; a cable the sound going into it if it holds it back there
        let delay = match c.role(node) {
            Role::Output(_) => c.delay(node),
            Role::Cable(_) if c.sides.get(node.id.as_str()) == Some(&Side::Into) => c.delay(node),
            _ => 0.0,
        };
        let mut mix = Mix::default();
        // a cable working on its way in takes everything played into it as one sound
        let stage = if c.sides.get(node.id.as_str()) == Some(&Side::Into) {
            mix.add_stage(0, &node.id, &node.id, c.gain(node), c.gate(node))
        } else {
            0
        };
        c.collect_sources(node, &engine_edges, &mut HashSet::new(), stage, delay, &mut mix);
        let Some(sink) = c.sink_device(node) else { continue };
        // a cable into itself: its input and output are one pair, sound would go round in circles.
        // It also happens implicitly: "default" is now the cable's input because "System" set it,
        // or the system has no other output device and took the cable
        if let Some(cable) = cables.iter().find(|cb| device_id(&cb.render) == Some(sink.as_str())) {
            if let Some(capture) = cable_source(cable).filter(|id| mix.sources.iter().any(|s| s.device == *id)) {
                mix.sources.retain(|s| s.device != capture);
                let into: Vec<&str> = engine_edges.iter().filter(|e| e.target == node.id).map(|e| e.id.as_str()).collect();
                let message = if matches!(c.role(node), Role::Output(Some(DeviceRef::Default))) {
                    tr!("issue_default_output_loop", cable = cable.name)
                } else {
                    tr!("issue_cable_into_itself", cable = cable.name)
                };
                c.issue(&[&node.id], &into, message);
            }
        }
        if matches!(c.role(node), Role::Output(_)) {
            outputs.insert(sink.clone());
        }
        streams.entry(sink).or_default().absorb(mix);
    }

    let looped = cut_device_loops(&mut streams, cables);
    for (source, sink) in &looped {
        for node in graph.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Output | NodeKind::Cable)) {
            let mut links = Vec::new();
            for edge in engine_edges.iter().filter(|e| e.source == node.id) {
                let target = c.nodes[edge.target.as_str()];
                // an internal cable has no device: the sound may go on from it
                let leads = matches!(c.role(target), Role::Bus) || c.sink_device(target).as_ref() == Some(sink);
                if leads && c.source_device(node).as_ref() == Some(source) {
                    links.push(edge.id.as_str());
                }
            }
            if !links.is_empty() {
                c.issue(&[&node.id], &links, tr!("issue_loop"));
            }
        }
    }

    Plan {
        routes,
        system,
        streams: into_streams(streams, &outputs),
        issues: c.issues,
        virtual_cables: Vec::new(),
    }
}

fn into_streams(streams: BTreeMap<String, Mix>, outputs: &BTreeSet<String>) -> Vec<Stream> {
    streams
        .into_iter()
        .filter(|(_, mix)| !mix.sources.is_empty())
        .map(|(sink, Mix { stages, mut sources })| {
            sources.sort_by(|a, b| a.device.cmp(&b.device).then(a.delay.total_cmp(&b.delay)).then(a.stage.cmp(&b.stage)));
            Stream { output: outputs.contains(&sink), stages, sources, sink }
        })
        .collect()
}

/// Delays of running graphs as the engine plays them (compile has applied each graph's range).
/// An output's minus means earlier than the rest, and since sound can't play earlier, the rest of
/// the outputs of running graphs are delayed by as much instead. A link holds at most MAX_DELAY,
/// more added up along internal cables is cut. The first running plans go to the engine
pub fn apply_delays(plans: &mut [Plan], running: usize) {
    let count = running.min(plans.len());
    let running = &mut plans[..count];
    let outputs = running.iter().flat_map(|p| &p.streams).filter(|s| s.output);
    let earliest = outputs.flat_map(|s| &s.sources).map(|s| s.delay).fold(0.0, f32::min);
    for stream in running.iter_mut().flat_map(|p| &mut p.streams) {
        let shift = if stream.output { earliest } else { 0.0 };
        for source in &mut stream.sources {
            source.delay = (source.delay - shift).min(MAX_DELAY);
        }
    }
}

/// Several graphs at once, in start order. Streams into one device are mixed, and what the
/// system has only one of (an app's devices, the default devices) goes to the graph
/// started earlier; for the others it is a problem. A stopped graph open in the window
/// goes last: this shows what it will run into if started
pub fn compile_all(graphs: &[&Graph], cables: &[Cable], defaults: &Defaults) -> Vec<Plan> {
    let mut defaults = defaults.clone();
    let mut app_owner: HashMap<String, &str> = HashMap::new();
    let mut system_owner: Option<&str> = None;
    // sound between devices: recording device → output device (streams of accepted graphs),
    // a cable's input → its output. A cycle in this network is a loop through several graphs
    let mut links = cable_links(cables);
    let mut plans = Vec::new();
    for (i, graph) in graphs.iter().enumerate() {
        let mut plan = compile(graph, cables, &defaults);
        let mut issues = Vec::new();

        plan.routes.retain(|route| {
            let key = route.app.exe.to_lowercase();
            if let Some(owner) = app_owner.get(&key) {
                issues.push(Issue {
                    nodes: vec![route.node.clone()],
                    edges: Vec::new(),
                    message: tr!(
                        "issue_app_taken",
                        app = route.app.name,
                        graph = owner
                    ),
                });
                return false;
            }
            app_owner.insert(key, &graph.name);
            true
        });

        if let Some(route) = &plan.system {
            if let Some(owner) = system_owner {
                issues.push(Issue {
                    nodes: vec![route.node.clone()],
                    edges: Vec::new(),
                    message: tr!(
                        "issue_defaults_taken",
                        graph = owner
                    ),
                });
                plan.system = None;
            } else {
                system_owner = Some(&graph.name);
                // in the graphs after this one "default" means the replacement devices
                if let Some(id) = &route.render {
                    defaults.render = Some(id.clone());
                }
                if let Some(id) = &route.capture {
                    defaults.capture = Some(id.clone());
                }
            }
        }

        // loops inside a graph are already removed, so only graphs after the first need checking
        let mut looped = false;
        for stream in &mut plan.streams {
            for s in &stream.sources {
                link_loopback(&mut links, &s.device);
            }
            if i > 0 {
                stream.sources.retain(|s| {
                    let closes = reaches(&links, &stream.sink, &s.device);
                    looped |= closes;
                    !closes
                });
            }
            for s in &stream.sources {
                links.entry(s.device.clone()).or_default().insert(stream.sink.clone());
            }
        }
        plan.streams.retain(|s| !s.sources.is_empty());
        if looped {
            issues.push(Issue {
                nodes: Vec::new(),
                edges: Vec::new(),
                message: tr!(
                    "issue_loop_across_graphs"
                ),
            });
        }

        plan.issues.extend(issues);
        plans.push(plan);
    }
    plans
}

/// Whether sound has a path from device from to device to
fn reaches(links: &DeviceLinks, from: &str, to: &str) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(device) = stack.pop() {
        if device == to {
            return true;
        }
        if seen.insert(device) {
            stack.extend(links.get(device).into_iter().flatten().map(String::as_str));
        }
    }
    false
}

/// One plan for the engine from the plans of running graphs (after compile_all)
pub fn merge(plans: &[Plan]) -> Plan {
    let mut streams: BTreeMap<String, Mix> = BTreeMap::new();
    let mut outputs = BTreeSet::new();
    for stream in plans.iter().flat_map(|p| &p.streams) {
        if stream.output {
            outputs.insert(stream.sink.clone());
        }
        let mix = Mix { stages: stream.stages.clone(), sources: stream.sources.clone() };
        streams.entry(stream.sink.clone()).or_default().absorb(mix);
    }
    Plan {
        routes: plans.iter().flat_map(|p| p.routes.iter().cloned()).collect(),
        system: plans.iter().find_map(|p| p.system.clone()),
        streams: into_streams(streams, &outputs),
        issues: plans.iter().flat_map(|p| p.issues.iter().cloned()).collect(),
        virtual_cables: plans.iter().flat_map(|p| p.virtual_cables.iter().cloned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphs::{GraphEdge, Position};

    fn dev(id: &str) -> Option<DeviceRef> {
        Some(DeviceRef::Device {
            id: id.into(),
            name: id.into(),
        })
    }

    fn node(id: &str, kind: NodeKind) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind,
            label: id.into(),
            sink: None,
            source: None,
            cable: None,
            app: None,
            volume: 1.0,
            system_volume: None,
            gate: NO_GATE,
            delay: 0.0,
            balance: 0.0,
            pinned: Vec::new(),
            position: Position { x: 0.0, y: 0.0 },
        }
    }
    fn input(id: &str, device: Option<DeviceRef>) -> GraphNode {
        GraphNode { source: device, ..node(id, NodeKind::Input) }
    }
    fn output(id: &str, device: Option<DeviceRef>) -> GraphNode {
        GraphNode { sink: device, ..node(id, NodeKind::Output) }
    }
    fn app(id: &str, exe: &str) -> GraphNode {
        GraphNode {
            app: Some(AppRef {
                exe: exe.into(),
                path: String::new(),
                name: exe.into(),
            }),
            ..node(id, NodeKind::App)
        }
    }
    fn system(id: &str) -> GraphNode {
        node(id, NodeKind::System)
    }
    fn cable_node(id: &str, cable: Option<&str>) -> GraphNode {
        GraphNode { cable: cable.map(Into::into), ..node(id, NodeKind::Cable) }
    }
    fn cable(id: &str, render: &str, capture: &str) -> Cable {
        Cable {
            id: id.into(),
            name: id.into(),
            render: dev(render),
            capture: dev(capture),
            created_at: 0,
        }
    }
    fn graph(nodes: Vec<GraphNode>, edges: &[(&str, &str)]) -> Graph {
        Graph {
            version: 1,
            id: "g".into(),
            name: "g".into(),
            created_at: 0,
            updated_at: 0,
            viewport: None,
            nodes,
            edges: edges
                .iter()
                .map(|(s, t)| GraphEdge {
                    id: format!("{s}->{t}"),
                    source: s.to_string(),
                    target: t.to_string(),
                })
                .collect(),
            show_delay: None,
            delay_limit: 0.5,
            volume_limit: 2.0,
            start_on_launch: false,
            show_balance: None,
        }
    }
    fn defaults() -> Defaults {
        Defaults {
            render: Some("spk".into()),
            capture: Some("mic".into()),
        }
    }
    /// What a stream carries, whatever the stages on the way: is it an output's, the device, what comes to it
    type Route = (bool, String, Vec<String>);

    /// In these tests cable inputs end with ".in" or are virtual devices, the rest are output nodes' devices
    fn stream(sink: &str, sources: &[&str]) -> Route {
        (!sink.ends_with(".in") && !sink.starts_with(VIRTUAL_PREFIX), sink.into(), sources.iter().map(|s| s.to_string()).collect())
    }

    fn routes(streams: &[Stream]) -> Vec<Route> {
        streams.iter().map(|s| (s.output, s.sink.clone(), s.sources.iter().map(|x| x.device.clone()).collect())).collect()
    }

    #[test]
    fn example_graph() {
        // like the example in the app: microphone → voice → Discord, Spotify → music → headphones and Discord
        let g = graph(
            vec![
                input("mic", Some(DeviceRef::Default)),
                app("spotify", "Spotify.exe"),
                cable_node("voice", Some("v")),
                cable_node("music", Some("m")),
                app("discord", "Discord.exe"),
                output("phones", dev("hp")),
            ],
            &[("mic", "voice"), ("voice", "discord"), ("spotify", "music"), ("music", "phones"), ("music", "discord")],
        );
        let cables = [cable("v", "v.in", "v.out"), cable("m", "m.in", "m.out")];
        let plan = compile(&g, &cables, &defaults());
        // Discord has two inputs: that is an error, the rest works
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["discord"]);
        assert_eq!(plan.routes.len(), 1);
        assert_eq!(plan.routes[0].node, "spotify");
        assert_eq!(plan.routes[0].render, Some(Target::Device("m.in".into())));
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["m.out"]), stream("v.in", &["mic"])]);
    }

    #[test]
    fn app_follows_default_output() {
        let g = graph(vec![app("a", "a.exe"), output("o", Some(DeviceRef::Default))], &[("a", "o")]);
        let plan = compile(&g, &[], &defaults());
        assert_eq!(plan.routes[0].render, Some(Target::Default));
        assert!(plan.streams.is_empty());
    }

    #[test]
    fn bus_mixes_sources() {
        // two microphones into an internal cable, from it to the headphones and to a cable
        let g = graph(
            vec![
                input("m1", dev("mic1")),
                input("m2", dev("mic2")),
                cable_node("bus", None),
                output("o", dev("hp")),
                cable_node("c", Some("c")),
            ],
            &[("m1", "bus"), ("m2", "bus"), ("bus", "o"), ("bus", "c")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(routes(&plan.streams), vec![stream("c.in", &["mic1", "mic2"]), stream("hp", &["mic1", "mic2"])]);
    }

    #[test]
    fn app_into_bus_is_an_issue() {
        let g = graph(
            vec![app("a", "a.exe"), cable_node("bus", None), output("o", dev("hp"))],
            &[("a", "bus"), ("bus", "o")],
        );
        let plan = compile(&g, &[], &defaults());
        assert_eq!(plan.issues.len(), 1);
        assert_eq!(plan.issues[0].edges, vec!["a->bus"]);
        assert!(plan.routes.is_empty() && plan.streams.is_empty());
    }

    #[test]
    fn loops_are_skipped() {
        let g = graph(
            vec![
                cable_node("c1", Some("c1")),
                cable_node("c2", Some("c2")),
                input("mic", dev("mic")),
                output("o", dev("hp")),
            ],
            &[("c1", "c2"), ("c2", "c1"), ("mic", "o")],
        );
        let cables = [cable("c1", "c1.in", "c1.out"), cable("c2", "c2.in", "c2.out")];
        let plan = compile(&g, &cables, &defaults());
        assert_eq!(plan.issues.len(), 1);
        assert_eq!(plan.issues[0].edges.len(), 2);
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["mic"])]);
    }

    #[test]
    fn missing_pieces() {
        let g = graph(
            vec![input("i", None), output("o", dev("hp")), cable_node("gone", Some("deleted")), node("a", NodeKind::App)],
            &[("i", "o"), ("gone", "o"), ("a", "o")],
        );
        let plan = compile(&g, &[], &defaults());
        let messages: Vec<&str> = plan.issues.iter().map(|i| i.message.as_str()).collect();
        assert_eq!(messages, vec!["No app selected", "No input device selected", "The cable was deleted from the cable list"]);
        assert!(plan.streams.is_empty());
    }

    #[test]
    fn same_app_twice() {
        let g = graph(
            vec![app("a1", "Discord.exe"), app("a2", "discord.exe"), output("o", dev("hp"))],
            &[("a1", "o"), ("a2", "o")],
        );
        let plan = compile(&g, &[], &defaults());
        assert_eq!(plan.routes.len(), 1);
        assert_eq!(plan.issues[0].nodes, vec!["a1", "a2"]);
    }

    #[test]
    fn system_defaults_to_cables() {
        // everything playing to the default goes into cable "Music", the default microphone is the output of cable "Voice";
        // Spotify → "default" follows the system, so it goes into "Music" too
        let g = graph(
            vec![
                system("sys"),
                cable_node("music", Some("m")),
                cable_node("voice", Some("v")),
                output("phones", dev("hp")),
                output("default", Some(DeviceRef::Default)),
                input("mic", dev("mic")),
                app("spotify", "Spotify.exe"),
            ],
            &[("sys", "music"), ("music", "phones"), ("mic", "voice"), ("voice", "sys"), ("spotify", "default")],
        );
        let cables = [cable("m", "m.in", "m.out"), cable("v", "v.in", "v.out")];
        let plan = compile(&g, &cables, &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(
            plan.system,
            Some(SystemRoute {
                node: "sys".into(),
                render: Some("m.in".into()),
                capture: Some("v.out".into()),
            })
        );
        assert_eq!(plan.routes[0].render, Some(Target::Default));
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["m.out"]), stream("v.in", &["mic"])]);
    }

    #[test]
    fn default_output_replaced_by_cable_is_a_loop() {
        // "System → cable → default output": the default is now this same cable's input
        let g = graph(
            vec![system("sys"), cable_node("c", Some("c")), output("default", Some(DeviceRef::Default)), output("hp", dev("hp"))],
            &[("sys", "c"), ("c", "default"), ("c", "hp")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["default"]);
        assert!(plan.issues[0].message.starts_with("Loop: the default output device"));
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["c.out"])]);
    }

    #[test]
    fn system_with_app_needs_cable() {
        let g = graph(vec![system("sys"), app("a", "a.exe"), system("sys2"), output("o", dev("hp"))], &[("sys", "a"), ("sys2", "o")]);
        let plan = compile(&g, &[], &defaults());
        let messages: Vec<&str> = plan.issues.iter().map(|i| i.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].starts_with("These can't be connected directly"));
        assert!(messages[1].starts_with("The “System” node appears in the graph more than once"));
    }

    #[test]
    fn only_cables_change_the_volume_on_the_way() {
        // the sliders of devices are the system's. An internal cable's 50% and the cables' 80% and 30% count,
        // the last on the way into it, so whoever listens to that cable hears it too
        let vol = |mut n: GraphNode, v: f32| {
            n.volume = v;
            n
        };
        let g = graph(
            vec![
                vol(input("mic", dev("mic")), 0.2),
                vol(cable_node("bus", None), 0.5),
                vol(output("o", dev("hp")), 0.3),
                vol(cable_node("c", Some("c")), 0.8),
                vol(cable_node("rec", Some("r")), 0.3),
            ],
            &[("mic", "bus"), ("bus", "o"), ("c", "o"), ("mic", "rec")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out"), cable("r", "r.in", "r.out")], &defaults());
        let gains: Vec<_> = plan
            .streams
            .iter()
            .flat_map(|s| s.sources.iter().map(move |src| (s.sink.as_str(), src.device.as_str(), s.gain_of(src)[0])))
            .collect();
        assert_eq!(gains, vec![("hp", "c.out", 0.8), ("hp", "mic", 0.5), ("r.in", "mic", 0.3)]);
    }

    #[test]
    fn balance_is_a_cables_and_multiplies_per_channel() {
        // a cable at 80% leaning left (-0.5) → one leaning right (0.25); what devices kept from before has no effect
        let first = GraphNode { volume: 0.8, balance: -0.5, ..cable_node("a", None) };
        let second = GraphNode { balance: 0.25, ..cable_node("b", None) };
        let g = graph(
            vec![
                GraphNode { balance: -1.0, ..input("mic", dev("mic")) },
                first,
                second,
                GraphNode { balance: 1.0, ..output("o", dev("hp")) },
            ],
            &[("mic", "a"), ("a", "b"), ("b", "o")],
        );
        let plan = compile(&g, &[], &defaults());
        assert_eq!(plan.streams[0].gain_of(&plan.streams[0].sources[0]), [0.8 * 0.75, 0.8 * 0.5]);
    }

    #[test]
    fn volume_limit_cuts_cables() {
        let mut g = graph(
            vec![input("mic", dev("mic")), GraphNode { volume: 1.8, ..cable_node("c", None) }, output("o", dev("hp"))],
            &[("mic", "c"), ("c", "o")],
        );
        let gain = |g: &Graph| {
            let plan = compile(g, &[], &defaults());
            plan.streams[0].gain_of(&plan.streams[0].sources[0])[0]
        };
        assert_eq!(gain(&g), 1.8);
        g.volume_limit = 0.5;
        assert_eq!(gain(&g), 0.5);
        // and it can reach above the default maximum
        g.nodes[1].volume = 4.0;
        g.volume_limit = 5.0;
        assert_eq!(gain(&g), 4.0);
    }

    /// The system applies an output's slider itself, so what plays straight into it has no problem with it
    #[test]
    fn straight_links_take_the_outputs_volume() {
        for source in [system("s"), app("a", "a.exe")] {
            let id = source.id.clone();
            let target = GraphNode { volume: 0.5, balance: 1.0, ..output("o", dev("hp")) };
            let g = graph(vec![source, target], &[(id.as_str(), "o")]);
            let plan = compile(&g, &[], &defaults());
            assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        }
    }

    fn named(mut g: Graph, id: &str) -> Graph {
        g.id = id.into();
        g.name = id.into();
        g
    }

    #[test]
    fn graphs_share_devices_and_mix() {
        // two graphs play into the same headphones: the streams are mixed
        let a = named(graph(vec![input("mic", dev("mic")), output("o", dev("hp"))], &[("mic", "o")]), "a");
        let b = named(graph(vec![cable_node("c", Some("c")), output("o2", dev("hp"))], &[("c", "o2")]), "b");
        let cables = [cable("c", "c.in", "c.out")];
        let plans = compile_all(&[&a, &b], &cables, &defaults());
        assert!(plans.iter().all(|p| p.issues.is_empty()), "{plans:?}");
        assert_eq!(routes(&merge(&plans).streams), vec![stream("hp", &["c.out", "mic"])]);
    }

    #[test]
    fn first_graph_owns_app_and_system() {
        let a = named(
            graph(
                vec![app("x", "x.exe"), cable_node("c", Some("c")), system("s"), cable_node("d", Some("d"))],
                &[("x", "c"), ("s", "d")],
            ),
            "a",
        );
        let b = named(
            graph(
                vec![app("x2", "X.exe"), output("o", dev("hp")), system("s2"), output("o2", dev("spk"))],
                &[("x2", "o"), ("s2", "o2")],
            ),
            "b",
        );
        let cables = [cable("c", "c.in", "c.out"), cable("d", "d.in", "d.out")];
        let plans = compile_all(&[&a, &b], &cables, &defaults());
        assert!(plans[0].issues.is_empty());
        let messages: Vec<&str> = plans[1].issues.iter().map(|i| i.message.as_str()).collect();
        assert!(messages[0].starts_with("“X.exe” is already routed by the running graph “a”"), "{messages:?}");
        assert!(messages[1].starts_with("The default devices are already set by"), "{messages:?}");
        let plan = merge(&plans);
        assert_eq!(plan.routes.len(), 1);
        assert_eq!(plan.system.unwrap().node, "s");
    }

    #[test]
    fn loop_across_graphs_is_cut_in_later_graph() {
        // a: cable c → cable d, b: cable d → cable c: together sound would go round in circles
        let cables = [cable("c", "c.in", "c.out"), cable("d", "d.in", "d.out")];
        let a = named(graph(vec![cable_node("c", Some("c")), cable_node("d", Some("d"))], &[("c", "d")]), "a");
        let b = named(graph(vec![cable_node("d", Some("d")), cable_node("c", Some("c"))], &[("d", "c")]), "b");
        let plans = compile_all(&[&a, &b], &cables, &defaults());
        assert!(plans[0].issues.is_empty());
        assert!(plans[1].issues[0].message.starts_with("Loop with another running graph"));
        assert_eq!(routes(&merge(&plans).streams), vec![stream("d.in", &["c.out"])]);
    }

    #[test]
    fn virtual_cables_are_devices() {
        // as on Linux: an app into a cable, the cable into the headphones and into a listening app
        let g = graph(
            vec![
                app("player", "player"),
                cable_node("mix", None),
                output("phones", dev("hp")),
                app("call", "call"),
                input("mic", dev("mic")),
            ],
            &[("player", "mix"), ("mix", "phones"), ("mix", "call"), ("mic", "mix")],
        );
        let (g, cables, devices) = virtualize(&g);
        let (sink, source) = virtual_device_names("mix");
        assert_eq!((sink.as_str(), source.as_str()), ("audiograph.mix", "audiograph.mix.out"));
        assert_eq!(devices, vec![VirtualCable { node: "mix".into(), label: "mix".into(), sink: sink.clone(), source: source.clone() }]);
        let plan = compile(&g, &cables, &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        let route = |node: &str| plan.routes.iter().find(|r| r.node == node).unwrap();
        assert_eq!(route("player").render, Some(Target::Device(sink.clone())));
        assert_eq!(route("call").capture, Some(Target::Device(source.clone())));
        assert_eq!(routes(&plan.streams), vec![stream("audiograph.mix", &["mic"]), stream("hp", &["audiograph.mix.out"])]);
    }

    fn delayed(node: GraphNode, delay: f32) -> GraphNode {
        GraphNode { delay, ..node }
    }

    /// The graph with the delay range ±limit
    fn with_range(mut g: Graph, limit: f32) -> Graph {
        g.delay_limit = limit;
        g
    }

    /// The delay of each output device's first source, after apply_delays
    fn delays(plan: &Plan) -> Vec<(&str, f32)> {
        plan.streams.iter().map(|s| (s.sink.as_str(), (s.sources[0].delay * 1000.0).round() / 1000.0)).collect()
    }

    /// Compiles one running graph and applies its delays
    fn delays_of(g: &Graph, cables: &[Cable]) -> Plan {
        let mut plans = compile_all(&[g], cables, &defaults());
        apply_delays(&mut plans, 1);
        plans.remove(0)
    }

    fn gated(node: GraphNode, gate: f32) -> GraphNode {
        GraphNode { gate, ..node }
    }

    /// The stages a stream's source goes through, from the first to hear it, as (node, gate)
    fn way(stream: &Stream, source: &Source) -> Vec<(String, f32)> {
        let mut way = Vec::new();
        let mut stage = source.stage;
        while stage != 0 {
            way.push((stream.stages[stage].node.clone(), stream.stages[stage].gate));
            stage = stream.stages[stage].parent;
        }
        way
    }

    #[test]
    fn a_cable_works_on_the_sum_of_what_comes_to_it() {
        // an internal cable: both devices are in one stage, the gate hears them together
        let g = graph(
            vec![input("mic", dev("mic")), input("line", dev("line")), gated(cable_node("bus", None), -40.0), output("hp", dev("hp"))],
            &[("mic", "bus"), ("line", "bus"), ("bus", "hp")],
        );
        let plan = compile(&g, &[], &defaults());
        let [line, mic] = plan.streams[0].sources.as_slice() else { panic!() };
        assert_eq!(line.stage, mic.stage);
        assert_eq!(way(&plan.streams[0], mic), vec![("bus".to_string(), -40.0)]);

        // into a cable from the list that an app listens to: the usual way to a call. Its volume acts too
        let g = graph(
            vec![
                input("mic", dev("mic")),
                input("line", dev("line")),
                GraphNode { volume: 0.5, ..gated(cable_node("c", Some("c")), -40.0) },
                app("a", "a.exe"),
            ],
            &[("mic", "c"), ("line", "c"), ("c", "a")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        let stream = &plan.streams[0];
        assert_eq!(stream.sink, "c.in");
        assert_eq!(stream.sources[0].stage, stream.sources[1].stage);
        assert_eq!(way(stream, &stream.sources[0]), vec![("c".to_string(), -40.0)]);
        assert_eq!(stream.gain_of(&stream.sources[0]), [0.5; 2]);

        // apps play into the cable: the cable's own sum is on the way out of it
        let g = graph(
            vec![system("s"), GraphNode { volume: 0.5, ..gated(cable_node("c", Some("c")), -40.0) }, output("hp", dev("hp"))],
            &[("s", "c"), ("c", "hp")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        let stream = &plan.streams[0];
        assert_eq!(way(stream, &stream.sources[0]), vec![("c".to_string(), -40.0)]);
        assert_eq!(stream.gain_of(&stream.sources[0]), [0.5; 2]);
    }

    #[test]
    fn a_cable_from_the_list_works_once_per_way() {
        // through two cables each has its stage in one stream only
        let g = graph(
            vec![
                input("mic", dev("mic")),
                GraphNode { volume: 0.5, ..cable_node("a", Some("a")) },
                GraphNode { volume: 0.5, ..cable_node("b", Some("b")) },
                output("hp", dev("hp")),
            ],
            &[("mic", "a"), ("a", "b"), ("b", "hp")],
        );
        let plan = compile(&g, &[cable("a", "a.in", "a.out"), cable("b", "b.in", "b.out")], &defaults());
        let gains: Vec<(&str, Gain)> = plan.streams.iter().map(|s| (s.sink.as_str(), s.gain_of(&s.sources[0]))).collect();
        assert_eq!(gains, vec![("a.in", [0.5; 2]), ("b.in", [0.5; 2]), ("hp", [1.0; 2])]);
    }

    #[test]
    fn cables_in_a_row_work_one_after_the_other() {
        let g = graph(
            vec![input("mic", dev("mic")), gated(cable_node("a", None), -60.0), gated(cable_node("b", None), -30.0), output("hp", dev("hp"))],
            &[("mic", "a"), ("a", "b"), ("b", "hp")],
        );
        let plan = compile(&g, &[], &defaults());
        let stream = &plan.streams[0];
        assert_eq!(way(stream, &stream.sources[0]), vec![("a".to_string(), -60.0), ("b".to_string(), -30.0)]);
    }

    #[test]
    fn a_gate_stays_on_its_own_path() {
        let g = graph(
            vec![input("mic", dev("mic")), gated(cable_node("a", None), -30.0), output("hp", dev("hp"))],
            &[("mic", "a"), ("a", "hp"), ("mic", "hp")],
        );
        let plan = compile(&g, &[], &defaults());
        let stream = &plan.streams[0];
        let mut ways: Vec<Vec<(String, f32)>> = stream.sources.iter().map(|s| way(stream, s)).collect();
        ways.sort_by_key(|w| w.len());
        assert_eq!(ways, vec![vec![], vec![("a".to_string(), -30.0)]]);
    }

    #[test]
    fn a_stage_keeps_its_id_and_two_paths_have_two() {
        let g = graph(
            vec![input("mic", dev("mic")), cable_node("a", None), cable_node("b", None), cable_node("c", None), output("hp", dev("hp"))],
            &[("mic", "a"), ("a", "b"), ("a", "c"), ("b", "hp"), ("c", "hp")],
        );
        let plan = compile(&g, &[], &defaults());
        let ids: Vec<&str> = plan.streams[0].stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len(), "{ids:?}");
        assert_eq!(plan, compile(&g, &[], &defaults()));
    }

    #[test]
    fn merged_graphs_keep_their_stages_apart() {
        let a = graph(vec![input("m1", dev("m1")), gated(cable_node("a", None), -10.0), output("o1", dev("hp"))], &[("m1", "a"), ("a", "o1")]);
        let b = named(graph(vec![input("m2", dev("m2")), gated(cable_node("b", None), -20.0), output("o2", dev("hp"))], &[("m2", "b"), ("b", "o2")]), "b");
        let merged = merge(&compile_all(&[&a, &b], &[], &defaults()));
        let stream = &merged.streams[0];
        assert_eq!(way(stream, &stream.sources[0]), vec![("a".to_string(), -10.0)]);
        assert_eq!(way(stream, &stream.sources[1]), vec![("b".to_string(), -20.0)]);
    }

    #[test]
    fn a_gate_cant_act_on_what_only_the_system_carries() {
        let g = graph(
            vec![system("s"), gated(cable_node("c", Some("c")), -30.0), app("a", "a.exe")],
            &[("s", "c"), ("c", "a")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes[0], "c");
    }

    #[test]
    fn a_volume_cant_act_on_what_only_the_system_carries() {
        let g = graph(
            vec![system("s"), GraphNode { volume: 0.5, ..cable_node("c", Some("c")) }, app("a", "a.exe")],
            &[("s", "c"), ("c", "a")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes[0], "c");
    }

    #[test]
    fn a_cable_without_an_input_still_works_on_its_way_out() {
        let bare = Cable { render: None, ..cable("c", "", "c.out") };
        let g = graph(
            vec![input("mic", dev("mic")), GraphNode { volume: 0.5, ..cable_node("c", Some("c")) }, output("hp", dev("hp"))],
            &[("mic", "c"), ("c", "hp")],
        );
        let plan = compile(&g, &[bare], &defaults());
        assert_eq!(plan.streams[0].gain_of(&plan.streams[0].sources[0]), [0.5; 2]);
    }

    #[test]
    fn output_delays_are_relative() {
        // the microphone into the speakers earlier than into the headphones
        let g = graph(
            vec![input("mic", dev("mic")), delayed(output("hp", dev("hp")), -0.1), delayed(output("spk", dev("spk")), -0.25)],
            &[("mic", "hp"), ("mic", "spk")],
        );
        // sound can't play earlier: the speakers get none, the headphones what is left
        assert_eq!(delays(&delays_of(&with_range(g.clone(), 0.5), &[])), vec![("hp", 0.15), ("spk", 0.0)]);
        // the range cuts what is beyond it
        assert_eq!(delays(&delays_of(&with_range(g, 0.125), &[])), vec![("hp", 0.025), ("spk", 0.0)]);
    }

    #[test]
    fn a_plus_on_a_device_counts_as_nothing() {
        let g = graph(vec![input("mic", dev("mic")), delayed(output("hp", dev("hp")), 0.25)], &[("mic", "hp")]);
        assert_eq!(delays(&delays_of(&with_range(g, 0.5), &[])), vec![("hp", 0.0)]);
    }

    #[test]
    fn an_input_has_no_delay() {
        // files of earlier versions may hold one: it does nothing, to an app either
        let g = graph(
            vec![delayed(input("a", dev("a")), -0.25), input("b", dev("b")), output("hp", dev("hp")), app("call", "call.exe")],
            &[("a", "hp"), ("b", "hp"), ("a", "call")],
        );
        let plan = delays_of(&with_range(g, 0.5), &[]);
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        let sources: Vec<(&str, f32)> = plan.streams[0].sources.iter().map(|s| (s.device.as_str(), s.delay)).collect();
        assert_eq!(sources, vec![("a", 0.0), ("b", 0.0)]);
    }

    #[test]
    fn delays_dont_go_into_cables() {
        // the microphone through a cable to the only output, set earlier: there is nothing to be earlier
        // than, and the link into the cable isn't delayed either, or the output would be late after all
        let g = graph(
            vec![input("mic", dev("mic")), cable_node("c", Some("c")), delayed(output("hp", dev("hp")), -0.4)],
            &[("mic", "c"), ("c", "hp")],
        );
        let plan = delays_of(&with_range(g, 0.5), &[cable("c", "c.in", "c.out")]);
        assert_eq!(delays(&plan), vec![("c.in", 0.0), ("hp", 0.0)]);
    }

    #[test]
    fn only_running_graphs_shift_each_other() {
        let running = named(graph(vec![input("mic", dev("mic")), output("hp", dev("hp"))], &[("mic", "hp")]), "a");
        let stopped = named(graph(vec![input("mic", dev("mic")), delayed(output("spk", dev("spk")), -0.3)], &[("mic", "spk")]), "b");
        let (running, stopped) = (with_range(running, 0.5), with_range(stopped, 0.5));
        let mut plans = compile_all(&[&running, &stopped], &[], &defaults());
        apply_delays(&mut plans, 1);
        assert_eq!(delays(&plans[0]), vec![("hp", 0.0)]);
    }

    #[test]
    fn app_straight_into_delayed_output() {
        // the system plays the app's sound, the engine can't hold it back
        let g = graph(vec![app("a", "a.exe"), delayed(output("hp", dev("hp")), -0.2)], &[("a", "hp")]);
        let plan = delays_of(&g, &[]);
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["hp", "a"]);
    }

    #[test]
    fn internal_cable_delays_add_up_on_the_way() {
        // the microphone through two internal cables to the headphones: their delays add up
        let g = graph(
            vec![
                input("mic", dev("mic")),
                delayed(cable_node("a", None), 0.2),
                delayed(cable_node("b", None), 0.1),
                output("hp", dev("hp")),
            ],
            &[("mic", "a"), ("a", "b"), ("b", "hp")],
        );
        assert_eq!(delays(&delays_of(&with_range(g.clone(), 0.5), &[])), vec![("hp", 0.3)]);
        // the range cuts each delay, not their sum
        assert_eq!(delays(&delays_of(&with_range(g, 0.15), &[])), vec![("hp", 0.25)]);
    }

    #[test]
    fn cable_delay_only_holds_sound_back() {
        // a cable can't send sound earlier: its minus is no delay
        let g = graph(
            vec![input("mic", dev("mic")), delayed(cable_node("a", None), -0.3), output("hp", dev("hp"))],
            &[("mic", "a"), ("a", "hp")],
        );
        assert_eq!(delays(&delays_of(&with_range(g, 0.5), &[])), vec![("hp", 0.0)]);
    }

    /// Volume and delay of each source of the only stream
    fn layers(plan: &Plan) -> Vec<(&str, Gain, f32)> {
        let stream = &plan.streams[0];
        stream.sources.iter().map(|s| (s.device.as_str(), stream.gain_of(s), (s.delay * 1000.0).round() / 1000.0)).collect()
    }

    /// A cable output's sound through two internal cables, each with its balance and delay, to two
    /// output nodes of one device
    fn two_paths(first: (f32, f32), second: (f32, f32)) -> Graph {
        let path = |id: &str, (balance, delay): (f32, f32)| GraphNode { balance, delay, ..cable_node(id, None) };
        graph(
            vec![
                input("mic", dev("mic")),
                path("a", first),
                path("b", second),
                output("o1", dev("hp")),
                output("o2", dev("hp")),
            ],
            &[("mic", "a"), ("mic", "b"), ("a", "o1"), ("b", "o2")],
        )
    }

    #[test]
    fn paths_of_one_device_delayed_differently_are_layers() {
        // the left channel at once, the right one half a second later
        let plan = delays_of(&two_paths((-1.0, 0.0), (1.0, 0.5)), &[]);
        assert_eq!(layers(&plan), vec![("mic", [1.0, 0.0], 0.0), ("mic", [0.0, 1.0], 0.5)]);
        let numbers: Vec<usize> = plan.streams[0].layers().map(|(_, n)| n).collect();
        assert_eq!(numbers, vec![0, 1]);
    }

    #[test]
    fn paths_of_one_device_with_one_delay_add_up() {
        // two wires from one microphone: each path plays with its own balance
        let plan = delays_of(&two_paths((-1.0, 0.2), (1.0, 0.2)), &[]);
        assert_eq!(layers(&plan), vec![("mic", [1.0, 0.0], 0.2), ("mic", [0.0, 1.0], 0.2)]);
    }

    #[test]
    fn layers_shift_with_the_other_outputs_and_survive_merging() {
        let a = two_paths((-1.0, 0.0), (1.0, 0.1));
        let mut b = graph(vec![input("m2", dev("m2")), delayed(output("spk", dev("spk")), -0.2)], &[("m2", "spk")]);
        b = named(with_range(b, 0.5), "b");
        let mut plans = compile_all(&[&a, &b], &[], &defaults());
        apply_delays(&mut plans, 2);
        assert_eq!(layers(&merge(&plans)), vec![("mic", [1.0, 0.0], 0.2), ("mic", [0.0, 1.0], 0.3)]);
    }

    #[test]
    fn a_delayed_cable_into_itself_is_a_loop_on_every_layer() {
        let mut g = two_paths((0.0, 0.0), (0.0, 0.3));
        g.nodes[0] = cable_node("mic", Some("c"));
        g.nodes[3] = output("o1", dev("c.in"));
        g.nodes[4] = output("o2", dev("c.in"));
        let plan = delays_of(&g, &[cable("c", "c.in", "c.out")]);
        assert!(plan.streams.is_empty(), "{:?}", plan.streams);
        assert!(plan.issues[0].message.starts_with("Loop: the audio of cable “c”"), "{:?}", plan.issues);
    }

    #[test]
    fn cable_delay_holds_back_what_goes_into_it() {
        // the microphone into a delayed cable: the headphones and the app listening to the cable get it later
        let g = graph(
            vec![input("mic", dev("mic")), delayed(cable_node("c", Some("c")), 0.25), output("hp", dev("hp")), app("call", "call.exe")],
            &[("mic", "c"), ("c", "hp"), ("c", "call")],
        );
        let plan = delays_of(&with_range(g, 0.5), &[cable("c", "c.in", "c.out")]);
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(delays(&plan), vec![("c.in", 0.25), ("hp", 0.0)]);
    }

    #[test]
    fn cable_delay_holds_back_what_comes_out_of_it_if_the_system_plays_into_it() {
        // apps play into the cable through "System": the engine delays the cable's sound to the headphones
        let g = graph(
            vec![system("sys"), delayed(cable_node("c", Some("c")), 0.25), output("hp", dev("hp"))],
            &[("sys", "c"), ("c", "hp")],
        );
        let cables = [cable("c", "c.in", "c.out")];
        let plan = delays_of(&with_range(g.clone(), 0.5), &cables);
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(delays(&plan), vec![("hp", 0.25)]);

        // an app also listens to the cable: the system carries what it hears, without the delay
        let mut g = g;
        g.nodes.push(app("call", "call.exe"));
        g.edges.push(GraphEdge { id: "c->call".into(), source: "c".into(), target: "call".into() });
        let plan = delays_of(&with_range(g, 0.5), &cables);
        assert_eq!(delays(&plan), vec![("hp", 0.25)]);
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["c", "call"]);
        assert_eq!(plan.issues[0].edges, vec!["c->call"]);
    }

    #[test]
    fn cable_delay_cant_apply_to_what_only_the_system_carries() {
        // an app into the cable and another listening to it: the engine has nothing to hold back
        let g = graph(
            vec![app("player", "player.exe"), delayed(cable_node("c", Some("c")), 0.25), app("call", "call.exe")],
            &[("player", "c"), ("c", "call")],
        );
        let cables = [cable("c", "c.in", "c.out")];
        let plan = delays_of(&g, &cables);
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["c", "player", "call"]);
        assert_eq!(plan.issues[0].edges, vec!["player->c", "c->call"]);
    }

    #[test]
    fn virtual_cable_delay_holds_back_its_device() {
        // as on Linux: an app plays into the cable's own device, the engine delays the cable's sound to the headphones
        let g = graph(
            vec![app("player", "player"), delayed(cable_node("mix", None), 0.2), output("phones", dev("hp"))],
            &[("player", "mix"), ("mix", "phones")],
        );
        let (g, cables, _) = virtualize(&with_range(g, 0.5));
        let plan = delays_of(&g, &cables);
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(delays(&plan), vec![("hp", 0.2)]);
    }

    #[test]
    fn output_gives_what_its_device_plays() {
        let g = graph(
            vec![input("i", dev("mic")), output("a", dev("hp")), output("b", dev("spk"))],
            &[("i", "a"), ("a", "b")],
        );
        let plan = compile(&g, &[], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["mic"]), stream("spk", &["loopback:hp"])]);
    }

    #[test]
    fn output_without_a_device_gives_nothing() {
        let g = graph(vec![output("a", None), output("b", dev("spk"))], &[("a", "b")]);
        let plan = compile(&g, &[], &defaults());
        assert!(plan.streams.is_empty());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["a"]);
    }

    #[test]
    fn default_output_gives_the_default_device() {
        let g = graph(vec![output("a", Some(DeviceRef::Default)), output("b", dev("hp"))], &[("a", "b")]);
        let plan = compile(&g, &[], &defaults());
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["loopback:spk"])]);
    }

    #[test]
    fn output_goes_through_internal_cables_and_into_cables() {
        let g = graph(
            vec![output("a", dev("hp")), cable_node("bus", None), cable_node("c", Some("c")), output("b", dev("spk"))],
            &[("a", "bus"), ("bus", "b"), ("a", "c")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(routes(&plan.streams), vec![stream("c.in", &["loopback:hp"]), stream("spk", &["loopback:hp"])]);
    }

    #[test]
    fn arrows_round_an_output_are_a_loop() {
        let g = graph(vec![output("a", dev("hp")), output("b", dev("spk"))], &[("a", "b"), ("b", "a")]);
        let plan = compile(&g, &[], &defaults());
        assert!(plan.streams.is_empty());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].edges, vec!["a->b", "b->a"]);
    }

    /// Two nodes of one device: no circle of arrows, but what the device plays would come back to it
    #[test]
    fn output_into_its_own_device_is_a_loop() {
        let g = graph(
            vec![input("i", dev("mic")), output("a", dev("hp")), output("b", dev("hp")), output("c", dev("spk"))],
            &[("i", "a"), ("a", "b"), ("a", "c")],
        );
        let plan = compile(&g, &[], &defaults());
        // the link to another device is no part of the loop and keeps working
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["mic"]), stream("spk", &["loopback:hp"])]);
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["a"]);
        assert_eq!(plan.issues[0].edges, vec!["a->b"]);
    }

    #[test]
    fn output_back_to_its_device_through_a_cable_is_a_loop() {
        let g = graph(
            vec![output("a", dev("hp")), cable_node("c", Some("c")), output("b", dev("hp"))],
            &[("a", "c"), ("c", "b")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["c.out"])]);
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].edges, vec!["a->c"]);
    }

    #[test]
    fn outputs_looped_across_graphs_are_found() {
        let first = graph(vec![output("a", dev("hp")), output("b", dev("spk"))], &[("a", "b")]);
        let second = graph(vec![output("c", dev("spk")), output("d", dev("hp"))], &[("c", "d")]);
        let plans = compile_all(&[&first, &second], &[], &defaults());
        assert_eq!(routes(&plans[0].streams), vec![stream("spk", &["loopback:hp"])]);
        assert!(plans[1].streams.is_empty());
        assert_eq!(plans[1].issues.len(), 1, "{:?}", plans[1].issues);
    }

    #[cfg(windows)]
    #[test]
    fn app_cant_listen_to_an_output() {
        let g = graph(vec![output("o", dev("hp")), app("a", "a.exe")], &[("o", "a")]);
        let plan = compile(&g, &[], &defaults());
        assert!(plan.routes.is_empty());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].edges, vec!["o->a"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn app_listens_to_an_outputs_monitor() {
        let g = graph(vec![output("o", dev("hp")), app("a", "a.exe")], &[("o", "a")]);
        let plan = compile(&g, &[], &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        assert_eq!(plan.routes[0].capture, Some(Target::Device("hp.monitor".into())));
    }

    #[test]
    fn cable_without_output_is_read_through_loopback() {
        // the default output goes into the input of cable "c" that has no output chosen: the engine
        // takes the sound played into its input and sends it to both outputs; an app can't hear it
        let g = graph(
            vec![system("sys"), cable_node("c", Some("c")), output("hp", dev("hp")), output("spk", dev("spk")), app("a", "a.exe")],
            &[("sys", "c"), ("c", "hp"), ("c", "spk"), ("c", "a")],
        );
        let cables = [Cable { capture: None, ..cable("c", "c.in", "") }];
        let plan = compile(&g, &cables, &defaults());
        assert_eq!(plan.system.unwrap().render, Some("c.in".into()));
        assert_eq!(routes(&plan.streams), vec![stream("hp", &["loopback:c.in"]), stream("spk", &["loopback:c.in"])]);
        assert!(plan.routes.is_empty());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].edges, vec!["c->a"]);
        assert!(plan.issues[0].message.starts_with("Cable “c” has no output"));
    }

    #[test]
    fn cable_without_output_into_itself_is_a_loop() {
        let g = graph(vec![cable_node("c", Some("c")), output("o", dev("c.in"))], &[("c", "o")]);
        let cables = [Cable { capture: None, ..cable("c", "c.in", "") }];
        let plan = compile(&g, &cables, &defaults());
        assert!(plan.issues[0].message.starts_with("Loop: the audio of cable “c”"), "{:?}", plan.issues);
        assert!(plan.streams.is_empty());
    }

    #[test]
    fn virtual_cable_into_itself_is_a_loop() {
        let g = graph(vec![cable_node("a", None), cable_node("b", None)], &[("a", "b"), ("b", "a")]);
        let (g, cables, _) = virtualize(&g);
        let plan = compile(&g, &cables, &defaults());
        assert!(plan.issues[0].message.starts_with("Loop"));
        assert!(plan.streams.is_empty());
    }

    #[test]
    fn virtual_device_names_are_safe() {
        let (sink, source) = virtual_device_names("3f2a-9b {x}/'\"");
        assert_eq!(sink, "audiograph.3f2a-9bx");
        assert_eq!(source, "audiograph.3f2a-9bx.out");
    }
}
