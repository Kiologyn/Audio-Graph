//! Разбор графа в план работы движка. Не зависит от платформы.
//!
//! Что выполняет система, а что сам движок:
//! - связи приложения — это настройки Windows «устройство вывода/ввода приложения»: приложение
//!   играет прямо в устройство ноды справа и слушает устройство ноды слева, движок звук не трогает;
//! - остальные связи (ввод, вывод, кабели) — потоки движка: он читает устройства записи,
//!   смешивает и играет в устройство вывода. Внутренний кабель — точка смешивания внутри движка;
//! - связи ноды «Система» — устройства по умолчанию в Windows: её выход — звук приложений,
//!   играющих в устройство вывода по умолчанию, вход — то, что они слышат как микрофон.

use crate::cables::Cable;
use crate::graphs::{AppRef, DeviceRef, Graph, GraphEdge, GraphNode, NodeKind};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// То, что в графе нельзя выполнить. Остальная часть графа при этом работает.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub nodes: Vec<String>,
    pub edges: Vec<String>,
    pub message: String,
}

/// Устройство, назначенное приложению
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Как у системы: снять с приложения отдельную настройку
    Default,
    Device(String),
}

/// Настройки Windows для одного приложения
#[derive(Clone, Debug, PartialEq)]
pub struct AppRoute {
    pub node: String,
    pub app: AppRef,
    /// Куда приложение играет
    pub render: Option<Target>,
    /// Откуда приложение берёт микрофон
    pub capture: Option<Target>,
    /// Громкость приложения в микшере Windows (не больше 1): нода приложения, а если оно
    /// играет прямо в ноду вывода — ещё и её громкость
    pub volume: f32,
}

/// Устройство записи в потоке и громкость, с которой оно подмешивается
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub device: String,
    /// Произведение громкостей нод на пути: ввод или кабель, внутренние кабели, вывод
    pub gain: f32,
}

/// Поток движка: звук устройств записи смешивается и играется в устройство вывода
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub sink: String,
    pub sources: Vec<Source>,
}

/// Устройства по умолчанию в Windows, которые назначает нода «Система»
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
}

/// Текущие устройства по умолчанию в Windows: «по умолчанию» в нодах ввода и вывода движок
/// открывает как конкретное устройство и переоткрывает, когда система его меняет.
/// Если граф подменяет их нодой «Система», «по умолчанию» значит подменённое устройство
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Defaults {
    pub render: Option<String>,
    pub capture: Option<String>,
}

/// Чем нода является для плана
enum Role<'a> {
    Input(Option<&'a DeviceRef>),
    Output(Option<&'a DeviceRef>),
    App,
    /// Устройства по умолчанию в Windows
    System,
    /// Кабель из общего списка: вход — устройство вывода, выход — устройство записи
    Cable(&'a Cable),
    /// Внутренний кабель: смешивание внутри движка, системе не виден
    Bus,
    /// Нода ссылается на удалённый кабель
    Deleted,
}

struct Compiler<'a> {
    nodes: HashMap<&'a str, &'a GraphNode>,
    cables: HashMap<&'a str, &'a Cable>,
    /// С учётом подмены нодой «Система»
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
        // одна и та же нода попадается на пути нескольких связей
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }

    /// Устройство, из которого движок читает звук ноды
    fn source_device(&mut self, node: &'a GraphNode) -> Option<String> {
        match self.role(node) {
            Role::Input(None) => self.issue(&[&node.id], &[], tr!("Не выбрано устройство ввода")),
            Role::Input(Some(DeviceRef::Device { id, .. })) => return Some(id.clone()),
            Role::Input(Some(DeviceRef::Default)) => match &self.defaults.capture {
                Some(id) => return Some(id.clone()),
                None => self.issue(&[&node.id], &[], tr!("В системе нет устройства ввода по умолчанию")),
            },
            Role::Cable(cable) => match &cable.capture {
                Some(DeviceRef::Device { id, .. }) => return Some(id.clone()),
                _ => self.issue(&[&node.id], &[], tr!(
                    "У кабеля «{cable}» не выбран выход",
                    cable = cable.name
                )),
            },
            Role::Deleted => self.issue(&[&node.id], &[], tr!("Кабель удалён из списка кабелей")),
            Role::Output(_) | Role::App | Role::System | Role::Bus => {}
        }
        None
    }

    /// Устройство, в которое движок играет звук для ноды
    fn sink_device(&mut self, node: &'a GraphNode) -> Option<String> {
        match self.role(node) {
            Role::Output(None) => self.issue(&[&node.id], &[], tr!("Не выбрано устройство вывода")),
            Role::Output(Some(DeviceRef::Device { id, .. })) => return Some(id.clone()),
            Role::Output(Some(DeviceRef::Default)) => match &self.defaults.render {
                Some(id) => return Some(id.clone()),
                None => self.issue(&[&node.id], &[], tr!("В системе нет устройства вывода по умолчанию")),
            },
            Role::Cable(cable) => match &cable.render {
                Some(DeviceRef::Device { id, .. }) => return Some(id.clone()),
                _ => self.issue(&[&node.id], &[], tr!("У кабеля «{cable}» не выбран вход", cable = cable.name)),
            },
            Role::Deleted => self.issue(&[&node.id], &[], tr!("Кабель удалён из списка кабелей")),
            Role::Input(_) | Role::App | Role::System | Role::Bus => {}
        }
        None
    }

    /// Куда направить звук приложения: устройство ноды, в которую идёт связь
    fn app_render(&mut self, edge: &GraphEdge, target: &'a GraphNode) -> Option<Target> {
        let ids = [edge.source.as_str(), target.id.as_str()];
        match self.role(target) {
            Role::Output(Some(DeviceRef::Default)) => return Some(Target::Default),
            Role::Output(_) | Role::Cable(_) | Role::Deleted => return self.sink_device(target).map(Target::Device),
            Role::Bus => self.issue(
                &ids,
                &[&edge.id],
                tr!(
                    "Windows отправляет звук приложения только в устройство, а внутренний кабель системе не виден. Выберите для кабеля кабель из списка"
                ),
            ),
            Role::App | Role::System => {
                self.issue(
                    &ids,
                    &[&edge.id],
                    tr!(
                        "Напрямую их не соединить: звук идёт из устройства вывода в устройство ввода, поставьте между ними кабель"
                    ),
                )
            }
            Role::Input(_) => {}
        }
        None
    }

    /// Откуда приложению брать микрофон: устройство ноды, из которой идёт связь
    fn app_capture(&mut self, edge: &GraphEdge, source: &'a GraphNode) -> Option<Target> {
        let ids = [source.id.as_str(), edge.target.as_str()];
        match self.role(source) {
            Role::Input(Some(DeviceRef::Default)) => return Some(Target::Default),
            Role::Input(_) | Role::Cable(_) | Role::Deleted => return self.source_device(source).map(Target::Device),
            Role::Bus => self.issue(
                &ids,
                &[&edge.id],
                tr!(
                    "Приложение слушает только устройство, а внутренний кабель системе не виден. Выберите для кабеля кабель из списка"
                ),
            ),
            // о связи двух приложений говорит сторона источника
            Role::App | Role::System | Role::Output(_) => {}
        }
        None
    }

    /// Устройства записи, звук которых доходит до ноды по связям движка (через внутренние кабели),
    /// с громкостью: gain — уже набранное по пути, сюда домножаются громкости нод-источников.
    /// Если до устройства несколько путей, берётся самый громкий: звук не удваивается
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

/// Связи, лежащие на петлях, сгруппированные по петлям (компоненты сильной связности)
fn cycles<'a>(nodes: &[&'a str], edges: &[&'a GraphEdge]) -> Vec<(Vec<&'a str>, Vec<&'a str>)> {
    // алгоритм Тарьяна; графы маленькие, рекурсии хватает
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
    // связь лежит на петле, если оба конца в одной компоненте (в том числе связь ноды с собой)
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

    // петли: звук пошёл бы по кругу и завыл. Такие связи не выполняются
    let mut looped: HashSet<&str> = HashSet::new();
    for (nodes, loop_edges) in cycles(&node_ids, &edges) {
        c.issue(
            &nodes,
            &loop_edges,
            tr!("Петля: звук пойдёт по кругу. Связи петли не работают, пока её не разорвать"),
        );
        looped.extend(loop_edges);
    }
    edges.retain(|e| !looped.contains(e.id.as_str()));

    // эти связи выполняет Windows, а не движок
    let is_routed = |c: &Compiler, id: &str| matches!(c.nodes[id].kind, NodeKind::App | NodeKind::System);

    // приложения и «Система»: у каждого одно устройство вывода и одно устройство ввода.
    // «Система» первая: от неё зависит, что значит «по умолчанию» у приложений
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
            c.issue(&[&node.id], &[], tr!("Не выбрано приложение"));
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
                            "Устройство вывода по умолчанию одно: Windows не может отправить звук в несколько мест. Направьте его в кабель, а из кабеля — куда нужно"
                        )
                    } else {
                        tr!(
                            "У приложения одно устройство вывода: Windows не может отправить его звук в несколько мест. Направьте его в кабель, а из кабеля — куда нужно"
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
                            "Устройство ввода по умолчанию одно. Чтобы слышать несколько источников, сведите их во внутренний кабель, оттуда — в кабель из списка, а его — сюда"
                        )
                    } else {
                        tr!(
                            "У приложения одно устройство ввода. Чтобы оно слышало несколько источников, сведите их во внутренний кабель, оттуда — в кабель из списка, а его — в приложение"
                        )
                    },
                );
                None
            }
        };

        if is_system {
            if let Some(first) = &system {
                c.issue(&[&first.node, &node.id], &[], tr!(
                    "Нода «Система» в графе несколько раз: устройство по умолчанию одно"
                ));
                continue;
            }
            // «Система → вывод по умолчанию» ничего не меняет
            let device = |t: Option<Target>| match t {
                Some(Target::Device(id)) => Some(id),
                _ => None,
            };
            let route = SystemRoute {
                node: node.id.clone(),
                render: device(render),
                capture: device(capture),
            };
            // дальше «по умолчанию» в нодах — уже подменённые устройства
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
        // Windows хранит настройку на приложение, а не на ноду
        let app = node.app.as_ref().unwrap();
        let key = app.exe.to_lowercase();
        if let Some(first) = seen_apps.get(&key) {
            c.issue(&[first, &node.id], &[], tr!(
                "«{app}» в графе несколько раз: у приложения одна настройка устройств",
                app = app.name
            ));
            continue;
        }
        seen_apps.insert(key, &node.id);
        // звук приложения, которое играет прямо в устройство ноды вывода, движок не трогает:
        // громкость той ноды применяется через громкость приложения
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

    // потоки движка: связи без приложений. Звук собирается к каждому устройству вывода
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
        // громкость кабеля действует на звук, который из него забирают, поэтому кабель-приёмник её не домножает
        let sink_volume = if matches!(c.role(node), Role::Output(_)) { node.volume } else { 1.0 };
        let mut sources = BTreeMap::new();
        c.collect_sources(node, &engine_edges, &mut HashSet::new(), sink_volume, &mut sources);
        let Some(sink) = c.sink_device(node) else { continue };
        // кабель сам в себя: его вход и выход — одна пара, звук пошёл бы по кругу.
        // Так бывает и неявно: «по умолчанию» сейчас вход кабеля, потому что его назначила «Система»
        if let Some(cable) = cables.iter().find(|cb| device_id(&cb.render) == Some(sink.as_str())) {
            if let Some(capture) = device_id(&cable.capture).filter(|id| sources.contains_key(*id)) {
                sources.remove(capture);
                let into: Vec<&str> = engine_edges.iter().filter(|e| e.target == node.id).map(|e| e.id.as_str()).collect();
                let message = if matches!(c.role(node), Role::Output(Some(DeviceRef::Default))) {
                    tr!(
                        "Петля: устройство вывода по умолчанию сейчас — вход кабеля «{cable}» (его назначила нода «Система»), и звук кабеля вернулся бы в него же",
                        cable = cable.name
                    )
                } else {
                    tr!("Петля: звук кабеля «{cable}» вернулся бы в него же", cable = cable.name)
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
    }
}

/// Несколько графов сразу, в порядке запуска. Потоки в одно устройство смешиваются, а то, что
/// в Windows одно на всех (устройства приложения, устройства по умолчанию), достаётся графу,
/// запущенному раньше; у остальных это проблема. Незапущенный граф, открытый в окне,
/// ставится последним: так видно, с чем он столкнётся, если его запустить
pub fn compile_all(graphs: &[&Graph], cables: &[Cable], defaults: &Defaults) -> Vec<Plan> {
    let mut defaults = defaults.clone();
    let mut app_owner: HashMap<String, &str> = HashMap::new();
    let mut system_owner: Option<&str> = None;
    // звук между устройствами: устройство записи → устройство вывода (потоки принятых графов),
    // вход кабеля → его выход. Кольцо в этой сети — петля через несколько графов
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
                        "«{app}» уже направляет запущенный граф «{graph}»: у приложения одна настройка устройств",
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
                        "Устройства по умолчанию уже назначает запущенный граф «{graph}»",
                        graph = owner
                    ),
                });
                plan.system = None;
            } else {
                system_owner = Some(&graph.name);
                // в графах после этого «по умолчанию» — подменённые устройства
                if let Some(id) = &route.render {
                    defaults.render = Some(id.clone());
                }
                if let Some(id) = &route.capture {
                    defaults.capture = Some(id.clone());
                }
            }
        }

        // петли внутри графа уже убраны, поэтому проверять нужно только графы после первого
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
                    "Петля с другим запущенным графом: звук пошёл бы по кругу через их кабели. Часть связей этого графа не работает, пока её не разорвать"
                ),
            });
        }

        plan.issues.extend(issues);
        plans.push(plan);
    }
    plans
}

/// Есть ли путь по звуку от устройства from к устройству to
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

/// Один план для движка из планов запущенных графов (после compile_all)
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
        // как пример в приложении: микрофон → голос → Discord, Spotify → музыка → наушники и Discord
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
        // у Discord два входа — это ошибка, остальное работает
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
        // два микрофона во внутренний кабель, из него в наушники и в кабель
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
        assert_eq!(messages, vec!["Не выбрано приложение", "Не выбрано устройство ввода", "Кабель удалён из списка кабелей"]);
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
        // всё, что играет по умолчанию, — в кабель «Музыка», микрофон по умолчанию — выход кабеля «Голос»;
        // Spotify → «по умолчанию» следует за системой, то есть тоже в «Музыку»
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
        // «Система → кабель → вывод по умолчанию»: по умолчанию теперь вход этого же кабеля
        let g = graph(
            vec![system("sys"), cable_node("c", Some("c")), output("default", Some(DeviceRef::Default)), output("hp", dev("hp"))],
            &[("sys", "c"), ("c", "default"), ("c", "hp")],
        );
        let plan = compile(&g, &[cable("c", "c.in", "c.out")], &defaults());
        assert_eq!(plan.issues.len(), 1, "{:?}", plan.issues);
        assert_eq!(plan.issues[0].nodes, vec!["default"]);
        assert!(plan.issues[0].message.starts_with("Петля: устройство вывода по умолчанию"));
        assert_eq!(plan.streams, vec![stream("hp", &["c.out"])]);
    }

    #[test]
    fn system_with_app_needs_cable() {
        let g = graph(vec![system("sys"), app("a", "a.exe"), system("sys2"), output("o", dev("hp"))], &[("sys", "a"), ("sys2", "o")]);
        let plan = compile(&g, &[], &defaults());
        let messages: Vec<&str> = plan.issues.iter().map(|i| i.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].starts_with("Напрямую их не соединить"));
        assert!(messages[1].starts_with("Нода «Система» в графе несколько раз"));
    }

    #[test]
    fn volumes_multiply_along_path() {
        // микрофон 50% → внутренний 50% → наушники 200% = 50%; кабель 80% → наушники 200% = 160%;
        // кабель-приёмник свою громкость не домножает; приложение → вывод 50% = 50% в микшере
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
        // два графа играют в одни наушники: потоки смешиваются
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
        assert!(messages[0].starts_with("«X.exe» уже направляет запущенный граф «a»"), "{messages:?}");
        assert!(messages[1].starts_with("Устройства по умолчанию уже назначает"), "{messages:?}");
        let plan = merge(&plans);
        assert_eq!(plan.routes.len(), 1);
        assert_eq!(plan.system.unwrap().node, "s");
    }

    #[test]
    fn loop_across_graphs_is_cut_in_later_graph() {
        // a: кабель c → кабель d, b: кабель d → кабель c — вместе звук пошёл бы по кругу
        let cables = [cable("c", "c.in", "c.out"), cable("d", "d.in", "d.out")];
        let a = named(graph(vec![cable_node("c", Some("c")), cable_node("d", Some("d"))], &[("c", "d")]), "a");
        let b = named(graph(vec![cable_node("d", Some("d")), cable_node("c", Some("c"))], &[("d", "c")]), "b");
        let plans = compile_all(&[&a, &b], &cables, &defaults());
        assert!(plans[0].issues.is_empty());
        assert!(plans[1].issues[0].message.starts_with("Петля с другим запущенным графом"));
        assert_eq!(merge(&plans).streams, vec![stream("d.in", &["c.out"])]);
    }
}
