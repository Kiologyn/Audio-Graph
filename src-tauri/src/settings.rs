//! Настройки приложения: тема, язык, поведение кабелей. Файл preferences.json
//! (settings.json занят библиотекой графов: там последний открытый граф).

use crate::engine::Engine;
use crate::storage::{err, write_atomic, CmdResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    System,
    Dark,
}

/// Языки интерфейса. Новый язык: вариант здесь, словарь src/locales/<код>.json,
/// строка в i18n.rs (Rust) и в LANGUAGES в src/i18n.ts (интерфейс)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Language {
    Ru,
    En,
    Es,
    Zh,
    Ja,
}

impl Language {
    pub const ALL: [Language; 5] = [Language::Ru, Language::En, Language::Es, Language::Zh, Language::Ja];
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub language: Language,
    /// Выключать кабели запущенного графа, когда он останавливается и когда программа закрывается;
    /// при запуске графа его кабели включаются
    pub disable_idle_cables: bool,
    /// Убрать карту графа из правого нижнего угла холста
    pub hide_minimap: bool,
    /// Галочка громкости у нового графа: показывать ли на его нодах регуляторы
    pub new_graph_volume: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            language: system_language(),
            disable_idle_cables: false,
            hide_minimap: false,
            new_graph_volume: true,
        }
    }
}

/// Язык интерфейса Windows, если он есть среди языков приложения; иначе английский
fn system_language() -> Language {
    #[cfg(windows)]
    {
        // младшие 10 бит LANGID — основной язык
        let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        match id & 0x3ff {
            0x19 => return Language::Ru,
            0x0a => return Language::Es,
            0x04 => return Language::Zh,
            0x11 => return Language::Ja,
            _ => {}
        }
    }
    Language::En
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("preferences.json");
        let current: Settings = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        crate::i18n::set_language(current.language);
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[tauri::command]
pub fn settings_get(store: State<SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
pub fn settings_set(store: State<SettingsStore>, engine: State<Engine>, settings: Settings) -> CmdResult<()> {
    let mut current = store.current.lock().map_err(err)?;
    let json = serde_json::to_vec_pretty(&settings).map_err(err)?;
    write_atomic(&store.path, &json).map_err(err)?;
    crate::i18n::set_language(settings.language);
    // движок пересоберёт сообщения на новом языке и обновит трей
    engine.settings_changed(&settings);
    *current = settings;
    Ok(())
}
