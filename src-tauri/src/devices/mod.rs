//! The system's audio device list. Enumeration depends on the platform,
//! the module looks the same from outside.

use serde::Serialize;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod pairs;
#[cfg(windows)]
pub(crate) mod win;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// The other side of the virtual cable this device belongs to, found by driver properties (Windows)
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

    fn state_of(id: &str) -> Option<DeviceState> {
        list().ok()?.into_iter().find(|d| d.id == id).map(|d| d.state)
    }

    /// Toggles a real Virtual Audio Cable recording device and turns it back on right away.
    /// Changes the system state, so it runs only explicitly:
    /// cargo test toggles_vac -- --ignored --nocapture
    #[test]
    #[ignore]
    fn toggles_vac_capture() {
        let vac: Vec<_> = list()
            .unwrap()
            .into_iter()
            .filter(|d| d.name.contains("Virtual Audio Cable"))
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
