//! Log for troubleshooting: <data folder>/logs/audio-graph.log. It records startup, the OS version,
//! graph start and stop, engine errors and panics. Over 1 MB the old log moves to .old.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: OnceLock<Mutex<PathBuf>> = OnceLock::new();
const MAX_SIZE: u64 = 1024 * 1024;

/// version: the app version (package.json through tauri.conf.json)
pub fn init(data_dir: &Path, version: &str) {
    let dir = data_dir.join("logs");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("audio-graph.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_SIZE) {
        let _ = fs::rename(&path, dir.join("audio-graph.log.old"));
    }
    let _ = FILE.set(Mutex::new(path));

    // panics go to the log too: without a console they'd be invisible
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write(format!("PANIC: {info}"));
        default_hook(info);
    }));

    write(format!("--- Audio Graph {version} started, {}", system_version()));
}

pub fn write(line: impl AsRef<str>) {
    let Some(file) = FILE.get() else { return };
    let path = file.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&*path) {
        let _ = writeln!(f, "{} {}", timestamp(), line.as_ref());
    }
}

fn timestamp() -> String {
    utc(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64)
}

/// Seconds since 1970 as UTC time: 2026-10-31 20:15:07 UTC
fn utc(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days since 1970-01-01 to a date (Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest / 60 % 60, rest % 60)
}

#[cfg(windows)]
fn system_version() -> String {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let read = |name: PCWSTR| -> Option<String> {
        let mut buf = [0u16; 128];
        let mut size = (buf.len() * 2) as u32;
        let key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) }
            .ok()
            .ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    };
    windows_version(
        &read(w!("ProductName")).unwrap_or_default(),
        &read(w!("DisplayVersion")).unwrap_or_default(),
        &read(w!("CurrentBuild")).unwrap_or_default(),
    )
}

/// "Windows 11 Pro 24H2, build 26100". Windows 11 still says "Windows 10" in ProductName:
/// builds from 22000 on are Windows 11
#[cfg(windows)]
fn windows_version(product: &str, display: &str, build: &str) -> String {
    let eleven = build.parse::<u32>().is_ok_and(|b| b >= 22_000);
    let product = if eleven { product.replacen("Windows 10", "Windows 11", 1) } else { product.to_string() };
    format!("{product} {display}, build {build}")
}

/// Distribution, kernel and audio server: the engine's behavior depends on the server (PulseAudio or PipeWire)
#[cfg(target_os = "linux")]
fn system_version() -> String {
    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| "Linux".into());
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let server = crate::pulse::Pulse::connect("Audio Graph")
        .and_then(|mut p| p.server())
        .map_or_else(|e| format!("audio server unavailable: {e}"), |s| format!("{} {}", s.name, s.version));
    format!("{distro}, kernel {}, {server}", kernel.trim())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_version() -> String {
    std::env::consts::OS.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_709_210_096), "2024-02-29 12:34:56 UTC");
        assert_eq!(utc(1_793_538_687), "2026-11-01 13:11:27 UTC");
        assert_eq!(utc(4_102_444_799), "2099-12-31 23:59:59 UTC");
    }

    #[cfg(windows)]
    #[test]
    fn windows_11_says_so() {
        assert_eq!(windows_version("Windows 10 Pro", "26H2", "26300"), "Windows 11 Pro 26H2, build 26300");
        assert_eq!(windows_version("Windows 10 Home", "22H2", "19045"), "Windows 10 Home 22H2, build 19045");
        assert_eq!(windows_version("", "", ""), " , build ");
    }
}
