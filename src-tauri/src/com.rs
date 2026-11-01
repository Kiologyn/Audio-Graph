//! Shared code for working with COM on Windows.

use windows::core::PWSTR;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};

/// COM on the current thread while working with the system.
/// Declared first so that it is released after all COM objects.
pub struct ComGuard(bool);

impl ComGuard {
    pub fn init() -> Self {
        // the thread may already be initialized in another mode: COM works then, but
        // CoUninitialize must not be called
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

/// A string allocated by COM, freed on drop
pub unsafe fn take_string(p: PWSTR) -> Option<String> {
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as _));
    s
}

/// A string for the Windows API: UTF-16 with a terminating zero
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
