//! Tray icon: starting and stopping graphs without the window.

use crate::engine::Engine;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "main";

/// The tray icon while it is on in the settings
#[derive(Default)]
pub struct TrayState(Mutex<Option<Tray>>);

/// Menu items are handles: a copy taken out of the lock updates the same menu
#[derive(Clone)]
pub struct Tray {
    /// "Start "graph"" or "Stop "graph"" for the open graph: follows the engine
    toggle: MenuItem<Wry>,
    stop_all: MenuItem<Wry>,
    show: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl Tray {
    fn new(app: &AppHandle) -> tauri::Result<Self> {
        let toggle = MenuItem::with_id(app, "toggle", tr!("start"), false, None::<&str>)?;
        let stop_all = MenuItem::with_id(app, "stop_all", tr!("tray_stop_all"), false, None::<&str>)?;
        let show = MenuItem::with_id(app, "show", tr!("tray_open"), true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", tr!("tray_quit"), true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&toggle, &stop_all, &PredefinedMenuItem::separator(app)?, &show, &quit])?;
        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            .tooltip("Audio Graph")
            .menu(&menu)
            // a left click opens the window, the menu is on the right click
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "toggle" => app.state::<Engine>().toggle(),
                "stop_all" => app.state::<Engine>().stop_all(),
                "show" => show_window(app),
                "quit" => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_window(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)?;
        Ok(Self { toggle, stop_all, show, quit })
    }

    /// running: names of running graphs, opened: the one open in the window, opened_running: whether it runs
    fn update(&self, app: &AppHandle, running: &[String], opened: Option<&str>, opened_running: bool) {
        let (text, enabled) = match opened.map(literal) {
            Some(name) if opened_running => (tr!("tray_stop_graph", name = name), true),
            Some(name) => (tr!("tray_start_graph", name = name), true),
            None => (tr!("start"), false),
        };
        let tooltip = match running {
            [] => "Audio Graph".to_string(),
            [name] => tr!("tray_tooltip_running_one", name = name),
            many => {
                let names = many.iter().map(|n| tr!("quoted_name", name = n)).collect::<Vec<_>>().join(", ");
                tr!("tray_tooltip_running_many", names = names)
            }
        };
        let _ = self.toggle.set_text(text);
        let _ = self.toggle.set_enabled(enabled);
        let _ = self.stop_all.set_enabled(!running.is_empty());
        // the language may have changed
        let _ = self.stop_all.set_text(tr!("tray_stop_all"));
        let _ = self.show.set_text(tr!("tray_open"));
        let _ = self.quit.set_text(tr!("tray_quit"));
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
}

/// Shows or removes the tray icon. The app works without it: closing the window then quits
pub fn set(app: &AppHandle, on: bool) {
    let Some(state) = app.try_state::<TrayState>() else { return };
    let mut tray = state.0.lock().unwrap_or_else(|e| e.into_inner());
    match (on, tray.is_some()) {
        (true, false) => match Tray::new(app) {
            Ok(t) => *tray = Some(t),
            Err(e) => crate::log::write(format!("tray unavailable: {e}")),
        },
        (false, true) => {
            *tray = None;
            let _ = app.remove_tray_by_id(TRAY_ID);
        }
        _ => {}
    }
}

/// Whether the tray icon is there: the closed window then waits in it
pub fn shown(app: &AppHandle) -> bool {
    app.try_state::<TrayState>()
        .is_some_and(|s| s.0.lock().unwrap_or_else(|e| e.into_inner()).is_some())
}

/// Shows the engine's state in the tray menu; false if there is no tray to show it in
pub fn update(app: &AppHandle, running: &[String], opened: Option<&str>, opened_running: bool) -> bool {
    // the lock isn't held while the menu changes: menu calls wait for the main thread,
    // which may itself be waiting for the lock to turn the tray on or off
    let tray = app.try_state::<TrayState>().and_then(|s| s.0.lock().unwrap_or_else(|e| e.into_inner()).clone());
    match tray {
        Some(tray) => {
            tray.update(app, running, opened, opened_running);
            true
        }
        None => false,
    }
}

/// A name in a menu item's text as it is: & there marks the letter of a keyboard shortcut (Windows, Linux)
fn literal(name: &str) -> String {
    name.replace('&', "&&")
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The window on the taskbar without taking the screen: a start with the system without a tray
pub fn show_minimized(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
        let _ = window.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_names_show_their_ampersands() {
        assert_eq!(literal("Rock & Roll"), "Rock && Roll");
        assert_eq!(literal("Music"), "Music");
    }
}
