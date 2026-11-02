//! Devices through Windows Core Audio (MMDevice API) and the undocumented IPolicyConfig.

use super::pairs::{self, Endpoint};
use super::{AudioDevice, DeviceState, Flow};
use crate::com::{take_string, wide, ComGuard};
use windows::core::{Result, GUID, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{E_ACCESSDENIED, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe { Ok(take_string(device.GetId()?).unwrap_or_default()) }
}

const fn key(guid: u128, pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: GUID::from_u128(guid), pid }
}

/// The device's own name without the driver name: "Line 1" (PKEY_Device_DeviceDesc)
const PKEY_DEVICE_DESC: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 2);
/// The bus of the driver device: "ROOT", "USB" (PKEY_Device_EnumeratorName)
const PKEY_ENUMERATOR: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 24);
/// The driver device the endpoint belongs to: "{1}.ROOT\{83ED7F0E-...}\0000"
const PKEY_DRIVER_DEVICE: PROPERTYKEY = key(0xb3f8fa53_0004_438e_9003_51a46e139bfc, 2);
/// The driver device's name: "Virtual Audio Cable"
const PKEY_DRIVER_NAME: PROPERTYKEY = key(0xb3f8fa53_0004_438e_9003_51a46e139bfc, 6);
/// The hardware id the driver was installed for: "VBVoicemeeterVAIO" (DEVPKEY_Device_MatchingDeviceId)
const PKEY_HARDWARE: PROPERTYKEY = key(0xa8b865dd_2e3d_4094_ad97_e593a70c75d6, 8);
/// The driver filter the endpoint is connected to: "{2}.\\?\root#...#{...}\wave1_r_rt"
const PKEY_FILTER: PROPERTYKEY = key(0x233164c8_1b2c_4c7d_bc68_b671687a2567, 1);

fn string_property(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let text = PropVariantToStringAlloc(&value).ok().and_then(|p| take_string(p));
        let _ = PropVariantClear(&mut value);
        text.filter(|s| !s.is_empty())
    }
}

/// What the driver tells about the device, to find the other side of a virtual cable
fn endpoint(store: &IPropertyStore, id: &str, render: bool) -> Endpoint {
    let get = |key| string_property(store, key).unwrap_or_default();
    // the values start with a "{n}." type mark
    let unmark = |s: String| s.split_once("}.").map_or_else(|| s.clone(), |(_, rest)| rest.to_string());
    Endpoint {
        id: id.to_string(),
        render,
        name: get(&PKEY_DEVICE_DESC),
        driver: unmark(get(&PKEY_DRIVER_DEVICE)),
        driver_name: get(&PKEY_DRIVER_NAME),
        hardware: get(&PKEY_HARDWARE),
        enumerator: get(&PKEY_ENUMERATOR),
        filter: get(&PKEY_FILTER).rsplit('\\').next().unwrap_or_default().to_string(),
    }
}

fn list_flow(enumerator: &IMMDeviceEnumerator, flow: EDataFlow, kind: Flow) -> Result<Vec<(AudioDevice, Endpoint)>> {
    unsafe {
        // there may be no default device at all (for example, no microphone)
        let default_id = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .ok()
            .and_then(|d| device_id(&d).ok());
        // disabled ones are needed too: they are shown and turned back on.
        // Unplugged jacks and removed devices are skipped, they can't be turned on.
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
            let store = device.OpenPropertyStore(STGM_READ).ok();
            let endpoint = store.as_ref().map_or_else(Endpoint::default, |s| endpoint(s, &id, kind == Flow::Render));
            devices.push((
                AudioDevice {
                    name: store
                        .as_ref()
                        .and_then(|s| string_property(s, &PKEY_Device_FriendlyName))
                        .unwrap_or_else(|| tr!("device_unnamed")),
                    is_default: default_id.as_deref() == Some(id.as_str()),
                    is_virtual: false,
                    pair: None,
                    pair_name: None,
                    flow: kind,
                    state,
                    id,
                },
                endpoint,
            ));
        }
        Ok(devices)
    }
}

pub fn defaults() -> (Option<String>, Option<String>) {
    (default_id(eRender, eConsole), default_id(eCapture, eConsole))
}

/// The default device for a direction and role (console, multimedia, communications)
pub fn default_id(flow: EDataFlow, role: ERole) -> Option<String> {
    let _com = ComGuard::init();
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) }.ok()?;
    device_id(&device).ok()
}

pub fn list() -> Result<Vec<AudioDevice>> {
    let _com = ComGuard::init();
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
    let mut listed = list_flow(&enumerator, eRender, Flow::Render)?;
    listed.extend(list_flow(&enumerator, eCapture, Flow::Capture)?);
    let (mut devices, endpoints): (Vec<AudioDevice>, Vec<Endpoint>) = listed.into_iter().unzip();
    for link in pairs::find(&endpoints) {
        if let Some(d) = devices.iter_mut().find(|d| d.id == link.device) {
            d.pair = Some(link.other);
            d.pair_name = Some(link.name);
        }
    }
    Ok(devices)
}

/// The undocumented interface the Sound control panel uses to turn devices on and off
/// and change the default device (EarTrumpet, SoundSwitch and AudioDeviceCmdlets
/// use it too). The method order is the virtual function table and must not change:
/// a mistake shifts calls to neighboring methods.
mod policy {
    // the interface macro takes no attributes on the trait, so the allows are module-wide:
    // method names are as in Windows, and most methods aren't used
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

/// Makes a device the default for a role, like the button in the Sound control panel
pub fn set_default(id: &str, role: ERole) -> std::result::Result<(), String> {
    let _com = ComGuard::init();
    let policy: IPolicyConfig = unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
        .map_err(|e| tr!("error_device_control_unavailable", e = e))?;
    let id = wide(id);
    unsafe { policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }
        .ok()
        .map_err(|e| e.message())
}

pub fn set_enabled(ids: &[String], enabled: bool) -> std::result::Result<(), String> {
    let _com = ComGuard::init();
    let policy: IPolicyConfig = unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
        .map_err(|e| tr!("error_device_control_unavailable", e = e))?;
    // try all devices even if one didn't switch
    let mut first_error = None;
    for id in ids {
        let id = wide(id);
        let result = unsafe { policy.SetEndpointVisibility(PCWSTR(id.as_ptr()), enabled.into()) }.ok();
        if let Err(e) = result {
            first_error.get_or_insert(if e.code() == E_ACCESSDENIED {
                tr!("error_device_state_admin")
            } else {
                e.to_string()
            });
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The system's default devices of every role, put back when dropped. Tests that turn devices off
/// and on keep it: Windows moves the default away from a device turned off and doesn't move it back
#[cfg(test)]
pub struct KeepDefaults(Vec<(String, ERole)>);

#[cfg(test)]
impl KeepDefaults {
    pub fn new() -> Self {
        use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, eMultimedia};
        let roles = [eConsole, eMultimedia, eCommunications];
        let defaults = [eRender, eCapture]
            .into_iter()
            .flat_map(|flow| roles.map(|role| default_id(flow, role).map(|id| (id, role))))
            .flatten()
            .collect();
        Self(defaults)
    }
}

#[cfg(test)]
impl Drop for KeepDefaults {
    fn drop(&mut self) {
        for (id, role) in &self.0 {
            let _ = set_default(id, *role);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the drivers tell about this machine's devices, to teach pairs.rs a new driver:
    /// cargo test driver_properties -- --nocapture
    #[test]
    fn prints_driver_properties() {
        let _com = ComGuard::init();
        let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.unwrap();
        for (flow, kind) in [(eRender, Flow::Render), (eCapture, Flow::Capture)] {
            for (d, e) in list_flow(&enumerator, flow, kind).unwrap() {
                println!("{kind:?} {} | {:?} {} on {} | hardware {} | filter {}", d.name, e.driver_name, e.driver, e.enumerator, e.hardware, e.filter);
                assert!(e.driver.is_empty() || !e.driver_name.is_empty(), "{} has a driver device without a name", d.name);
            }
        }
    }
}
