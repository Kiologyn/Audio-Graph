//! The engine on Windows: WASAPI streams and app devices.

pub(crate) mod probe;
mod router;
mod streams;

use super::plan::{Defaults, Plan};
use super::{AppStatus, StreamStatus};
use std::path::Path;

pub struct Backend {
    streams: streams::Streams,
    router: router::Router,
}

impl Backend {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            streams: streams::Streams::default(),
            router: router::Router::new(data_dir),
        }
    }

    pub fn defaults(&self) -> Defaults {
        let (render, capture) = crate::devices::defaults();
        Defaults { render, capture }
    }

    pub fn apply(&mut self, plan: &Plan) -> (Vec<StreamStatus>, Vec<AppStatus>) {
        let streams = self.streams.apply(&plan.streams);
        let mut apps = self.router.apply(&plan.routes);
        apps.extend(self.router.apply_system(plan.system.as_ref()));
        (streams, apps)
    }

    /// Devices changed: failed streams may try again without waiting for their retry pause
    pub fn retry_now(&mut self) {
        self.streams.retry_now();
    }

    pub fn shutdown(&mut self) {
        self.streams.shutdown();
    }
}
