//! Linux apps are clients of the audio server: the ones with sound have playback
//! or recording streams, the others are connected to the server but silent now.
//! An app is recognized by its executable file name (application.process.binary).

use super::AudioApp;
use crate::pulse::{Pulse, Stream};
use std::collections::HashMap;

/// The audio server itself, its services and tools: these are not apps
const SKIP: &[&str] = &["pipewire", "pipewire-pulse", "wireplumber", "pulseaudio", "pactl", "pacmd", "audio-graph"];

/// Full path to a process's executable file
pub fn process_path(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Whether a process with this executable file runs (name in lower case)
pub fn is_running(exe: &str) -> bool {
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else { return false };
        let file = process_path(pid).and_then(|p| p.rsplit('/').next().map(str::to_lowercase));
        // another user's process exe can't be read: then by the process name (cut to 15 characters)
        let file = file.or_else(|| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|c| c.trim().to_lowercase()));
        file.is_some_and(|f| f == exe || (f.len() == 15 && exe.starts_with(&f)))
    })
}

/// A stream of an app, not of a server module or of Audio Graph itself
pub fn is_app_stream(s: &Stream) -> bool {
    s.client.is_some() && s.pid != Some(std::process::id()) && s.binary.is_some()
}

pub fn list() -> Result<Vec<AudioApp>, String> {
    let mut pulse = Pulse::connect("Audio Graph")?;
    let streams: Vec<Stream> = pulse.sink_inputs()?.into_iter().chain(pulse.source_outputs()?).collect();
    let clients = pulse.clients()?;
    let own = std::process::id();

    let mut apps: Vec<AudioApp> = Vec::new();
    let mut by_exe: HashMap<String, usize> = HashMap::new();
    let entries = streams
        .iter()
        .filter(|s| is_app_stream(s))
        .map(|s| (s.binary.clone(), s.pid, s.app_name.clone(), Some(!s.corked)))
        .chain(clients.into_iter().filter(|c| c.pid != Some(own)).map(|c| (c.binary, c.pid, c.app_name, None)));
    for (binary, pid, app_name, stream) in entries {
        let Some(exe) = binary else { continue };
        if SKIP.contains(&exe.to_lowercase().as_str()) {
            continue;
        }
        let i = *by_exe.entry(exe.to_lowercase()).or_insert_with(|| {
            apps.push(AudioApp {
                name: app_name.filter(|n| !n.is_empty()).unwrap_or_else(|| exe.clone()),
                path: pid.and_then(process_path).unwrap_or_else(|| exe.clone()),
                exe: exe.clone(),
                has_audio: false,
                playing: false,
            });
            apps.len() - 1
        });
        if let Some(active) = stream {
            apps[i].has_audio = true;
            apps[i].playing |= active;
        }
    }
    apps.sort_by_cached_key(|a| (!a.has_audio, a.name.to_lowercase()));
    Ok(apps)
}
