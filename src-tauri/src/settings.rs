//! App settings: theme, language, cable behavior. The file is preferences.json
//! (settings.json belongs to the graph library: it holds the last open graph).

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

/// Interface languages. A new language: a variant here, a dictionary src/locales/<code>.json,
/// a row in i18n.rs (Rust) and in LANGUAGES in src/i18n.ts (interface)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Language {
    En,
    Ru,
    Es,
    Zh,
    Ja,
}

impl Language {
    pub const ALL: [Language; 5] = [Language::En, Language::Ru, Language::Es, Language::Zh, Language::Ja];
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: Theme,
    pub language: Language,
    /// Turn a running graph's cables off when it stops and when the app closes;
    /// a graph's cables are turned on when it starts
    pub disable_idle_cables: bool,
    /// Hide the graph minimap in the bottom right corner of the canvas
    pub hide_minimap: bool,
    /// The volume checkbox of a new graph: whether its nodes show volume sliders
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

/// The system's interface language if the app has it; English otherwise
fn system_language() -> Language {
    #[cfg(windows)]
    {
        // the low 10 bits of a LANGID are the primary language
        let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
        match id & 0x3ff {
            0x19 => return Language::Ru,
            0x0a => return Language::Es,
            0x04 => return Language::Zh,
            0x11 => return Language::Ja,
            _ => {}
        }
    }
    #[cfg(not(windows))]
    {
        // the message language as gettext picks it: ru_RU.UTF-8 → ru
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty()))
            .unwrap_or_default();
        match locale.get(..2) {
            Some("ru") => return Language::Ru,
            Some("es") => return Language::Es,
            Some("zh") => return Language::Zh,
            Some("ja") => return Language::Ja,
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
    // the engine rebuilds its messages in the new language and updates the tray
    engine.settings_changed(&settings);
    *current = settings;
    Ok(())
}
