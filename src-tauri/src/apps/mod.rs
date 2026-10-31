//! Список запущенных приложений для выбора в ноде приложения. Перечисление зависит
//! от платформы, снаружи модуль одинаковый.

use serde::Serialize;

#[cfg(windows)]
pub(crate) mod win;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioApp {
    /// Имя исполняемого файла: по нему приложение узнаётся между запусками,
    /// даже если после обновления сменилась папка
    pub exe: String,
    /// Полный путь к исполняемому файлу
    pub path: String,
    /// Название программы из свойств файла, иначе имя файла
    pub name: String,
    /// Есть звуковой сеанс: приложение видно в микшере громкости
    pub has_audio: bool,
    /// Сейчас воспроизводит или записывает звук
    pub playing: bool,
}

pub fn list() -> Result<Vec<AudioApp>, String> {
    #[cfg(windows)]
    return win::list().map_err(|e| e.to_string());
    #[cfg(not(windows))]
    Ok(Vec::new())
}

/// Перечисление идёт через COM и обход окон, поэтому выполняется вне потока окна
#[tauri::command]
pub async fn audio_apps() -> Result<Vec<AudioApp>, String> {
    tauri::async_runtime::spawn_blocking(list)
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Печатает приложения этой машины и время перечисления:
    /// cargo test lists_apps -- --nocapture
    #[test]
    fn lists_apps() {
        let started = std::time::Instant::now();
        let apps = list().expect("перечисление приложений");
        println!("{} приложений за {:?}", apps.len(), started.elapsed());
        for a in &apps {
            let audio = if a.playing { "играет" } else if a.has_audio { "звук" } else { "-" };
            println!("{audio:7} {} ({})  [{}]", a.name, a.exe, a.path);
        }
    }
}
