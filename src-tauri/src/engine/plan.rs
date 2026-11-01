//! Turns a graph into the engine's work plan. Platform-independent.
//!
//! What the system does and what the engine does itself:
//! - an app's links are the system's per-app output/input device settings: the app
//!   plays straight into the device of the node on its right and hears the node on its left, the engine doesn't touch the sound;
//! - other links (input, output, cables) are engine streams: it reads recording devices,
//!   mixes and plays into an output device. An internal cable is a mix point inside the engine;
//! - links of the "System" node are the system default devices: its output is the sound of apps
//!   playing into the default output device, its input is what they hear as the microphone.
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
    /// The app's volume in the system mixer (at most 1): the app node's, and if it
    /// plays straight into an output node, that node's volume too
    pub volume: f32,
}

/// A recording device in a stream and the volume it is mixed in with
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub device: String,
    /// The product of node volumes along the path: input or cable, internal cables, output
    pub gain: f32,
}

/// An engine stream: sound of recording devices is mixed and played into an output device
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub sink: String,
    pub sources: Vec<Source>,
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

/// A stream source "loopback:<output device>" is the sound played into that output device
/// (WASAPI loopback). It stands in for the output of a cable that has only its input chosen
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

struct Compiler<'a> {
    nodes: HashMap<&'a str, &'a GraphNode>,
    cables: HashMap<&'a str, &'a Cable>,
    /// Taking the "System" node replacement into account
    defaults: Defaults,
    issues: Vec<Issue>,
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
            Role::Deleted => self.issue(&[&node.id], &[], tr!("issue_cable_deleted")),
            Role::Output(_) | Role::App | Role::System | Role::Bus => {}
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
            // a link between two apps is reported by the source side
            Role::App | Role::System | Role::Output(_) => {}
        }
        None
    }

    /// Recording devices whose sound reaches the node through engine links (via internal cables),
    /// with volume: gain is what the path has gathered so far, source node volumes multiply into it.
    /// If there are several paths to a device, the loudest one counts: the sound isn't doubled
    fn collect_sources(
        &mut self,
        node: &'a GraphNode,
        engine_edges: &[&'a GraphEdge],
        visited: &mut HashSet<&'a str>,
        gain: f32,
        out: &mut BTreeMap<String, f32>,
    ) {
        for edge in engine_edges.iter().filter(|e| e.target == node.id) {
            let source = self.nodes[edge.source.as_str()];
            let gain = gain * source.volume;
            if matches!(self.role(source), Role::Bus) {
                if visited.insert(source.id.as_str()) {
                    self.collect_sources(source, engine_edges, visited, gain, out);
                }
            } else if let Some(id) = self.source_device(source) {
                let g = out.entry(id).or_insert(gain);
                *g = g.max(gain);
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

    // the system carries out these links, not the engine
    let is_routed = |c: &Compiler, id: &str| matches!(c.nodes[id].kind, NodeKind::App | NodeKind::System);

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
            [edge] => c.app_render(edge, c.nodes[edge.target.as_str()]),
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
            [edge] => c.app_capture(edge, c.nodes[edge.source.as_str()]),
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
        // the engine doesn't touch the sound of an app playing straight into an output node's device:
        // that node's volume is applied through the app's volume
        let output_volume = match outgoing.as_slice() {
            [edge] if c.nodes[edge.target.as_str()].kind == NodeKind::Output => c.nodes[edge.target.as_str()].volume,
            _ => 1.0,
        };
        routes.push(AppRoute {
            node: node.id.clone(),
            app: app.clone(),
            render,
            capture,
            volume: (node.volume * output_volume).clamp(0.0, 1.0),
        });
    }

    // engine streams: links without apps. Sound is gathered for every output device
    let engine_edges: Vec<&GraphEdge> = edges
        .iter()
        .copied()
        .filter(|e| !is_routed(&c, &e.source) && !is_routed(&c, &e.target))
        .collect();
    let mut streams: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();
    for node in &graph.nodes {
        let is_sink = matches!(c.role(node), Role::Output(_) | Role::Cable(_) | Role::Deleted);
        if !is_sink || !engine_edges.iter().any(|e| e.target == node.id) {
            continue;
        }
        // a cable's volume applies to the sound taken from it, so a receiving cable doesn't multiply it in
        let sink_volume = if matches!(c.role(node), Role::Output(_)) { node.volume } else { 1.0 };
        let mut sources = BTreeMap::new();
        c.collect_sources(node, &engine_edges, &mut HashSet::new(), sink_volume, &mut sources);
        let Some(sink) = c.sink_device(node) else { continue };
        // a cable into itself: its input and output are one pair, sound would go round in circles.
        // It also happens implicitly: "default" is now the cable's input because "System" set it
        if let Some(cable) = cables.iter().find(|cb| device_id(&cb.render) == Some(sink.as_str())) {
            if let Some(capture) = cable_source(cable).filter(|id| sources.contains_key(id)) {
                sources.remove(&capture);
                let into: Vec<&str> = engine_edges.iter().filter(|e| e.target == node.id).map(|e| e.id.as_str()).collect();
                let message = if matches!(c.role(node), Role::Output(Some(DeviceRef::Default))) {
                    tr!(
                        "issue_default_output_loop",
                        cable = cable.name
                    )
                } else {
                    tr!("issue_cable_into_itself", cable = cable.name)
                };
                c.issue(&[&node.id], &into, message);
            }
        }
        let merged = streams.entry(sink).or_default();
        for (device, gain) in sources {
            let g = merged.entry(device).or_insert(gain);
            *g = g.max(gain);
        }
    }

    Plan {
        routes,
        system,
        streams: streams
            .into_iter()
            .filter(|(_, sources)| !sources.is_empty())
            .map(|(sink, sources)| Stream {
                sink,
                sources: sources.into_iter().map(|(device, gain)| Source { device, gain }).collect(),
            })
            .collect(),
        issues: c.issues,
        virtual_cables: Vec::new(),
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
    let mut links: HashMap<String, BTreeSet<String>> = HashMap::new();
    for c in cables {
        if let (Some(render), Some(capture)) = (device_id(&c.render), device_id(&c.capture)) {
            links.entry(render.to_string()).or_default().insert(capture.to_string());
        }
    }
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
fn reaches(links: &HashMap<String, BTreeSet<String>>, from: &str, to: &str) -> bool {
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
    let mut streams: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();
    for stream in plans.iter().flat_map(|p| &p.streams) {
        let merged = streams.entry(stream.sink.clone()).or_default();
        for s in &stream.sources {
            let g = merged.entry(s.device.clone()).or_insert(s.gain);
            *g = g.max(s.gain);
        }
    }
    Plan {
        routes: plans.iter().flat_map(|p| p.routes.iter().cloned()).collect(),
        system: plans.iter().find_map(|p| p.system.clone()),
        streams: streams
            .into_iter()
            .map(|(sink, sources)| Stream {
                sink,
                sources: sources.into_iter().map(|(device, gain)| Source { device, gain }).collect(),
            })
            .collect(),
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
            show_volume: true,
        }
    }
    fn defaults() -> Defaults {
        Defaults {
            render: Some("spk".into()),
            capture: Some("mic".into()),
        }
    }
    fn stream(sink: &str, sources: &[&str]) -> Stream {
        Stream {
            sink: sink.into(),
            sources: sources
                .iter()
                .map(|s| Source {
                    device: s.to_string(),
                    gain: 1.0,
                })
                .collect(),
        }
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
        assert_eq!(plan.streams, vec![stream("hp", &["m.out"]), stream("v.in", &["mic"])]);
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
        assert_eq!(plan.streams, vec![stream("c.in", &["mic1", "mic2"]), stream("hp", &["mic1", "mic2"])]);
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
        assert_eq!(plan.streams, vec![stream("hp", &["mic"])]);
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
        assert_eq!(plan.streams, vec![stream("hp", &["m.out"]), stream("v.in", &["mic"])]);
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
        assert_eq!(plan.streams, vec![stream("hp", &["c.out"])]);
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
    fn volumes_multiply_along_path() {
        // microphone 50% → internal 50% → headphones 200% = 50%; cable 80% → headphones 200% = 160%;
        // a receiving cable doesn't multiply in its volume; app → output 50% = 50% in the mixer
        let vol = |mut n: GraphNode, v: f32| {
            n.volume = v;
            n
        };
        let g = graph(
            vec![
                vol(input("mic", dev("mic")), 0.5),
                vol(cable_node("bus", None), 0.5),
                vol(output("o", dev("hp")), 2.0),
                vol(cable_node("c", Some("c")), 0.8),
                vol(cable_node("rec", Some("r")), 0.3),
                app("a", "a.exe"),
                vol(output("o2", dev("spk")), 0.5),
            ],
            &[("mic", "bus"), ("bus", "o"), ("c", "o"), ("mic", "rec"), ("a", "o2")],
        );
        let cables = [cable("c", "c.in", "c.out"), cable("r", "r.in", "r.out")];
        let plan = compile(&g, &cables, &defaults());
        assert!(plan.issues.is_empty(), "{:?}", plan.issues);
        let gains: Vec<(String, String, f32)> = plan
            .streams
            .iter()
            .flat_map(|s| s.sources.iter().map(|src| (s.sink.clone(), src.device.clone(), src.gain)))
            .collect();
        assert_eq!(
            gains,
            vec![
                ("hp".to_string(), "c.out".to_string(), 1.6),
                ("hp".to_string(), "mic".to_string(), 0.5),
                ("r.in".to_string(), "mic".to_string(), 0.5),
            ]
        );
        assert_eq!(plan.routes[0].volume, 0.5);
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
        assert_eq!(merge(&plans).streams, vec![stream("hp", &["c.out", "mic"])]);
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
        assert_eq!(merge(&plans).streams, vec![stream("d.in", &["c.out"])]);
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
        assert_eq!(plan.streams, vec![stream("audiograph.mix", &["mic"]), stream("hp", &["audiograph.mix.out"])]);
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
        assert_eq!(plan.streams, vec![stream("hp", &["loopback:c.in"]), stream("spk", &["loopback:c.in"])]);
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
