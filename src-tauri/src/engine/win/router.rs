//! Устройства приложений — та же настройка, что «Параметры → Звук → Громкость приложений».
//! Windows хранит её на приложение (исполняемый файл) и применяет к запущенным процессам сразу.
//! Здесь же устройства по умолчанию, которые назначает нода «Система».
//!
//! Прежние значения записываются в журнал на диске до изменения: после остановки графа
//! они возвращаются. Если приложение в этот момент закрыто, возврат ждёт его запуска —
//! в том числе после перезапуска Audio Graph или его падения.

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

/// Недокументированная фабрика, через которую настройки Windows назначают устройства
/// приложениям (ей же пользуется EarTrumpet). Порядок методов — таблица виртуальных функций,
/// менять его нельзя. У Windows 11 и у Windows 10 этот интерфейс с разными IID, но с одинаковой
/// таблицей: объявлен вариант Windows 11, вариант Windows 10 запрашивается по своему IID.
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
        // методы, которые не нужны: только место в таблице
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
        /// device — HSTRING с путём устройства; пустой снимает настройку
        pub fn SetPersistedDefaultAudioEndpoint(&self, pid: u32, flow: EDataFlow, role: ERole, device: *mut c_void) -> HRESULT;
        pub fn GetPersistedDefaultAudioEndpoint(&self, pid: u32, flow: EDataFlow, role: ERole, device: *mut *mut c_void) -> HRESULT;
        fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
    }
}

use factory::IAudioPolicyConfigFactory;

/// IID того же интерфейса в Windows 10
const DOWNLEVEL_IID: GUID = GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);

/// Путь устройства в этой настройке: id из MMDevice с префиксом и классом интерфейса
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

/// Значение до того, как его поменял граф
#[derive(Serialize, Deserialize, Clone, Debug)]
struct Saved {
    /// Имя исполняемого файла в нижнем регистре
    exe: String,
    flow: Flow,
    /// None — у приложения не было своей настройки, оно шло за системным устройством
    previous: Option<String>,
}

/// Роли устройства по умолчанию: обычное, мультимедиа и связь (звонки). «Система» меняет все три
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

/// Устройство по умолчанию до того, как его поменял граф
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SavedDefault {
    flow: Flow,
    role: Role,
    previous: Option<String>,
}

/// Громкость приложения в микшере до того, как её поменял граф
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
    /// Что движок сам назначил процессам — на случай, если Windows не даёт прочитать
    applied: HashMap<(u32, Flow), Option<String>>,
    /// Процессы, для которых чтение уже не удалось (в журнал — один раз)
    read_failed: HashSet<(u32, Flow)>,
}

/// Ошибка с тем, что именно не удалось, и кодом: по журналу видно, где Windows отказала
fn describe(step: &str, flow: Flow, pid: u32, e: &windows::core::Error) -> String {
    let flow = if flow == Flow::Render { tr!("вывод") } else { tr!("ввод") };
    let code = format!("0x{:08X}", e.code().0 as u32);
    tr!(
        "{step} ({flow}, процесс {pid}): {message} [{code}]",
        step = step,
        flow = flow,
        pid = pid,
        message = e.message(),
        code = code
    )
}

/// Запущенные процессы: имя файла в нижнем регистре → pid
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
    /// Процесс со звуковым сеансом: только его Windows принимает в этой настройке,
    /// для остальных процессов приложения (у браузеров и Discord их десятки) — «неверный параметр»
    Audio(u32),
    /// Приложение запущено, но ещё ни разу не открывало звук
    Silent,
    Absent,
}

/// Процесс приложения со звуковым сеансом: сначала с тем же путём, иначе любой с тем же
/// именем файла (после обновления папка могла смениться)
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
                "Windows не дала менять устройства приложений: {error}",
                error = e.message()
            );
            let unknown = unsafe { RoGetActivationFactory::<IUnknown>(&class) }.map_err(fail)?;
            let factory = match unknown.cast::<IAudioPolicyConfigFactory>() {
                Ok(f) => f,
                Err(_) => {
                    // Windows 10 (до сборки 21390): тот же интерфейс под старым IID
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
            "не удалось записать журнал устройств: {e}",
            e = e
        ))
    }

    /// Устройство приложения; None — своей настройки нет
    fn get(&mut self, pid: u32, flow: Flow) -> Result<Option<String>, String> {
        let factory = self.factory()?;
        let mut raw: *mut c_void = std::ptr::null_mut();
        unsafe { factory.GetPersistedDefaultAudioEndpoint(pid, flow.data_flow(), eConsole, &mut raw) }
            .ok()
            .map_err(|e| describe(&tr!("чтение устройства приложения"), flow, pid, &e))?;
        // строку отдала Windows, HSTRING освободит её
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
        // HSTRING — указатель; пустой (нулевой) снимает настройку
        let raw: *mut c_void = unsafe { std::mem::transmute_copy(&value) };
        for role in [eMultimedia, eConsole] {
            unsafe { factory.SetPersistedDefaultAudioEndpoint(pid, flow.data_flow(), role, raw) }
                .ok()
                .map_err(|e| describe(&tr!("назначение устройства приложению"), flow, pid, &e))?;
        }
        Ok(())
    }

    /// Ставит приложению устройство, сначала запомнив прежнее
    fn ensure(&mut self, pid: u32, exe: &str, flow: Flow, want: Option<&str>) -> Result<(), String> {
        // в Windows 10 чтение бывает недоступно (EarTrumpet тоже его не требует): тогда считаем
        // текущим то, что ставили сами, а до этого — что своей настройки у приложения не было
        let current = match self.get(pid, flow) {
            Ok(current) => current,
            Err(e) => {
                if self.read_failed.insert((pid, flow)) {
                    crate::log::write(format!("не удалось прочитать, продолжаю без него: {e}"));
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

    /// Ставит устройства по умолчанию по ноде «Система», а без неё возвращает прежние
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
                            // прежнее устройство записано до изменения: после сбоя его можно вернуть
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
                        // устройства, которое было по умолчанию, могло уже не стать — тогда Windows выбрала другое
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

    /// Приводит устройства приложений к плану и возвращает прежние тем, кого в плане нет
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

        // возврат прежних устройств
        let mut kept = Vec::new();
        let mut restored = false;
        for saved in std::mem::take(&mut self.saved) {
            if wanted.contains(&(saved.exe.clone(), saved.flow)) {
                kept.push(saved);
                continue;
            }
            match find(&processes, &sessions, &saved.exe, "") {
                Found::Audio(pid) if self.set(pid, saved.flow, saved.previous.as_deref()).is_ok() => restored = true,
                // приложение закрыто или без звука: вернём, когда оно откроет звук
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

    /// Громкость приложений в микшере Windows. На 100% граф громкость приложения не трогает,
    /// а если трогал раньше — возвращает прежнюю
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
                // ещё нужна или приложение без звука: вернём позже
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

    /// Назначает устройство вывода самому тесту (его исполняемому файлу) и возвращает прежнее:
    /// проверяет недокументированный интерфейс и журнал. Другие приложения не трогает:
    /// cargo test app_routing -- --ignored --nocapture
    #[test]
    #[ignore]
    fn app_routing_own_process() {
        let _com = ComGuard::init();
        let vac = crate::devices::list()
            .unwrap()
            .into_iter()
            .find(|d| d.name.contains("Virtual Audio Cable") && d.flow == crate::devices::Flow::Render)
            .expect("нет Virtual Audio Cable")
            .id;
        // Windows принимает настройку только у процесса со звуковым сеансом: открываем поток на VAC
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
        println!("до: {before:?}");
        let route = AppRoute {
            node: "n".into(),
            app: AppRef { exe, path, name: "тест".into() },
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
        println!("во время: {during:?} громкость {volume_during:?} {statuses:?}\nжурнал: {journal}");

        router.apply(&[]);
        let after = router.get(pid, Flow::Render).unwrap();
        println!("после: {after:?} громкость {:?}", volume());
        assert_eq!(statuses[0].state, AppState::Routed);
        assert_eq!(during.as_deref(), Some(vac.as_str()));
        assert_eq!(volume_during, Some(0.5));
        assert_eq!(after, before);
        assert_eq!(volume(), volume_before);
        assert!(router.saved.is_empty() && router.volumes.is_empty());
    }
}

