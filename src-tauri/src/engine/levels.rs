//! How loud the sound coming into nodes is, for the window's meters. An audio thread keeps the loudest
//! value of a node in an atomic and the window takes it from there: neither waits for the other,
//! and nothing is sent anywhere unless the window asks.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// The loudest value since it was last taken. As bits: floats that aren't negative compare like their bits
#[derive(Default)]
pub struct Level(AtomicU32);

impl Level {
    pub fn heard(&self, peak: f32) {
        self.0.fetch_max(peak.to_bits(), Ordering::Relaxed);
    }

    fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

/// The levels of the nodes the engine carries sound through
#[derive(Default)]
pub struct Levels(Mutex<HashMap<String, Arc<Level>>>);

impl Levels {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Level>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A node's level; all the stages of one node share it
    pub fn of(&self, node: &str) -> Arc<Level> {
        self.lock().entry(node.to_string()).or_default().clone()
    }

    /// Forgets the nodes that no longer carry sound
    pub fn keep(&self, nodes: &HashSet<&str>) {
        self.lock().retain(|node, _| nodes.contains(node.as_str()));
    }

    /// Every node's loudest value since the last time
    pub fn take(&self) -> HashMap<String, f32> {
        self.lock().iter().map(|(node, level)| (node.clone(), level.take())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loudest_value_is_kept_until_taken() {
        let levels = Levels::default();
        let (first, second) = (levels.of("a"), levels.of("a"));
        first.heard(0.25);
        second.heard(0.5);
        first.heard(0.125);
        assert_eq!(levels.take()["a"], 0.5);
        assert_eq!(levels.take()["a"], 0.0);
    }

    #[test]
    fn only_nodes_with_sound_stay() {
        let levels = Levels::default();
        levels.of("a");
        levels.of("b");
        levels.keep(&HashSet::from(["b"]));
        assert_eq!(levels.take().into_keys().collect::<Vec<_>>(), vec!["b"]);
    }
}
