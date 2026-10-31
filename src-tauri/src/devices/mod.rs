//! Список аудиоустройств системы. Перечисление зависит от платформы,
//! снаружи модуль одинаковый.

use serde::Serialize;

#[cfg(windows)]
pub(crate) mod win;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    /// Устройство вывода: в него играют (колонки, наушники, вход кабеля)
    Render,
    /// Устройство ввода: с него записывают (микрофон, выход кабеля)
    Capture,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Active,
    /// Выключено в настройках звука; можно включить программно
    Disabled,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Системный идентификатор устройства, на Windows — endpoint ID
    pub id: String,
    pub name: String,
    pub flow: Flow,
    pub state: DeviceState,
    /// Устройство по умолчанию в системе для своего направления
    pub is_default: bool,
}

/// Включённые и выключенные устройства вывода и ввода
pub fn list() -> Result<Vec<AudioDevice>, String> {
    #[cfg(windows)]
    return win::list().map_err(|e| e.to_string());

    // Linux (PipeWire) и macOS пока не реализованы
    #[cfg(not(windows))]
    Ok(Vec::new())
}

/// Включает или выключает устройства, как это делают настройки звука системы
pub fn set_enabled(ids: &[String], enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    return win::set_enabled(ids, enabled);

    #[cfg(not(windows))]
    {
        let _ = (ids, enabled);
        Err(tr!("включение и выключение устройств пока есть только на Windows"))
    }
}

/// Текущие устройства по умолчанию (вывод, ввод); вызывается из потока движка
pub fn defaults() -> (Option<String>, Option<String>) {
    #[cfg(windows)]
    return win::defaults();
    #[cfg(not(windows))]
    (None, None)
}

/// Перечисление идёт через COM, поэтому выполняется вне потока окна
#[tauri::command]
pub async fn audio_devices() -> Result<Vec<AudioDevice>, String> {
    tauri::async_runtime::spawn_blocking(list)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_devices_enabled(ids: Vec<String>, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_enabled(&ids, enabled))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Печатает устройства этой машины и время перечисления:
    /// cargo test devices -- --nocapture
    #[test]
    fn lists_devices() {
        let started = std::time::Instant::now();
        let devices = list().expect("перечисление устройств");
        println!("{} устройств за {:?}", devices.len(), started.elapsed());
        for d in &devices {
            println!("{:?} {:?}{} {}  [{}]", d.flow, d.state, if d.is_default { " *" } else { "" }, d.name, d.id);
        }
    }

    /// Назначает устройствами по умолчанию те же, что уже стоят: проверяет вызов, ничего не меняя.
    /// cargo test sets_same_default -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn sets_same_default() {
        use windows::Win32::Media::Audio::{eCapture, eCommunications, eConsole, eMultimedia, eRender};
        for flow in [eRender, eCapture] {
            for role in [eConsole, eMultimedia, eCommunications] {
                let Some(id) = win::default_id(flow, role) else { continue };
                let result = win::set_default(&id, role);
                println!("{flow:?} {role:?}: {id} → {result:?}");
                assert!(result.is_ok());
                assert_eq!(win::default_id(flow, role).as_deref(), Some(id.as_str()));
            }
        }
    }

    fn state_of(id: &str) -> Option<DeviceState> {
        list().ok()?.into_iter().find(|d| d.id == id).map(|d| d.state)
    }

    /// Переключает реальное устройство записи Virtual Audio Cable и сразу включает обратно.
    /// Меняет состояние системы, поэтому запускается только явно:
    /// cargo test toggles_vac -- --ignored --nocapture
    #[test]
    #[ignore]
    fn toggles_vac_capture() {
        let vac: Vec<_> = list()
            .unwrap()
            .into_iter()
            .filter(|d| d.name.contains("Virtual Audio Cable"))
            .collect();
        assert!(!vac.is_empty(), "устройств Virtual Audio Cable нет");
        let ids: Vec<String> = vac.iter().map(|d| d.id.clone()).collect();

        // включение уже включённых устройств ничего не меняет, но проверяет вызов и права
        println!("включение (без изменений): {:?}", set_enabled(&ids, true));

        let capture = vac.iter().find(|d| d.flow == Flow::Capture).expect("устройство записи VAC");
        let id = vec![capture.id.clone()];
        let disabled = set_enabled(&id, false);
        println!("выключение: {disabled:?}, состояние: {:?}", state_of(&capture.id));
        // включаем обратно в любом случае
        let enabled = set_enabled(&id, true);
        println!("включение: {enabled:?}, состояние: {:?}", state_of(&capture.id));
        assert_eq!(state_of(&capture.id), Some(DeviceState::Active), "устройство должно снова быть включено");
    }
}
