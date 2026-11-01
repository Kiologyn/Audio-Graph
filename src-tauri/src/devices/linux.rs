//! Linux devices: sinks are output devices, sources are input devices (monitors of output
//! devices don't count). Virtual cable devices are listed too but marked: nodes can't
//! choose them, but showing that a cable is the default now is needed.

use super::{AudioDevice, DeviceState, Flow};
use crate::engine::plan::VIRTUAL_PREFIX;
use crate::pulse::Pulse;

pub fn list() -> Result<Vec<AudioDevice>, String> {
    let mut pulse = Pulse::connect("Audio Graph")?;
    let server = pulse.server()?;
    let mut devices = Vec::new();
    for (flow, list, default) in [
        (Flow::Render, pulse.sinks()?, &server.default_sink),
        (Flow::Capture, pulse.sources()?, &server.default_source),
    ] {
        for d in list.into_iter().filter(|d| d.monitor_of.is_none()) {
            devices.push(AudioDevice {
                is_default: default.as_deref() == Some(d.name.as_str()),
                is_virtual: d.name.starts_with(VIRTUAL_PREFIX),
                // cables on Linux are the graph's own devices, not a driver's pairs
                pair: None,
                pair_name: None,
                name: if d.description.is_empty() { d.name.clone() } else { d.description },
                id: d.name,
                flow,
                state: DeviceState::Active,
            });
        }
    }
    Ok(devices)
}

/// Current default devices (output, input)
pub fn defaults() -> (Option<String>, Option<String>) {
    Pulse::connect("Audio Graph")
        .and_then(|mut p| p.server())
        .map(|s| (s.default_sink, s.default_source))
        .unwrap_or_default()
}
