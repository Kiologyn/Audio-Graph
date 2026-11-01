//! App devices: the same setting as "Settings → Sound → Volume mixer" per app.
//! Windows stores it per app (executable file) and applies it to running processes at once.
//! The default devices the "System" node sets are here too.
//!
//! Previous values are written to a journal on disk before a change: after the graph stops
//! they are restored. If the app is closed at that moment, restoring waits for it to start,
//! including after Audio Graph restarts or crashes.

use super::super::plan::{AppRoute, SystemRoute, Target};
use super::super::{AppState, AppStatus};
use crate::apps::win::{process_path, session_pids, session_volumes};
use crate::devices::win::{default_id, set_default};
use crate::storage::write_atomic;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs;
use std::path::{Path, PathBuf};
use windows::core::{Interface, IUnknown, GUID, HSTRING};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;

/// The undocumented factory Windows settings use to assign devices
/// to apps (EarTrumpet uses it too). The method order is the virtual function table
/// and must not change. Windows 11 and Windows 10 have this interface under different IIDs but with the same
/// table: the Windows 11 variant is declared, the Windows 10 one is queried by its own IID.
mod factory {
    #![allow(non_snake_case, dead_code)]

    use std::ffi::c_void;
    use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT};
    use windows::Win32::Media::Audio::{EDataFlow, ERole};

    #[interface("ab3d4648-e242-459f-b02f-541c70306324")]
    pub unsafe trait IAudioPolicyConfigFactory: IUnknown {
        // IInspectable
        fn GetIids(&self) -> HRESULT;
        fn GetRuntimeClassName(&self) -> HRESULT;
        fn GetTrustLevel(&self) -> HRESULT;
        // methods that aren't needed: only their slots in the table
        fn add_CtxVolumeChange(&self) -> HRESULT;
        fn remove_CtxVolumeChanged(&self) -> HRESULT;
        fn add_RingerVibrateStateChanged(&self) -> HRESULT;
        fn remove_RingerVibrateStateChange(&self) -> HRESULT;
        fn SetVolumeGroupGainForId(&self) -> HRESULT;
        fn GetVolumeGroupGainForId(&self) -> HRESULT;
        fn GetActiveVolumeGroupForEndpointId(&self) -> HRESULT;
        fn GetVolumeGroupsForEndpoint(&self) -> HRESULT;
        fn GetCurrentVolumeContext(&self) -> HRESULT;
        fn SetVolumeGroupMuteForId(&self) -> HRESULT;
        fn GetVolumeGroupMuteForId(&self) -> HRESULT;
        fn SetRingerVibrateState(&self) -> HRESULT;
        fn GetRingerVibrateState(&self) -> HRESULT;
        fn SetPreferredChatApplication(&self) -> HRESULT;
        fn ResetPreferredChatApplication(&self) -> HRESULT;
        fn GetPreferredChatApplication(&self) -> HRESULT;
        fn GetCurrentChatApplications(&self) -> HRESULT;
        fn add_ChatContextChanged(&self) -> HRESULT;
        fn remove_ChatContextChanged(&self) -> HRESULT;
        /// device is an HSTRING with the device path; an empty one clears the setting
        pub fn SetPersistedDefaultAudioEndpoint(&self, pid: u32, flow: EDataFlow, role: ERole, device: *mut c_void) -> HRESULT;
        pub fn GetPersistedDefaultAudioEndpoint(&self, pid: u32, flow: EDataFlow, role: ERole, device: *mut *mut c_void) -> HRESULT;
        fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
    }
}

use factory::IAudioPolicyConfigFactory;

/// IID of the same interface in Windows 10
const DOWNLEVEL_IID: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);

/// The device path in this setting: the MMDevice id with a prefix and the interface class
const PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER_SUFFIX: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const CAPTURE_SUFFIX: &str = "#{2eef81be-33fa-4800-9670-1cd474972c3f}";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum Flow {
    Render,
    Capture,
}

impl Flow {
    fn data_flow(self) -> EDataFlow {
        match self {
            Flow::Render => eRender,
            Flow::Capture => eCapture,
        }
    }
}

/// The value before the graph changed it
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Saved {
    /// Executable file name in lower case
    exe: String,
    flow: Flow,
    /// None: the app had no setting of its own and followed the system device
    previous: Option<String>,
}

/// Default device roles: console, multimedia and communications (calls). "System" changes all three
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Role {
    Console,
    Multimedia,
    Communications,
}

const ROLES: [Role; 3] = [Role::Console, Role::Multimedia, Role::Communications];

impl Role {
    fn erole(self) -> ERole {
        match self {
            Role::Console => eConsole,
            Role::Multimedia => eMultimedia,
            Role::Communications => eCommunications,
        }
    }
}

/// The default device before the graph changed it
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedDefault {
    flow: Flow,
    role: Role,
    previous: Option<String>,
}

/// The app's mixer volume before the graph changed it
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedVolume {
    exe: String,
    previous: f32,
}

#[derive(Serialize, Deserialize, Default)]
struct Journal {
    saved: Vec<Saved>,
    #[serde(default)]
    defaults: Vec<SavedDefault>,
    #[serde(default)]
    volumes: Vec<SavedVolume>,
}

pub struct Router {
    factory: Option<IAudioPolicyConfigFactory>,
    journal_path: PathBuf,
    saved: Vec<Saved>,
    defaults: Vec<SavedDefault>,
    volumes: Vec<SavedVolume>,
    /// What the engine itself assigned to processes, in case Windows doesn't allow reading it
    applied: HashMap<(u32, Flow), Option<String>>,
    /// Processes for which reading already failed (logged once)
    read_failed: HashSet<(u32, Flow)>,
}

/// An error with what exactly failed and the code: the log shows where Windows refused
fn describe(step: &str, flow: Flow, pid: u32, e: &windows::core::Error) -> String {
    let flow = if flow == Flow::Render { tr!("flow_output") } else { tr!("flow_input") };
    let code = format!("0x{:08X}", e.code().0 as u32);
    tr!(
        "error_app_device_step",
        step = step,
        flow = flow,
        pid = pid,
        message = e.message(),
        code = code
    )
}

/// Running processes: file name in lower case → pid
fn processes() -> HashMap<String, Vec<u32>> {
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return map;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            map.entry(exe).or_default().push(entry.th32ProcessID);
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    map
}

enum Found {
    /// A process with an audio session: Windows accepts only such a process in this setting,
    /// for the app's other processes (browsers and Discord have dozens) it says "invalid parameter"
    Audio(u32),
    /// The app runs but has never opened audio yet
    Silent,
    Absent,
}

/// The app's process with an audio session: one with the same path first, else any with the same
/// file name (an update may have changed the folder)
fn find(processes: &HashMap<String, Vec<u32>>, sessions: &HashMap<u32, bool>, exe: &str, path: &str) -> Found {
    let Some(pids) = processes.get(exe) else {
        return Found::Absent;
    };
    let audio: Vec<u32> = pids.iter().copied().filter(|pid| sessions.contains_key(pid)).collect();
    if audio.is_empty() {
        return Found::Silent;
    }
    if !path.is_empty() {
        if let Some(&pid) = audio
            .iter()
            .find(|&&pid| process_path(pid).is_some_and(|p| p.eq_ignore_ascii_case(path)))
        {
            return Found::Audio(pid);
        }
    }
    Found::Audio(audio[0])
}

impl Router {
    pub fn new(data_dir: &Path) -> Self {
        let journal_path = data_dir.join("app-routing.json");
        let journal = fs::read_to_string(&journal_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Journal>(&t).ok())
            .unwrap_or_default();
        Self {
            factory: None,
            journal_path,
            saved: journal.saved,
            defaults: journal.defaults,
            volumes: journal.volumes,
            applied: HashMap::new(),
            read_failed: HashSet::new(),
        }
    }

    fn factory(&mut self) -> Result<&IAudioPolicyConfigFactory, String> {
        if self.factory.is_none() {
            let class = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
            let fail = |e: windows::core::Error| tr!(
                "error_app_devices_denied",
                error = e.message()
            );
            let unknown = unsafe { RoGetActivationFactory::<IUnknown>(&class) }.map_err(fail)?;
            let factory = match unknown.cast::<IAudioPolicyConfigFactory>() {
                Ok(f) => f,
                Err(_) => {
                    // Windows 10 (before build 21390): the same interface under the old IID
                    let mut raw = std::ptr::null_mut();
                    unsafe { unknown.query(&DOWNLEVEL_IID, &mut raw) }.ok().map_err(fail)?;
                    unsafe { IAudioPolicyConfigFactory::from_raw(raw) }
                }
            };
            self.factory = Some(factory);
        }
        Ok(self.factory.as_ref().unwrap())
    }

    fn write_journal(&self) -> Result<(), String> {
        let journal = Journal {
            saved: self.saved.clone(),
            defaults: self.defaults.clone(),
            volumes: self.volumes.clone(),
        };
        let json = serde_json::to_vec_pretty(&journal).map_err(|e| e.to_string())?;
        write_atomic(&self.journal_path, &json).map_err(|e| tr!(
            "error_journal_write",
            e = e
        ))
    }

    /// The app's device; None means no setting of its own
    fn get(&mut self, pid: u32, flow: Flow) -> Result<Option<String>, String> {
        let factory = self.factory()?;
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe { factory.GetPersistedDefaultAudioEndpoint(pid, flow.data_flow(), eConsole, &mut raw) }
            .ok()
            .map_err(|e| describe(&tr!("step_read_app_device"), flow, pid, &e))?;
        // Windows handed over the string, HSTRING frees it
        let value: HSTRING = unsafe { std::mem::transmute(raw) };
        let value = value.to_string();
        let id = value.strip_prefix(PREFIX).unwrap_or(&value);
        let id = id.strip_suffix(RENDER_SUFFIX).or_else(|| id.strip_suffix(CAPTURE_SUFFIX)).unwrap_or(id);
        Ok((!id.is_empty()).then(|| id.to_string()))
    }

    fn set(&mut self, pid: u32, flow: Flow, device: Option<&str>) -> Result<(), String> {
        let factory = self.factory()?;
        let suffix = if flow == Flow::Render { RENDER_SUFFIX } else { CAPTURE_SUFFIX };
        let value = device.map(|id| HSTRING::from(format!("{PREFIX}{id}{suffix}"))).unwrap_or_default();
        // HSTRING is a pointer; an empty (null) one clears the setting
        let raw: *mut c_void = unsafe { std::mem::transmute_copy(&value) };
        for role in [eMultimedia, eConsole] {
            unsafe { factory.SetPersistedDefaultAudioEndpoint(pid, flow.data_flow(), role, raw) }
                .ok()
                .map_err(|e| describe(&tr!("step_set_app_device"), flow, pid, &e))?;
        }
        Ok(())
    }

    /// Sets an app's device after remembering the previous one
    fn ensure(&mut self, pid: u32, exe: &str, flow: Flow, want: Option<&str>) -> Result<(), String> {
        // on Windows 10 reading can be unavailable (EarTrumpet doesn't need it either): then the current
        // value is what we set ourselves, and before that, that the app had no setting of its own
        let current = match self.get(pid, flow) {
            Ok(current) => current,
            Err(e) => {
                if self.read_failed.insert((pid, flow)) {
                    crate::log::write(format!("failed to read, going on without it: {e}"));
                }
                self.applied.get(&(pid, flow)).cloned().flatten()
            }
        };
        if current.as_deref() == want {
            return Ok(());
        }
        if !self.saved.iter().any(|s| s.exe == exe && s.flow == flow) {
            self.saved.push(Saved {
                exe: exe.to_string(),
                flow,
                previous: current,
            });
            self.write_journal()?;
        }
        self.set(pid, flow, want)?;
        self.applied.insert((pid, flow), want.map(str::to_string));
        Ok(())
    }

    /// Sets the default devices from the "System" node, and restores the previous ones without it
    pub fn apply_system(&mut self, system: Option<&SystemRoute>) -> Option<AppStatus> {
        let wanted = [
            (Flow::Render, system.and_then(|s| s.render.clone())),
            (Flow::Capture, system.and_then(|s| s.capture.clone())),
        ];
        let mut error: Option<String> = None;
        let mut changed = false;
        for (flow, want) in wanted {
            match want {
                Some(id) => {
                    for role in ROLES {
                        let current = default_id(flow.data_flow(), role.erole());
                        if current.as_deref() == Some(id.as_str()) {
                            continue;
                        }
                        if !self.defaults.iter().any(|d| d.flow == flow && d.role == role) {
                            self.defaults.push(SavedDefault { flow, role, previous: current });
                            changed = true;
                            // the previous device is journaled before the change: it can be restored after a crash
                            if let Err(e) = self.write_journal() {
                                error.get_or_insert(e);
                                continue;
                            }
                        }
                        if let Err(e) = set_default(&id, role.erole()) {
                            error.get_or_insert(e);
                        }
                    }
                }
                None => {
                    for saved in self.defaults.iter().filter(|d| d.flow == flow) {
                        // the former default device may be gone: then Windows has chosen another
                        if let Some(previous) = &saved.previous {
                            let _ = set_default(previous, saved.role.erole());
                        }
                    }
                    let before = self.defaults.len();
                    self.defaults.retain(|d| d.flow != flow);
                    changed |= self.defaults.len() != before;
                }
            }
        }
        if changed {
            let _ = self.write_journal();
        }
        system.map(|s| AppStatus {
            node: s.node.clone(),
            state: if error.is_some() { AppState::Failed } else { AppState::Routed },
            error,
        })
    }

    /// Brings app devices in line with the plan and restores the previous ones for apps not in it
    pub fn apply(&mut self, routes: &[AppRoute]) -> Vec<AppStatus> {
        let processes = processes();
        let sessions = session_pids().unwrap_or_default();
        let mut wanted: HashSet<(String, Flow)> = HashSet::new();
        let mut statuses = Vec::new();

        for route in routes {
            let exe = route.app.exe.to_lowercase();
            let targets = [(Flow::Render, &route.render), (Flow::Capture, &route.capture)];
            for (flow, target) in &targets {
                if target.is_some() {
                    wanted.insert((exe.clone(), *flow));
                }
            }
            let pid = match find(&processes, &sessions, &exe, &route.app.path) {
                Found::Audio(pid) => pid,
                found => {
                    statuses.push(AppStatus {
                        node: route.node.clone(),
                        state: if matches!(found, Found::Silent) { AppState::NoAudio } else { AppState::NotRunning },
                        error: None,
                    });
                    continue;
                }
            };
            let mut error = None;
            for (flow, target) in targets {
                let want = match target {
                    None => continue,
                    Some(Target::Default) => None,
                    Some(Target::Device(id)) => Some(id.as_str()),
                };
                if let Err(e) = self.ensure(pid, &exe, flow, want) {
                    error.get_or_insert(e);
                }
            }
            statuses.push(AppStatus {
                node: route.node.clone(),
                state: if error.is_some() { AppState::Failed } else { AppState::Routed },
                error,
            });
        }

        // restoring previous devices
        let mut kept = Vec::new();
        let mut restored = false;
        for saved in std::mem::take(&mut self.saved) {
            if wanted.contains(&(saved.exe.clone(), saved.flow)) {
                kept.push(saved);
                continue;
            }
            match find(&processes, &sessions, &saved.exe, "") {
                Found::Audio(pid) if self.set(pid, saved.flow, saved.previous.as_deref()).is_ok() => restored = true,
                // the app is closed or without audio: restore when it opens audio
                _ => kept.push(saved),
            }
        }
        self.saved = kept;
        if restored {
            let _ = self.write_journal();
        }
        self.apply_volumes(routes, &processes);
        statuses
    }

    /// App volume in the Windows mixer. At 100% the graph leaves the app's volume alone,
    /// and if it changed it before, restores the previous one
    fn apply_volumes(&mut self, routes: &[AppRoute], processes: &HashMap<String, Vec<u32>>) {
        let volumes = session_volumes();
        let controls = |exe: &str| -> Vec<&windows::Win32::Media::Audio::ISimpleAudioVolume> {
            processes
                .get(exe)
                .into_iter()
                .flatten()
                .filter_map(|pid| volumes.get(pid))
                .flatten()
                .collect()
        };
        let mut changed = false;
        let mut wanted = HashSet::new();
        for route in routes.iter().filter(|r| (r.volume - 1.0).abs() > 0.001) {
            let exe = route.app.exe.to_lowercase();
            wanted.insert(exe.clone());
            let list = controls(&exe);
            let Some(first) = list.first() else { continue };
            if !self.volumes.iter().any(|v| v.exe == exe) {
                let previous = unsafe { first.GetMasterVolume() }.unwrap_or(1.0);
                self.volumes.push(SavedVolume { exe: exe.clone(), previous });
                changed = true;
            }
            for c in list {
                if unsafe { c.GetMasterVolume() }.is_ok_and(|v| (v - route.volume).abs() > 0.005) {
                    let _ = unsafe { c.SetMasterVolume(route.volume, std::ptr::null()) };
                }
            }
        }
        let mut kept = Vec::new();
        for saved in std::mem::take(&mut self.volumes) {
            let list = controls(&saved.exe);
            if wanted.contains(&saved.exe) || list.is_empty() {
                // still needed or the app has no audio: restore later
                kept.push(saved);
                continue;
            }
            for c in list {
                let _ = unsafe { c.SetMasterVolume(saved.previous, std::ptr::null()) };
            }
            changed = true;
        }
        self.volumes = kept;
        if changed {
            let _ = self.write_journal();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::plan::Target;
    use super::*;
    use crate::com::ComGuard;
    use crate::graphs::AppRef;

    /// Sets an output device for the test itself (its executable file) and restores the previous one:
    /// checks the undocumented interface and the journal. Other apps are left alone:
    /// cargo test app_routing -- --ignored --nocapture
    #[test]
    #[ignore]
    fn app_routing_own_process() {
        let _com = ComGuard::init();
        let vac = crate::devices::list()
            .unwrap()
            .into_iter()
            .find(|d| d.name.contains("Virtual Audio Cable") && d.flow == crate::devices::Flow::Render)
            .expect("no Virtual Audio Cable")
            .id;
        // Windows accepts the setting only for a process with an audio session: open a stream on VAC
        let _session = unsafe {
            use windows::core::PCWSTR;
            use windows::Win32::Media::Audio::{IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED};
            use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let id = crate::com::wide(&vac);
            let client: IAudioClient = enumerator.GetDevice(PCWSTR(id.as_ptr())).unwrap().Activate(CLSCTX_ALL, None).unwrap();
            let format = client.GetMixFormat().unwrap();
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 200_000, 0, format, None).unwrap();
            client
        };
        let dir = std::env::temp_dir().join("audio-graph-router-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut router = Router::new(&dir);
        let pid = std::process::id();
        let path = std::env::current_exe().unwrap().to_string_lossy().to_string();
        let exe = path.rsplit('\\').next().unwrap().to_string();

        let before = router.get(pid, Flow::Render).unwrap();
        println!("before: {before:?}");
        let route = AppRoute {
            node: "n".into(),
            app: AppRef { exe, path, name: "test".into() },
            render: Some(Target::Device(vac.clone())),
            capture: None,
            volume: 0.5,
        };
        let volume = || {
            session_volumes()
                .get(&pid)
                .and_then(|v| v.first().map(|c| unsafe { c.GetMasterVolume() }.unwrap()))
        };
        let volume_before = volume();
        let statuses = router.apply(&[route]);
        let during = router.get(pid, Flow::Render).unwrap();
        let volume_during = volume();
        let journal = fs::read_to_string(dir.join("app-routing.json")).unwrap_or_default();
        println!("during: {during:?} volume {volume_during:?} {statuses:?}\njournal: {journal}");

        router.apply(&[]);
        let after = router.get(pid, Flow::Render).unwrap();
        println!("after: {after:?} volume {:?}", volume());
        assert_eq!(statuses[0].state, AppState::Routed);
        assert_eq!(during.as_deref(), Some(vac.as_str()));
        assert_eq!(volume_during, Some(0.5));
        assert_eq!(after, before);
        assert_eq!(volume(), volume_before);
        assert!(router.saved.is_empty() && router.volumes.is_empty());
    }
}

