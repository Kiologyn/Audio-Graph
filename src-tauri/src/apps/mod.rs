//! The list of running apps to choose from in an app node. Enumeration depends
//! on the platform, the module looks the same from outside.

use serde::Serialize;

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(windows)]
pub(crate) mod win;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioApp {
    /// Executable file name: the app is recognized by it between launches,
    /// even if an update changed its folder
    pub exe: String,
    /// Full path to the executable file
    pub path: String,
    /// The program name from the file properties, else the file name
    pub name: String,
    /// Has an audio session: the app is shown in the volume mixer
    pub has_audio: bool,
    /// Is playing or recording sound right now
    pub playing: bool,
}

pub fn list() -> Result<Vec<AudioApp>, String> {
    #[cfg(windows)]
    return win::list().map_err(|e| e.to_string());
    #[cfg(target_os = "linux")]
    return linux::list();
    #[cfg(not(any(windows, target_os = "linux")))]
    Ok(Vec::new())
}

/// Enumeration goes through COM and window walking (Windows) or the audio server (Linux),
/// so it runs off the window thread
#[tauri::command]
pub async fn audio_apps() -> Result<Vec<AudioApp>, String> {
    tauri::async_runtime::spawn_blocking(list)
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints this machine's apps and the enumeration time:
    /// cargo test lists_apps -- --nocapture
    #[test]
    fn lists_apps() {
        let started = std::time::Instant::now();
        let apps = list().expect("app enumeration");
        println!("{} apps in {:?}", apps.len(), started.elapsed());
        for a in &apps {
            let audio = if a.playing { "playing" } else if a.has_audio { "audio" } else { "-" };
            println!("{audio:7} {} ({})  [{}]", a.name, a.exe, a.path);
        }
    }
}
