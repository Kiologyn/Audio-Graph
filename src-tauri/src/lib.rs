// tr! из i18n нужен модулям ниже, поэтому он объявлен первым
#[macro_use]
mod i18n;
mod apps;
mod cables;
#[cfg(windows)]
mod com;
mod devices;
mod engine;
mod graphs;
mod log;
mod settings;
mod storage;
mod tray;

use tauri::{Manager, RunEvent, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            log::init(&data_dir);
            // настройки первыми: от них язык сообщений движка и трея
            let settings = settings::SettingsStore::new(&data_dir);
            app.manage(graphs::GraphStore::new(data_dir.clone())?);
            let cables = cables::CableStore::new(&data_dir);
            let engine = engine::Engine::new(
                app.handle().clone(),
                data_dir.clone(),
                cables.all().unwrap_or_default(),
                &settings.get(),
            );
            app.manage(settings);
            app.manage(cables);
            app.manage(engine);
            let tray = tray::Tray::new(app.handle())?;
            app.manage(tray);
            Ok(())
        })
        // крестик и Alt+F4 прячут окно в трей: граф продолжает работать, выход — из меню трея
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            graphs::graphs_init,
            graphs::graph_load,
            graphs::graph_create,
            graphs::graph_save,
            graphs::graph_rename,
            graphs::graph_set_show_volume,
            graphs::graph_delete,
            cables::cables_list,
            cables::cable_create,
            cables::cable_update,
            cables::cable_delete,
            devices::audio_devices,
            devices::set_devices_enabled,
            apps::audio_apps,
            engine::engine_status,
            engine::engine_start,
            engine::engine_stop,
            engine::engine_volume,
            settings::settings_get,
            settings::settings_set,
        ])
        .build(tauri::generate_context!())
        .expect("ошибка при запуске Audio Graph")
        .run(|app, event| {
            // при закрытии вернуть приложениям их устройства и закрыть потоки звука
            if let RunEvent::Exit = event {
                app.state::<engine::Engine>().shutdown();
            }
        });
}
