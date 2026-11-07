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
mod system_volume;
mod tray;
mod window_state;

use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use window_state::WindowKeeper;

/// How long the window may take to write what it hasn't saved yet when the app quits
const QUIT_WAIT: Duration = Duration::from_secs(2);

/// The OS the app runs on: it decides how cables work in the interface
#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

/// Quit the app: graphs stop, devices are put back
#[tauri::command]
fn app_quit(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        app.state::<WindowKeeper>().save(&window);
    }
    app.exit(0);
}

/// Quits once the window has written what it hasn't saved yet: it answers with app_quit, and
/// one that doesn't answer doesn't hold the app
pub(crate) fn request_quit(app: &AppHandle) {
    let _ = app.emit("quit-requested", ());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(QUIT_WAIT);
        app_quit(app);
    });
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
            app.manage(WindowKeeper::new(&data_dir));
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
            let minimized = autostart::launched() && !tray::shown(app.handle());
            if let Some(window) = app.get_webview_window("main") {
                app.state::<WindowKeeper>().restore(&window, minimized);
            }
            // started with the system the app waits in the tray, or minimized without one
            match (autostart::launched(), tray::shown(app.handle())) {
                (true, true) => {}
                (true, false) => tray::show_minimized(app.handle()),
                (false, _) => tray::show_window(app.handle()),
            }
            Ok(())
        })
        // with the tray the close button and Alt+F4 hide the window there: graphs keep running, quit from the tray menu;
        // without it the window goes at once and the app quits after it
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Some(window) = app.get_webview_window(window.label()) {
                        app.state::<WindowKeeper>().save(&window);
                    }
                    let _ = window.hide();
                    if !tray::shown(app) {
                        request_quit(app);
                    }
                }
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    // the first events come before the app is set up
                    if let (Some(keeper), Some(window)) = (app.try_state::<WindowKeeper>(), app.get_webview_window(window.label())) {
                        keeper.changed(&window);
                    }
                }
                _ => {}
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
            engine::engine_gate,
            engine::engine_levels,
            engine::engine_quiet,
            system_volume::system_volumes,
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
