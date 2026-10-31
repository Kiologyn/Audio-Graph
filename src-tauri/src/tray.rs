//! Значок в трее: запуск и остановка графа без окна.

use crate::engine::Engine;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "main";

pub struct Tray {
    /// «Запустить «граф»» или «Остановить «граф»» для открытого графа — следует за движком
    toggle: MenuItem<Wry>,
    stop_all: MenuItem<Wry>,
    show: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl Tray {
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        let toggle = MenuItem::with_id(app, "toggle", tr!("Запустить"), false, None::<&str>)?;
        let stop_all = MenuItem::with_id(app, "stop_all", tr!("Остановить все"), false, None::<&str>)?;
        let show = MenuItem::with_id(app, "show", tr!("Открыть Audio Graph"), true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", tr!("Выйти"), true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&toggle, &stop_all, &PredefinedMenuItem::separator(app)?, &show, &quit])?;
        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            .tooltip("Audio Graph")
            .menu(&menu)
            // левый клик открывает окно, меню — по правому
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

    /// running — имена запущенных графов, opened — открытого в окне, opened_running — запущен ли он
    pub fn update(&self, app: &AppHandle, running: &[String], opened: Option<&str>, opened_running: bool) {
        let (text, enabled) = match opened {
            Some(name) if opened_running => (tr!("Остановить «{name}»", name = name), true),
            Some(name) => (tr!("Запустить «{name}»", name = name), true),
            None => (tr!("Запустить"), false),
        };
        let tooltip = match running {
            [] => "Audio Graph".to_string(),
            [name] => tr!("Audio Graph — работает «{name}»", name = name),
            many => {
                let names = many.iter().map(|n| tr!("«{name}»", name = n)).collect::<Vec<_>>().join(", ");
                tr!("Audio Graph — работают {names}", names = names)
            }
        };
        let _ = self.toggle.set_text(text);
        let _ = self.toggle.set_enabled(enabled);
        let _ = self.stop_all.set_enabled(!running.is_empty());
        // язык мог смениться
        let _ = self.stop_all.set_text(tr!("Остановить все"));
        let _ = self.show.set_text(tr!("Открыть Audio Graph"));
        let _ = self.quit.set_text(tr!("Выйти"));
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
