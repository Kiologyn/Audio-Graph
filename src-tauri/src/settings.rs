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
    /// Start with the system. The entry in the system is the truth: it is read on every request
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            language: system_language(),
            disable_idle_cables: false,
            hide_minimap: false,
            new_graph_volume: true,
            autostart: false,
        }
    }
}

/// The system's interface language if the app has it; English otherwise
fn system_language() -> Language {
    #[cfg(windows)]
    let language = language_of_langid(unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() });
    #[cfg(not(windows))]
    let language = {
        // the message language as gettext picks it
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|l| !l.is_empty()))
            .unwrap_or_default();
        language_of_locale(&locale)
    };
    language.unwrap_or(Language::En)
}

/// The language of a Windows LANGID: its low 10 bits are the primary language
#[cfg(windows)]
fn language_of_langid(id: u16) -> Option<Language> {
    match id & 0x3ff {
        0x19 => Some(Language::Ru),
        0x0a => Some(Language::Es),
        0x04 => Some(Language::Zh),
        0x11 => Some(Language::Ja),
        _ => None,
    }
}

/// The language of a POSIX locale: ru_RU.UTF-8 → ru
#[cfg(not(windows))]
fn language_of_locale(locale: &str) -> Option<Language> {
    match locale.get(..2) {
        Some("ru") => Some(Language::Ru),
        Some("es") => Some(Language::Es),
        Some("zh") => Some(Language::Zh),
        Some("ja") => Some(Language::Ja),
        _ => None,
    }
}

/// Settings from the file. A value this version doesn't know (a newer version's language, say)
/// resets only that setting, not all of them
fn parse(text: &str) -> Settings {
    use serde_json::Value;
    let (Ok(Value::Object(file)), Ok(Value::Object(mut fields))) = (serde_json::from_str(text), serde_json::to_value(Settings::default()))
    else {
        return Settings::default();
    };
    for (key, value) in file {
        let previous = fields.insert(key.clone(), value);
        if serde_json::from_value::<Settings>(Value::Object(fields.clone())).is_err() {
            match previous {
                Some(v) => fields.insert(key, v),
                None => fields.remove(&key),
            };
        }
    }
    serde_json::from_value(Value::Object(fields)).unwrap_or_default()
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("preferences.json");
        let current = fs::read_to_string(&path).map_or_else(|_| Settings::default(), |t| parse(&t));
        crate::i18n::set_language(current.language);
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        let mut settings = self.current.lock().unwrap_or_else(|e| e.into_inner()).clone();
        settings.autostart = crate::autostart::enabled();
        settings
    }
}

#[tauri::command]
pub fn settings_get(store: State<SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
pub fn settings_set(store: State<SettingsStore>, engine: State<Engine>, settings: Settings) -> CmdResult<()> {
    let mut current = store.current.lock().map_err(err)?;
    if settings.autostart != crate::autostart::enabled() {
        crate::autostart::set(settings.autostart)?;
    }
    let json = serde_json::to_vec_pretty(&settings).map_err(err)?;
    write_atomic(&store.path, &json).map_err(err)?;
    crate::i18n::set_language(settings.language);
    // the engine rebuilds its messages in the new language and updates the tray
    engine.settings_changed(&settings);
    *current = settings;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_value_resets_only_its_setting() {
        let s = parse(r#"{"theme":"dark","language":"de","disableIdleCables":true,"futureSetting":1}"#);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.language, Settings::default().language);
        assert!(s.disable_idle_cables);
        // missing ones are the defaults
        assert!(s.new_graph_volume && !s.hide_minimap);
    }

    #[test]
    fn damaged_file_gives_the_defaults() {
        for text in ["", "{", "[]", "null"] {
            let s = parse(text);
            assert_eq!((s.theme, s.disable_idle_cables), (Theme::System, false), "{text}");
        }
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let s = Settings {
            theme: Theme::Light,
            language: Language::Ja,
            hide_minimap: true,
            ..Settings::default()
        };
        let back = parse(&serde_json::to_string(&s).unwrap());
        assert_eq!((back.theme, back.language, back.hide_minimap), (Theme::Light, Language::Ja, true));
    }

    #[cfg(windows)]
    #[test]
    fn languages_of_windows() {
        // ru-RU, es-MX, zh-TW, ja-JP, en-US, de-DE
        let found: Vec<_> = [0x0419, 0x080a, 0x0404, 0x0411, 0x0409, 0x0407].map(language_of_langid).into();
        let expected = [Some(Language::Ru), Some(Language::Es), Some(Language::Zh), Some(Language::Ja), None, None];
        assert_eq!(found, expected);
    }

    #[cfg(not(windows))]
    #[test]
    fn languages_of_locales() {
        assert_eq!(language_of_locale("ru_RU.UTF-8"), Some(Language::Ru));
        assert_eq!(language_of_locale("zh_CN"), Some(Language::Zh));
        assert_eq!(language_of_locale("C.UTF-8"), None);
        assert_eq!(language_of_locale("POSIX"), None);
        assert_eq!(language_of_locale(""), None);
    }
}
