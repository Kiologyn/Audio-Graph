//! Starting with the system: a Run value in the registry on Windows, an XDG autostart entry on Linux.
//! Only an installed app can start with the system: a portable file or an AppImage may be moved
//! or replaced by the next version, and the entry would point nowhere.

use crate::storage::{err, CmdResult};

/// The command line flag of a start with the system: the window stays in the tray
pub const FLAG: &str = "--autostart";

/// Whether this copy of the app is installed (not portable), so it can start with the system
pub fn supported() -> bool {
    // in development the flag says whether to act as installed
    if cfg!(debug_assertions) {
        if let Ok(v) = std::env::var("AUDIO_GRAPH_INSTALLED") {
            return v == "1";
        }
    }
    std::env::current_exe().is_ok_and(|exe| installed(&exe))
}

/// Whether this launch is a start with the system
pub fn launched() -> bool {
    std::env::args().any(|a| a == FLAG)
}

/// The installer puts the app into Program Files
#[cfg(windows)]
fn installed(exe: &std::path::Path) -> bool {
    let exe = exe.to_string_lossy().to_lowercase();
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .any(|dir| exe.starts_with(&format!("{}\\", dir.to_lowercase().trim_end_matches('\\'))))
}

/// The deb and rpm packages put the app into /usr; an AppImage runs from a temporary mount
#[cfg(not(windows))]
fn installed(exe: &std::path::Path) -> bool {
    std::env::var_os("APPIMAGE").is_none() && (exe.starts_with("/usr") || exe.starts_with("/opt"))
}

#[cfg(windows)]
mod system {
    use super::*;
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
    const NAME: PCWSTR = w!("Audio Graph");

    pub fn enabled() -> bool {
        unsafe { RegGetValueW(HKEY_CURRENT_USER, KEY, NAME, RRF_RT_REG_SZ, None, None, None) }.is_ok()
    }

    pub fn set(on: bool) -> CmdResult<()> {
        if !on {
            // already absent is fine
            let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, NAME) };
            return Ok(());
        }
        let exe = std::env::current_exe().map_err(err)?;
        let command: Vec<u16> = format!("\"{}\" {FLAG}", exe.display()).encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                KEY,
                NAME,
                REG_SZ.0,
                Some(command.as_ptr().cast()),
                (command.len() * 2) as u32,
            )
        }
        .ok()
        .map_err(err)
    }
}

#[cfg(not(windows))]
mod system {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn entry() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("autostart").join("dev.audiograph.app.desktop"))
    }

    pub fn enabled() -> bool {
        entry().is_some_and(|p| p.exists())
    }

    pub fn set(on: bool) -> CmdResult<()> {
        let path = entry().ok_or_else(|| err("no home folder"))?;
        if !on {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(err(e)),
                _ => Ok(()),
            };
        }
        let exe = std::env::current_exe().map_err(err)?;
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Audio Graph\nExec=\"{}\" {FLAG}\nIcon=audio-graph\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(err)?;
        fs::write(&path, text).map_err(err)
    }
}

/// Whether the app starts with the system now (the entry may also be removed by hand)
pub fn enabled() -> bool {
    system::enabled()
}

pub fn set(on: bool) -> CmdResult<()> {
    if on && !supported() {
        return Err(tr!("error_autostart_portable"));
    }
    system::set(on)
}

/// Whether this copy can start with the system: false for a portable file or an AppImage
#[tauri::command]
pub fn autostart_supported() -> bool {
    supported()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn program_files_is_installed() {
        let pf = std::env::var("ProgramFiles").unwrap();
        assert!(installed(std::path::Path::new(&format!("{pf}\\Audio Graph\\audio-graph.exe"))));
        assert!(!installed(std::path::Path::new(r"C:\Users\me\Downloads\Audio Graph 1.1.1 portable.exe")));
    }

    /// Writes the real Run value of the current user and removes it: run by hand
    #[test]
    #[ignore]
    fn run_entry_round_trip() {
        assert!(!enabled(), "remove the existing entry first");
        system::set(true).unwrap();
        assert!(enabled());
        system::set(false).unwrap();
        assert!(!enabled());
    }
}
