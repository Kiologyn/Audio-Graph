//! Устройства через Windows Core Audio (MMDevice API) и недокументированный IPolicyConfig.

use super::{AudioDevice, DeviceState, Flow};
use crate::com::{take_string, wide, ComGuard};
use windows::core::{Result, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe { Ok(take_string(device.GetId()?).unwrap_or_default()) }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = PropVariantToStringAlloc(&value).ok().and_then(|p| take_string(p));
        let _ = PropVariantClear(&mut value);
        name
    }
}

fn list_flow(enumerator: &IMMDeviceEnumerator, flow: EDataFlow, kind: Flow) -> Result<Vec<AudioDevice>> {
    unsafe {
        // устройства по умолчанию может не быть вовсе (например, нет ни одного микрофона)
        let default_id = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .ok()
            .and_then(|d| device_id(&d).ok());
        // выключенные тоже нужны: их показывают и включают обратно.
        // Отсоединённые гнёзда и удалённые устройства не берём, их не включить.
        let mask = DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0);
        let collection = enumerator.EnumAudioEndpoints(flow, mask)?;
        let mut devices = Vec::new();
        for i in 0..collection.GetCount()? {
            let device = collection.Item(i)?;
            let id = device_id(&device)?;
            let state = if device.GetState()?.0 == DEVICE_STATE_ACTIVE.0 {
                DeviceState::Active
            } else {
                DeviceState::Disabled
            };
            devices.push(AudioDevice {
                name: friendly_name(&device).unwrap_or_else(|| tr!("Без названия")),
                is_default: default_id.as_deref() == Some(id.as_str()),
                flow: kind,
                state,
                id,
            });
        }
        Ok(devices)
    }
}

pub fn defaults() -> (Option<String>, Option<String>) {
    (default_id(eRender, eConsole), default_id(eCapture, eConsole))
}

/// Устройство по умолчанию для направления и роли (обычное, мультимедиа, связь)
pub fn default_id(flow: EDataFlow, role: ERole) -> Option<String> {
    let _com = ComGuard::init();
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) }.ok()?;
    device_id(&device).ok()
}

pub fn list() -> Result<Vec<AudioDevice>> {
    let _com = ComGuard::init();
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let mut devices = list_flow(&enumerator, eRender, Flow::Render)?;
    devices.extend(list_flow(&enumerator, eCapture, Flow::Capture)?);
    Ok(devices)
}

/// Недокументированный интерфейс, через который панель «Звук» включает и выключает
/// устройства и меняет устройство по умолчанию (им же пользуются EarTrumpet, SoundSwitch,
/// AudioDeviceCmdlets). Порядок методов — это таблица виртуальных функций, менять его нельзя:
/// ошибка сдвигает вызовы на соседние методы.
mod policy {
    // макрос interface не принимает атрибуты на трейте, поэтому разрешения на весь модуль:
    // имена методов как в Windows, а большая часть методов нам не нужна
    #![allow(non_snake_case, dead_code)]

    use std::ffi::c_void;
    use windows::core::{interface, IUnknown, IUnknown_Vtbl, BOOL, GUID, HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::ERole;

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device: PCWSTR, default: BOOL, format: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, device: PCWSTR, endpoint_format: *mut c_void, mix_format: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, device: PCWSTR, default: BOOL, default_period: *mut i64, min_period: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, device: PCWSTR, fx_store: BOOL, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, device: PCWSTR, fx_store: BOOL, key: *const c_void, value: *mut c_void) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        pub fn SetEndpointVisibility(&self, device: PCWSTR, visible: BOOL) -> HRESULT;
    }

    pub const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
}

use policy::{IPolicyConfig, CLSID_POLICY_CONFIG_CLIENT};

/// Делает устройство устройством по умолчанию для роли — как кнопка в панели «Звук»
pub fn set_default(id: &str, role: ERole) -> std::result::Result<(), String> {
    let _com = ComGuard::init();
    let policy: IPolicyConfig = unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
        .map_err(|e| tr!("управление устройствами недоступно: {e}", e = e))?;
    let id = wide(id);
    unsafe { policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }
        .ok()
        .map_err(|e| e.message())
}

pub fn set_enabled(ids: &[String], enabled: bool) -> std::result::Result<(), String> {
    let _com = ComGuard::init();
    let policy: IPolicyConfig = unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
        .map_err(|e| tr!("управление устройствами недоступно: {e}", e = e))?;
    // пробуем все устройства, даже если какое-то не переключилось
    let mut first_error = None;
    for id in ids {
        let id = wide(id);
        let result = unsafe { policy.SetEndpointVisibility(PCWSTR(id.as_ptr()), enabled.into()) }.ok();
        if let Err(e) = result {
            first_error.get_or_insert(if e.code() == E_ACCESSDENIED {
                tr!("Windows не разрешила изменить состояние устройства: нужны права администратора")
            } else {
                e.to_string()
            });
        }
    }
    first_error.map_or(Ok(()), Err)
}
