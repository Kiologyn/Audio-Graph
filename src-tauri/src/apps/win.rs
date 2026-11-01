//! Windows apps: audio sessions (as in the volume mixer) and windows shown
//! on the taskbar. An app without sound can be chosen in advance too.

use super::AudioApp;
use crate::com::{wide, ComGuard};
use std::collections::HashMap;
use std::ffi::c_void;
use windows::core::{w, Interface, Result, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, S_OK};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Media::Audio::{
    eAll, eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

/// Shell service processes: they have windows but are not apps
const SKIP: &[&str] = &["applicationframehost.exe", "textinputhost.exe", "explorer.exe"];

/// Volume controls of playback sessions, the same as in the volume mixer: pid → its sessions
pub fn session_volumes() -> HashMap<u32, Vec<ISimpleAudioVolume>> {
    let mut map: HashMap<u32, Vec<ISimpleAudioVolume>> = HashMap::new();
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return map;
        };
        let Ok(devices) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else {
            return map;
        };
        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(manager) = devices.Item(i).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
            else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
            for j in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(control) = sessions.GetSession(j).and_then(|s| s.cast::<IAudioSessionControl2>()) else {
                    continue;
                };
                if control.IsSystemSoundsSession() == S_OK || control.GetState().unwrap_or(AudioSessionStateExpired) == AudioSessionStateExpired {
                    continue;
                }
                let (Ok(pid), Ok(volume)) = (control.GetProcessId(), control.cast::<ISimpleAudioVolume>()) else {
                    continue;
                };
                map.entry(pid).or_default().push(volume);
            }
        }
    }
    map
}

/// Processes with audio sessions on all enabled devices: pid → a session is active now
pub fn session_pids() -> Result<HashMap<u32, bool>> {
    let mut pids = HashMap::new();
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            // one broken device must not hide the sessions on the others
            let Ok(manager) = devices.Item(i).and_then(|d| d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
            else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
            for j in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(control) = sessions.GetSession(j).and_then(|s| s.cast::<IAudioSessionControl2>()) else {
                    continue;
                };
                // "System sounds" is a separate session without an app
                if control.IsSystemSoundsSession() == S_OK {
                    continue;
                }
                let state = control.GetState().unwrap_or(AudioSessionStateExpired);
                let pid = control.GetProcessId().unwrap_or(0);
                if state == AudioSessionStateExpired || pid == 0 {
                    continue;
                }
                *pids.entry(pid).or_default() |= state == AudioSessionStateActive;
            }
        }
    }
    Ok(pids)
}

/// A window shown on the taskbar: visible, with a title, without an owner, not a tool window
unsafe fn is_app_window(hwnd: HWND) -> bool {
    if !IsWindowVisible(hwnd).as_bool() || GetWindowTextLengthW(hwnd) == 0 {
        return false;
    }
    if GetWindow(hwnd, GW_OWNER).is_ok() || GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }
    // windows hidden by the system: Store apps in the background, shell service windows
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4);
    cloaked == 0
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let pids = &mut *(data.0 as *mut Vec<u32>);
    if is_app_window(hwnd) {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pids.push(pid);
    }
    true.into()
}

fn window_pids() -> Result<Vec<u32>> {
    let mut pids: Vec<u32> = Vec::new();
    unsafe { EnumWindows(Some(collect_window), LPARAM(&mut pids as *mut Vec<u32> as isize))? };
    Ok(pids)
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// "File description" from its properties: the volume mixer names the app this way
fn file_description(path: &str) -> Option<String> {
    let path = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, data.as_mut_ptr() as *mut c_void).ok()?;
        let block = data.as_ptr() as *const c_void;
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        // strings are stored per language; take the first language of the translation table
        if !VerQueryValueW(block, w!("\\VarFileInfo\\Translation"), &mut ptr, &mut len).as_bool() || len < 4 {
            return None;
        }
        let (lang, codepage) = (*(ptr as *const u16), *(ptr as *const u16).add(1));
        let key = wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription"));
        if !VerQueryValueW(block, PCWSTR(key.as_ptr()), &mut ptr, &mut len).as_bool() || len == 0 {
            return None;
        }
        // some files give a length past the string that covers the next entry: cut at the zero
        let raw = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        let raw = &raw[..raw.iter().position(|&c| c == 0).unwrap_or(raw.len())];
        let text = String::from_utf16_lossy(raw);
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

pub fn list() -> Result<Vec<AudioApp>> {
    let _com = ComGuard::init();
    let sessions = session_pids()?;
    let windows = window_pids()?;
    let own = unsafe { GetCurrentProcessId() };

    // browsers and Discord have many processes of one file: one app per file
    let mut apps: Vec<AudioApp> = Vec::new();
    let mut by_path: HashMap<String, usize> = HashMap::new();
    let pids = sessions
        .iter()
        .map(|(&pid, &active)| (pid, Some(active)))
        .chain(windows.into_iter().map(|pid| (pid, None)));
    for (pid, session) in pids {
        if pid == own {
            continue;
        }
        let Some(path) = process_path(pid) else { continue };
        let exe = path.rsplit('\\').next().unwrap_or(&path).to_string();
        if SKIP.contains(&exe.to_lowercase().as_str()) {
            continue;
        }
        let i = *by_path.entry(path.to_lowercase()).or_insert_with(|| {
            let stem = exe.strip_suffix(".exe").or(exe.strip_suffix(".EXE")).unwrap_or(&exe);
            apps.push(AudioApp {
                name: file_description(&path).unwrap_or_else(|| stem.to_string()),
                exe: exe.clone(),
                path: path.clone(),
                has_audio: false,
                playing: false,
            });
            apps.len() - 1
        });
        if let Some(active) = session {
            apps[i].has_audio = true;
            apps[i].playing |= active;
        }
    }
    apps.sort_by_cached_key(|a| (!a.has_audio, a.name.to_lowercase()));
    Ok(apps)
}
