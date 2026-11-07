//! The system's audio device list. Enumeration depends on the platform,
//! the module looks the same from outside.

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod pairs;
mod watch;

pub use watch::watch;
#[cfg(windows)]
pub(crate) mod win;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    /// An output device: sound is played into it (speakers, headphones, a cable's input)
    Render,
    /// An input device: sound is recorded from it (a microphone, a cable's output)
    Capture,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Active,
    /// Turned off in the sound settings; can be turned on programmatically (Windows only)
    #[cfg_attr(not(windows), allow(dead_code))]
    Disabled,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// The system's device id, an endpoint ID on Windows
    pub id: String,
    pub name: String,
    pub flow: Flow,
    pub state: DeviceState,
    /// The system default device for its direction
    pub is_default: bool,
    /// A cable device the graph created itself (Linux): nodes don't choose it
    pub is_virtual: bool,
    /// The other side of the virtual cable this device belongs to, found by driver properties (Windows).
    /// A cable's main sides point to each other, a spare side to the main one only (see pairs.rs)
    pub pair: Option<String>,
    /// The name of that cable: "Line 1"
    pub pair_name: Option<String>,
}

/// Enabled and disabled output and input devices
pub fn list() -> Result<Vec<AudioDevice>, String> {
    #[cfg(windows)]
    return win::list().map_err(|e| e.to_string());

    #[cfg(target_os = "linux")]
    return linux::list();

    // macOS isn't implemented yet
    #[cfg(not(any(windows, target_os = "linux")))]
    Ok(Vec::new())
}

/// A chosen device in the list: the one with its id, or if the system no longer has that id
/// (plugged into another USB port, another Bluetooth profile on Linux) the only active device
/// of that direction with the same name. Matches findDevice in src/graph/devices.ts
pub fn find<'a>(devices: &'a [AudioDevice], flow: Flow, id: &str, name: &str) -> Option<&'a AudioDevice> {
    if let Some(d) = devices.iter().find(|d| d.flow == flow && d.id == id) {
        return Some(d);
    }
    let mut same = devices
        .iter()
        .filter(|d| d.flow == flow && d.state == DeviceState::Active && !d.is_virtual && d.name == name);
    match (same.next(), same.next()) {
        (Some(d), None) => Some(d),
        _ => None,
    }
}

/// Turns devices on or off the way the system's sound settings do
pub fn set_enabled(ids: &[String], enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    return win::set_enabled(ids, enabled);

    #[cfg(not(windows))]
    {
        let _ = (ids, enabled);
        Err(tr!("error_device_control_windows_only"))
    }
}

/// Current default devices (output, input); called from the engine thread
#[cfg_attr(not(windows), allow(dead_code))]
pub fn defaults() -> (Option<String>, Option<String>) {
    #[cfg(windows)]
    return win::defaults();
    #[cfg(target_os = "linux")]
    return linux::defaults();
    #[cfg(not(any(windows, target_os = "linux")))]
    (None, None)
}

/// Enumeration goes through COM, so it runs off the window thread
#[tauri::command]
pub async fn audio_devices() -> Result<Vec<AudioDevice>, String> {
    tauri::async_runtime::spawn_blocking(list)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_devices_enabled(ids: Vec<String>, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_enabled(&ids, enabled))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, state: DeviceState) -> AudioDevice {
        AudioDevice {
            id: id.into(),
            name: name.into(),
            flow: Flow::Render,
            state,
            is_default: false,
            is_virtual: false,
            pair: None,
            pair_name: None,
        }
    }

    #[test]
    fn finds_a_device_under_a_new_id() {
        let list = [
            device("usb-port-2", "Headphones (USB)", DeviceState::Active),
            device("bt", "Speakers", DeviceState::Active),
            device("bt-2", "Speakers", DeviceState::Active),
            device("off", "Mic", DeviceState::Disabled),
        ];
        let id = |id: &str, name: &str| find(&list, Flow::Render, id, name).map(|d| d.id.as_str());
        assert_eq!(id("bt", "Old name"), Some("bt"));
        // the same headphones in another USB port
        assert_eq!(id("usb-port-1", "Headphones (USB)"), Some("usb-port-2"));
        // two devices with the name: which one is meant is unknown
        assert_eq!(id("gone", "Speakers"), None);
        // a turned off device isn't a replacement
        assert_eq!(id("gone", "Mic"), None);
        assert_eq!(find(&list, Flow::Capture, "usb-port-1", "Headphones (USB)").map(|d| &d.id), None);
    }

    /// Prints this machine's devices and the enumeration time:
    /// cargo test devices -- --nocapture
    #[test]
    fn lists_devices() {
        let started = std::time::Instant::now();
        let devices = list().expect("device enumeration");
        println!("{} devices in {:?}", devices.len(), started.elapsed());
        for d in &devices {
            println!("{:?} {:?}{} {}  [{}]", d.flow, d.state, if d.is_default { " *" } else { "" }, d.name, d.id);
            if let Some(pair) = &d.pair {
                println!("    cable {:?} with {pair}", d.pair_name.as_deref().unwrap_or_default());
            }
        }
    }

    /// Sets as defaults the same devices that already are: checks the call without changing anything.
    /// cargo test sets_same_default -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn sets_same_default() {
        use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, eMultimedia, eRender};
        for flow in [eRender, eCapture] {
            for role in [eConsole, eMultimedia, eCommunications] {
                let Some(id) = win::default_id(flow, role) else { continue };
                let result = win::set_default(&id, role);
                println!("{flow:?} {role:?}: {id} → {result:?}");
                assert!(result.is_ok());
                assert_eq!(win::default_id(flow, role).as_deref(), Some(id.as_str()));
            }
        }
    }

    /// Makes VAC Line 1 the default microphone for a moment: KeepDefaults must put the old one back.
    /// cargo test defaults_come_back -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn defaults_come_back() {
        use windows::Win32::Media::Audio::{eCapture, eConsole};
        let before = win::default_id(eCapture, eConsole);
        {
            let _defaults = win::KeepDefaults::new();
            let line = list()
                .unwrap()
                .into_iter()
                .find(|d| d.name.starts_with("Line 1 (Virtual Audio Cable") && d.flow == Flow::Capture && d.state == DeviceState::Active)
                .expect("VAC Line 1 must be on")
                .id;
            win::set_default(&line, eConsole).unwrap();
            assert_eq!(win::default_id(eCapture, eConsole), Some(line));
        }
        println!("default microphone: {before:?}");
        assert_eq!(win::default_id(eCapture, eConsole), before);
    }

    #[cfg(windows)]
    fn state_of(id: &str) -> Option<DeviceState> {
        list().ok()?.into_iter().find(|d| d.id == id).map(|d| d.state)
    }

    /// Toggles a real Virtual Audio Cable recording device and turns it back on right away.
    /// Changes the system state, so it runs only explicitly:
    /// cargo test toggles_vac -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn toggles_vac_capture() {
        let _defaults = win::KeepDefaults::new();
        // only devices that are on: a cable turned off by the user stays off
        let vac: Vec<_> = list()
            .unwrap()
            .into_iter()
            .filter(|d| d.name.contains("Virtual Audio Cable") && d.state == DeviceState::Active)
            .collect();
        assert!(!vac.is_empty(), "no Virtual Audio Cable devices");
        let ids: Vec<String> = vac.iter().map(|d| d.id.clone()).collect();

        // turning on devices that are already on changes nothing but checks the call and rights
        println!("turning on (no change): {:?}", set_enabled(&ids, true));

        let capture = vac.iter().find(|d| d.flow == Flow::Capture).expect("VAC recording device");
        let id = vec![capture.id.clone()];
        let disabled = set_enabled(&id, false);
        println!("turning off: {disabled:?}, state: {:?}", state_of(&capture.id));
        // turn it back on in any case
        let enabled = set_enabled(&id, true);
        println!("turning on: {enabled:?}, state: {:?}", state_of(&capture.id));
        assert_eq!(state_of(&capture.id), Some(DeviceState::Active), "the device must be on again");
    }
}
