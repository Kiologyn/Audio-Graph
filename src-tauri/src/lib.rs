// the modules below need tr! from i18n, so it is declared first
#[macro_use]
mod i18n;
mod apps;
mod autostart;
mod calibrate;
mod cables;
#[cfg(windows)]
mod com;
mod devices;
mod engine;
mod graphs;
mod log;
#[cfg(target_os = "linux")]
mod pulse;
mod settings;
mod storage;
mod tray;

use tauri::{Manager, RunEvent, WindowEvent};

/// The OS the app runs on: it decides how cables work in the interface
#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

/// Quit the app as from the tray menu: graphs stop, devices are put back
#[tauri::command]
fn app_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // first: a second instance must exit before it starts anything
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)))
        // file dialogs of graph export and import
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            log::init(&data_dir, &app.package_info().version.to_string());
            // settings first: they set the language of engine and tray messages
            let settings = settings::SettingsStore::new(&data_dir);
            let prefs = settings.get();
            let graphs = graphs::GraphStore::new(data_dir.clone())?;
            let cables = cables::CableStore::new(&data_dir);
            let engine = engine::Engine::new(
                app.handle().clone(),
                data_dir.clone(),
                cables.all().unwrap_or_default(),
                &prefs,
            );
            // the graphs marked for it start with the app while it starts with the system
            if prefs.autostart {
                match graphs.launch_graphs() {
                    Ok(list) => list.iter().for_each(|g| engine.start(g)),
                    Err(e) => log::write(format!("failed to start graphs at launch: {e}")),
                }
                // the entry follows the app's path (another install folder)
                if let Err(e) = autostart::set(true) {
                    log::write(format!("failed to update the autostart entry: {e}"));
                }
            }
            app.manage(graphs);
            app.manage(settings);
            app.manage(cables);
            app.manage(engine);
            app.manage(tray::TrayState::default());
            tray::set(app.handle(), prefs.tray);
            // started with the system the app waits in the tray, or minimized without one
            match (autostart::launched(), tray::shown(app.handle())) {
                (true, true) => {}
                (true, false) => tray::show_minimized(app.handle()),
                (false, _) => tray::show_window(app.handle()),
            }
            Ok(())
        })
        // with the tray the close button and Alt+F4 hide the window there: graphs keep running, quit from the tray menu;
        // without it the window closes and the app quits with it
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if tray::shown(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            graphs::graphs_init,
            graphs::graph_load,
            graphs::graph_create,
            graphs::graph_save,
            graphs::graph_rename,
            graphs::graph_set_prefs,
            graphs::graph_delete,
            graphs::graphs_reorder,
            graphs::graph_export,
            graphs::graph_import,
            cables::cables_list,
            cables::cable_create,
            cables::cable_update,
            cables::cable_delete,
            cables::cables_reorder,
            devices::audio_devices,
            devices::set_devices_enabled,
            apps::audio_apps,
            engine::engine_status,
            engine::engine_start,
            engine::engine_stop,
            engine::engine_volume,
            engine::engine_balance,
            engine::engine_quiet,
            calibrate::calibrate_measure,
            calibrate::calibrate_preview,
            settings::settings_get,
            settings::settings_set,
            autostart::autostart_supported,
            platform,
            app_quit,
        ])
        .build(tauri::generate_context!())
        .expect("error while running Audio Graph")
        .run(|app, event| {
            // on exit give apps back their devices and close the audio streams
            if let RunEvent::Exit = event {
                app.state::<engine::Engine>().shutdown();
            }
        });
}
