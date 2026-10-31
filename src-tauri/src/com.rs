//! Общее для работы с COM на Windows.

use windows::core::PWSTR;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED};

/// COM на текущем потоке на время работы с системой.
/// Объявляется первым, чтобы освободиться после всех COM-объектов.
pub struct ComGuard(bool);

impl ComGuard {
    pub fn init() -> Self {
        // поток мог уже быть инициализирован в другом режиме: COM тогда работает, но
        // CoUninitialize вызывать нельзя
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

/// Строка, выделенная COM, с освобождением памяти
pub unsafe fn take_string(p: PWSTR) -> Option<String> {
    let s = p.to_string().ok();
    CoTaskMemFree(Some(p.0 as _));
    s
}

/// Строка для Windows API: UTF-16 с нулём в конце
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
