//! Engine streams: every recording device and every output device gets its own stream, a thread
//! that the platform provides (win/streams.rs, linux/streams.rs). Recording hands packets
//! to all links, output plays what its mixer makes of its links.

use super::levels::Levels;
use super::link::{capacity_for, link};
use super::mixer::Mixer;
use super::plan::{Stream, LOOPBACK_PREFIX};
use super::ring::{ring, Producer};
use super::{DeviceError, StreamStatus};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use super::linux::streams::{capture_thread, render_thread};
#[cfg(windows)]
use super::win::streams::{capture_thread, render_thread};

/// A failed stream (device gone or busy) restarts no more often than this
const RETRY: Duration = Duration::from_secs(2);

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // a panic in one audio thread must not bring down the others
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The device at the other end of a link and the link's layer: one source can come into
/// one output device several times, delayed differently (Stream::layers)
pub(super) type Peer = (String, usize);
/// What a recording device hands its packets to
pub(super) type Recipients = Vec<(Peer, Producer)>;
type Body<T> = fn(&str, &Mutex<T>, &AtomicBool, &AtomicBool) -> Result<(), String>;

/// The stream of one device
pub(super) struct Worker<T> {
    /// Changes on the fly, the stream reads it on every packet
    pub(super) links: Arc<Mutex<T>>,
    stop: Arc<AtomicBool>,
    /// The device is open and sound flows
    working: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
    started: Option<Instant>,
    error: Option<String>,
}

impl<T: Default + Send + 'static> Worker<T> {
    pub(super) fn new() -> Self {
        Self {
            links: Arc::default(),
            stop: Arc::new(AtomicBool::new(false)),
            working: Arc::new(AtomicBool::new(false)),
            thread: None,
            started: None,
            error: None,
        }
    }

    /// Starts the stream if it isn't running; a failed one at most once per RETRY
    pub(super) fn supervise(&mut self, device: &str, body: Body<T>) {
        if self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        if let Some(thread) = self.thread.take() {
            self.error = match thread.join() {
                Ok(result) => result.err(),
                Err(_) => Some(tr!("error_stream_crashed")),
            };
        }
        if self.started.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        let (device, links, stop, working) = (device.to_string(), self.links.clone(), self.stop.clone(), self.working.clone());
        self.started = Some(Instant::now());
        self.working.store(false, Ordering::Relaxed);
        self.thread = std::thread::Builder::new()
            .name(format!("audio {device}"))
            .spawn(move || body(&device, &links, &stop, &working))
            .ok();
    }

    /// The next supervise may start a failed stream at once
    fn retry_now(&mut self) {
        self.started = None;
    }

    pub(super) fn error(&self) -> Option<String> {
        if self.working.load(Ordering::Relaxed) {
            None
        } else {
            self.error.clone()
        }
    }

    pub(super) fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn device_error<T: Default + Send + 'static>(device: &str, worker: Option<&Worker<T>>) -> Option<DeviceError> {
    worker.and_then(|w| w.error()).map(|message| DeviceError {
        // a loopback's errors belong to its output device: the interface finds the node by it
        device: device.strip_prefix(LOOPBACK_PREFIX).unwrap_or(device).to_string(),
        message,
    })
}

#[derive(Default)]
pub struct Streams {
    pub(super) captures: HashMap<String, Worker<Recipients>>,
    pub(super) renders: HashMap<String, Worker<Mixer>>,
    levels: Arc<Levels>,
}

impl Streams {
    pub fn new(levels: Arc<Levels>) -> Self {
        Self { levels, ..Self::default() }
    }

    /// Brings streams in line with the plan on the fly: new links are added, extra ones removed
    pub fn apply(&mut self, plan: &[Stream]) -> Vec<StreamStatus> {
        // link → delay
        let wanted: BTreeMap<(&str, &str, usize), f32> =
            plan.iter().flat_map(|s| s.layers().map(|(src, layer)| ((src.device.as_str(), s.sink.as_str(), layer), src.delay))).collect();

        for (device, w) in &self.captures {
            lock(&w.links).retain(|((sink, layer), _)| wanted.contains_key(&(device.as_str(), sink.as_str(), *layer)));
        }
        for (device, w) in &self.renders {
            lock(&w.links).links.retain(|((source, layer), _)| wanted.contains_key(&(source.as_str(), device.as_str(), *layer)));
        }
        for &(source, sink, layer) in wanted.keys() {
            let capture = self.captures.entry(source.to_string()).or_insert_with(Worker::new);
            if lock(&capture.links).iter().any(|((s, l), _)| s == sink && *l == layer) {
                continue;
            }
            let (producer, reader) = link();
            lock(&capture.links).push(((sink.to_string(), layer), producer));
            let render = self.renders.entry(sink.to_string()).or_insert_with(Worker::new);
            lock(&render.links).links.push(((source.to_string(), layer), reader));
        }
        // they change on the fly without recreating the link
        for (&(source, sink, layer), &delay) in &wanted {
            self.fit_buffer((source, sink, layer), delay);
            let mixer = lock(&self.renders[sink].links);
            if let Some((_, reader)) = mixer.links.iter().find(|((s, l), _)| s == source && *l == layer) {
                reader.set_delay(delay);
            }
        }
        // stages and the links into them change at one moment: the stream must not see half of it
        for stream in plan {
            let Some(render) = self.renders.get(&stream.sink) else { continue };
            let mut mixer = lock(&render.links);
            mixer.set_stages(&stream.stages, &self.levels);
            for (source, layer) in stream.layers() {
                if let Some((_, reader)) = mixer.links.iter_mut().find(|((s, l), _)| *s == source.device && *l == layer) {
                    reader.stage = source.stage;
                }
            }
        }
        let carrying: HashSet<&str> = plan.iter().flat_map(|s| &s.stages).map(|s| s.node.as_str()).collect();
        self.levels.keep(&carrying);

        // devices without links are closed
        let unused: Vec<String> = self.captures.iter().filter(|(_, w)| lock(&w.links).is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.captures.remove(&device).unwrap().shutdown();
        }
        let unused: Vec<String> = self.renders.iter().filter(|(_, w)| lock(&w.links).links.is_empty()).map(|(d, _)| d.clone()).collect();
        for device in unused {
            self.renders.remove(&device).unwrap().shutdown();
        }

        for (device, w) in &mut self.captures {
            w.supervise(device, capture_thread);
        }
        for (device, w) in &mut self.renders {
            w.supervise(device, render_thread);
        }

        plan.iter()
            .map(|s| StreamStatus {
                sink: s.sink.clone(),
                sources: s.sources.iter().map(|src| src.device.clone()).collect(),
                errors: device_error(&s.sink, self.renders.get(&s.sink))
                    .into_iter()
                    .chain(s.sources.iter().filter_map(|src| device_error(&src.device, self.captures.get(&src.device))))
                    .collect(),
            })
            .collect()
    }

    /// Gives a link the buffer its delay needs: a longer delay could never gather in a smaller one.
    /// The buffer is made before taking the locks: the audio threads only wait for the sound to move
    fn fit_buffer(&self, (source, sink, layer): (&str, &str, usize), delay: f32) {
        let (capture, render) = (&self.captures[source], &self.renders[sink]);
        let capacity = capacity_for(delay);
        let matches = |peer: &Peer| peer.0 == source && peer.1 == layer;
        if lock(&render.links).links.iter().find(|(peer, _)| matches(peer)).is_none_or(|(_, r)| r.capacity() == capacity) {
            return;
        }
        let buffer = ring(capacity);
        let (mut producers, mut mixer) = (lock(&capture.links), lock(&render.links));
        let producer = producers.iter_mut().find(|((s, l), _)| s == sink && *l == layer);
        let reader = mixer.links.iter_mut().find(|(peer, _)| matches(peer));
        if let (Some((_, producer)), Some((_, reader))) = (producer, reader) {
            reader.resize(producer, buffer, delay);
        }
    }

    pub fn retry_now(&mut self) {
        self.captures.values_mut().for_each(Worker::retry_now);
        self.renders.values_mut().for_each(Worker::retry_now);
    }

    #[cfg(target_os = "linux")]
    pub fn is_empty(&self) -> bool {
        self.captures.is_empty() && self.renders.is_empty()
    }

    /// The device is about to go away: its streams close now and its links are forgotten,
    /// the next apply makes the ones still planned. A stream left to fail would wait for RETRY
    #[cfg(target_os = "linux")]
    pub fn close(&mut self, device: &str) {
        if let Some(w) = self.captures.remove(device) {
            w.shutdown();
        }
        if let Some(w) = self.renders.remove(device) {
            w.shutdown();
        }
        for w in self.captures.values() {
            lock(&w.links).retain(|((sink, _), _)| sink != device);
        }
        for w in self.renders.values() {
            lock(&w.links).links.retain(|((source, _), _)| source != device);
        }
    }

    pub fn shutdown(&mut self) {
        for (_, w) in self.captures.drain() {
            w.shutdown();
        }
        for (_, w) in self.renders.drain() {
            w.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::plan::Gain;
    use super::*;

    fn plan(sources: &[(Gain, f32)]) -> [Stream; 1] {
        let sources: Vec<(&str, Gain, f32)> = sources.iter().map(|&(gain, delay)| ("no.source", gain, delay)).collect();
        [Stream::through("no.sink", true, &sources)]
    }

    /// One source in layers: links are made and removed per layer. The devices don't exist, no sound is played
    #[test]
    fn layers_of_one_source_are_separate_links() {
        let mut streams = Streams::default();
        let links = |streams: &Streams| (lock(&streams.captures["no.source"].links).len(), lock(&streams.renders["no.sink"].links).links.len());

        streams.apply(&plan(&[([1.0, 0.0], 0.0), ([0.0, 1.0], 0.5)]));
        assert_eq!(links(&streams), (2, 2));
        // another delay is the same links
        streams.apply(&plan(&[([1.0, 0.0], 0.0), ([0.0, 1.0], 0.3)]));
        assert_eq!(links(&streams), (2, 2));
        streams.apply(&plan(&[([1.0, 1.0], 0.3)]));
        assert_eq!(links(&streams), (1, 1));
        streams.shutdown();
    }

    /// The devices don't exist here either: only the link's buffer is looked at
    #[test]
    fn a_link_takes_the_memory_its_delay_needs() {
        let mut streams = Streams::default();
        let capacity = |streams: &Streams| lock(&streams.renders["no.sink"].links).links[0].1.capacity();
        streams.apply(&plan(&[([1.0; 2], 0.0)]));
        assert_eq!(capacity(&streams), capacity_for(0.0));
        streams.apply(&plan(&[([1.0; 2], 300.0)]));
        assert_eq!(capacity(&streams), capacity_for(300.0));
        streams.apply(&plan(&[([1.0; 2], 0.1)]));
        assert_eq!(capacity(&streams), capacity_for(0.1));
        streams.shutdown();
    }

    /// Links go into the stages of the plan, whatever order they were made in
    #[test]
    fn links_follow_their_stages() {
        let mut streams = Streams::default();
        let stages = |streams: &Streams| lock(&streams.renders["no.sink"].links).links.iter().map(|(_, r)| r.stage).collect::<Vec<_>>();
        streams.apply(&plan(&[([1.0; 2], 0.0), ([1.0; 2], 0.2)]));
        assert_eq!(stages(&streams), vec![1, 2]);
        let mut changed = plan(&[([1.0; 2], 0.0), ([1.0; 2], 0.2)]);
        changed[0].sources[1].stage = 1;
        streams.apply(&changed);
        assert_eq!(stages(&streams), vec![1, 1]);
        streams.shutdown();
    }
}
