//! Tray icon: starting and stopping graphs without the window. Its menu has the first graphs of the list, those
//! that run marked, and a click on one starts or stops it. On Windows the menu is a small window of the app's own,
//! with the dots of the graph list: green by a running graph, red by one running with problems. It stays open for
//! more clicks and goes when something else is clicked. On Linux it is the system's menu with checks, which closes
//! on a click and gives no clicks on the icon itself.

use crate::engine::Engine;
use crate::graphs::{GraphMeta, GraphStore};
use crate::storage::CmdResult;
use serde::Serialize;
use std::sync::Mutex;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
#[cfg(windows)]
use tauri::Emitter;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

const TRAY_ID: &str = "main";
/// How many graphs of the list the menu has
const GRAPHS: usize = 5;
/// The label of the menu's window (Windows)
pub const MENU: &str = "tray-menu";

/// Whether the tray icon is on, and where its menu was last asked for
#[derive(Default)]
pub struct TrayState {
    shown: Mutex<bool>,
    asked_at: Mutex<Option<PhysicalPosition<f64>>>,
}

/// A graph in the menu
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayGraph {
    pub id: String,
    pub name: String,
    pub running: bool,
    /// Running, and something in it doesn't work
    pub troubled: bool,
}

/// What the menu shows
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayMenu {
    pub graphs: Vec<TrayGraph>,
    /// Any graph at all, so "Stop all" has something to stop
    pub any_running: bool,
}

/// The first graphs of the list, each with whether it runs and whether it runs with problems
fn offered(list: Vec<GraphMeta>, running: &[String], troubled: &[String]) -> Vec<TrayGraph> {
    list.into_iter()
        .take(GRAPHS)
        .map(|g| TrayGraph {
            running: running.contains(&g.id),
            troubled: troubled.contains(&g.id),
            id: g.id,
            name: g.name,
        })
        .collect()
}

fn menu(app: &AppHandle) -> TrayMenu {
    let status = app.state::<Engine>().status();
    let list = app.state::<GraphStore>().graphs().unwrap_or_default();
    TrayMenu { graphs: offered(list, &status.running, &status.troubled), any_running: !status.running.is_empty() }
}

/// Starts a graph that is stopped and stops one that runs
fn toggle(app: &AppHandle, id: &str) -> CmdResult<()> {
    let engine = app.state::<Engine>();
    if engine.status().running.iter().any(|r| r == id) {
        engine.stop(id);
    } else {
        engine.start(&app.state::<GraphStore>().load(id)?);
    }
    Ok(())
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID).tooltip("Audio Graph").on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = event {
            match button {
                MouseButton::Left => show_window(tray.app_handle()),
                MouseButton::Right => open_menu(tray.app_handle(), position),
                MouseButton::Middle => {}
            }
        }
    });
    #[cfg(not(windows))]
    {
        builder = builder
            .menu(&native::menu(app)?)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| native::clicked(app, event.id.as_ref()));
    }
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    #[cfg(windows)]
    {
        // the menu is ready before it is first asked for. A window made in a command's thread would hang
        let handle = app.clone();
        app.run_on_main_thread(move || {
            if let Err(e) = menu_window(&handle) {
                crate::log::write(format!("tray menu unavailable: {e}"));
            }
        })?;
    }
    Ok(())
}

/// The window the menu is in: no frame, over everything, not on the taskbar, hidden until asked for
#[cfg(windows)]
fn menu_window(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(MENU).is_some() {
        return Ok(());
    }
    let mut builder = tauri::WebviewWindowBuilder::new(app, MENU, tauri::WebviewUrl::App("index.html".into()))
        .title("Audio Graph")
        .visible(false)
        .focused(false)
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(true)
        .inner_size(240.0, 160.0);
    // the app's webviews share one browser, which takes them only with the same arguments
    let config = app.config().app.windows.iter().find(|w| w.label == "main");
    if let Some(args) = config.and_then(|w| w.additional_browser_args.as_deref()) {
        builder = builder.additional_browser_args(args);
    }
    // the app's window owns its menu: tools that place new windows on the screen leave owned ones be
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.owner(&main)?;
    }
    builder.build()?;
    Ok(())
}

/// The menu asks for its size and place itself once it knows what it shows (tray_menu_show)
#[cfg(windows)]
fn open_menu(app: &AppHandle, at: PhysicalPosition<f64>) {
    *app.state::<TrayState>().asked_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
    if app.get_webview_window(MENU).is_none() {
        if let Err(e) = menu_window(app) {
            crate::log::write(format!("tray menu unavailable: {e}"));
        }
    }
    let _ = app.emit_to(MENU, "tray-menu-open", ());
}

/// The system shows the menu on Linux
#[cfg(not(windows))]
fn open_menu(_app: &AppHandle, _at: PhysicalPosition<f64>) {}

/// Where a menu of this size goes for a click at this point, as the system puts one: right of it and below
/// if there is room, otherwise left of it and above, and always within the work area (left, top, right, bottom)
fn menu_spot(at: (i32, i32), size: (i32, i32), area: (i32, i32, i32, i32)) -> (i32, i32) {
    let ((x, y), (width, height), (left, top, right, bottom)) = (at, size, area);
    let x = if x + width > right { x - width } else { x };
    let y = if y + height > bottom { y - height } else { y };
    (x.min(right - width).max(left), y.min(bottom - height).max(top))
}

/// Shows or removes the tray icon. The app works without it: closing the window then quits
pub fn set(app: &AppHandle, on: bool) {
    let Some(state) = app.try_state::<TrayState>() else { return };
    let mut shown = state.shown.lock().unwrap_or_else(|e| e.into_inner());
    match (on, *shown) {
        (true, false) => match create(app) {
            Ok(()) => *shown = true,
            Err(e) => crate::log::write(format!("tray unavailable: {e}")),
        },
        (false, true) => {
            *shown = false;
            let _ = app.remove_tray_by_id(TRAY_ID);
            if let Some(window) = app.get_webview_window(MENU) {
                let _ = window.destroy();
            }
        }
        _ => {}
    }
}

/// Whether the tray icon is there: the closed window then waits in it
pub fn shown(app: &AppHandle) -> bool {
    app.try_state::<TrayState>()
        .is_some_and(|s| *s.shown.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Shows the running graphs in the tray: in the tooltip by name, in the menu by their mark. False if there is no tray
pub fn update(app: &AppHandle, running: &[String]) -> bool {
    if !shown(app) {
        return false;
    }
    let tooltip = match running {
        [] => "Audio Graph".to_string(),
        [name] => tr!("tray_tooltip_running_one", name = name),
        many => {
            let names = many.iter().map(|n| tr!("quoted_name", name = n)).collect::<Vec<_>>().join(", ");
            tr!("tray_tooltip_running_many", names = names)
        }
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(tooltip));
    }
    refresh(app);
    true
}

/// The menu after the graph list changed: names, order, graphs added or deleted. It may be told while the
/// graph list is held: it reads the list on its own thread, once the list is free
pub fn refresh(app: &AppHandle) {
    if !shown(app) {
        return;
    }
    #[cfg(windows)]
    let _ = app.emit_to(MENU, "tray-menu-changed", ());
    #[cfg(not(windows))]
    {
        let app = app.clone();
        std::thread::spawn(move || native::rebuild(&app));
    }
}

/// The menu's graphs and whether anything runs
#[tauri::command]
pub fn tray_menu(app: AppHandle) -> TrayMenu {
    menu(&app)
}

/// A click on a graph in the menu
#[tauri::command]
pub fn tray_toggle(app: AppHandle, id: String) -> CmdResult<()> {
    toggle(&app, &id)
}

/// Shows the menu at the size of what it shows, where the tray icon was clicked, the way a menu opens there
#[tauri::command]
pub fn tray_menu_show(app: AppHandle, width: f64, height: f64) {
    let Some(window) = app.get_webview_window(MENU) else { return };
    let asked = *app.state::<TrayState>().asked_at.lock().unwrap_or_else(|e| e.into_inner());
    let Some(at) = asked.or_else(|| app.cursor_position().ok()) else { return };
    let monitor = app.monitor_from_point(at.x, at.y).ok().flatten().or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let size = ((width * scale).ceil() as i32, (height * scale).ceil() as i32);
    let area = monitor.work_area();
    let (x, y) = (area.position.x, area.position.y);
    let bounds = (x, y, x + area.size.width as i32, y + area.size.height as i32);
    let (left, top) = menu_spot((at.x as i32, at.y as i32), size, bounds);
    let spot = PhysicalPosition::new(left, top);
    // on another monitor the window takes its scale on arrival: the size comes after the move, and the place again
    let _ = window.set_position(spot);
    let _ = window.set_size(PhysicalSize::new(size.0 as u32, size.1 as u32));
    let _ = window.set_position(spot);
    let _ = window.show();
    let _ = window.set_focus();
}

/// Hides the menu, and opens the app's window if asked
#[tauri::command]
pub fn tray_menu_close(app: AppHandle, open: bool) {
    if let Some(window) = app.get_webview_window(MENU) {
        let _ = window.hide();
    }
    if open {
        show_window(&app);
    }
}

/// The system's menu (Linux): built again whenever what it shows changes
#[cfg(not(windows))]
mod native {
    use super::*;
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    use tauri::Wry;

    /// A name in a menu item's text as it is: & there marks the letter of a keyboard shortcut
    pub fn literal(name: &str) -> String {
        name.replace('&', "&&")
    }

    pub fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
        let shown = super::menu(app);
        let menu = Menu::new(app)?;
        for graph in &shown.graphs {
            let id = format!("graph:{}", graph.id);
            menu.append(&CheckMenuItem::with_id(app, id, literal(&graph.name), true, graph.running, None::<&str>)?)?;
        }
        if !shown.graphs.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        menu.append(&MenuItem::with_id(app, "stop_all", tr!("tray_stop_all"), shown.any_running, None::<&str>)?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(app, "show", tr!("tray_open"), true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "quit", tr!("tray_quit"), true, None::<&str>)?)?;
        Ok(menu)
    }

    pub fn rebuild(app: &AppHandle) {
        if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), menu(app)) {
            let _ = tray.set_menu(Some(menu));
        }
    }

    pub fn clicked(app: &AppHandle, id: &str) {
        match id {
            "stop_all" => app.state::<Engine>().stop_all(),
            "show" => show_window(app),
            "quit" => crate::request_quit(app),
            _ => {
                if let Some(graph) = id.strip_prefix("graph:") {
                    if let Err(e) = toggle(app, graph) {
                        crate::log::write(format!("tray: {e}"));
                    }
                }
                // the system checks the item at once; the engine's answer sets it right
                rebuild(app);
            }
        }
    }
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

    fn meta(id: &str) -> GraphMeta {
        GraphMeta {
            id: id.into(),
            name: format!("Graph {id}"),
            created_at: 0,
            updated_at: 0,
            delay_limit: 1.0,
            volume_limit: 2.0,
            distance_limit: 1.0,
            start_on_launch: false,
        }
    }

    #[test]
    fn the_menu_has_the_first_five_graphs_of_the_list_with_the_running_ones_marked() {
        let list: Vec<GraphMeta> = ["a", "b", "c", "d", "e", "f", "g"].into_iter().map(meta).collect();
        let shown = offered(list, &["b".into(), "d".into(), "f".into()], &["d".into(), "f".into()]);
        assert_eq!(shown.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["a", "b", "c", "d", "e"]);
        assert_eq!(shown.iter().map(|g| g.running).collect::<Vec<_>>(), [false, true, false, true, false]);
        assert_eq!(shown.iter().map(|g| g.troubled).collect::<Vec<_>>(), [false, false, false, true, false]);
        assert_eq!(shown[0].name, "Graph a");
    }

    #[test]
    fn a_short_list_is_all_there() {
        assert_eq!(offered(vec![meta("a")], &[], &[]).len(), 1);
        assert!(offered(vec![], &["x".into()], &[]).is_empty());
    }

    // a taskbar at the bottom of a 1920x1080 screen, 40 high
    const AREA: (i32, i32, i32, i32) = (0, 0, 1920, 1040);

    #[test]
    fn opens_above_a_click_on_the_taskbar_and_left_of_it_at_the_screens_edge() {
        assert_eq!(menu_spot((1700, 1060), (240, 200), AREA), (1460, 840));
        assert_eq!(menu_spot((1000, 1060), (240, 200), AREA), (1000, 840));
    }

    #[test]
    fn opens_below_a_taskbar_at_the_top_and_right_of_one_on_the_left() {
        assert_eq!(menu_spot((1700, 20), (240, 200), (0, 40, 1920, 1080)), (1460, 40));
        assert_eq!(menu_spot((20, 900), (240, 200), (48, 0, 1920, 1080)), (48, 700));
    }

    #[test]
    fn stays_inside_on_a_monitor_left_of_the_main_one() {
        let area = (-2560, 0, 0, 1560);
        let (x, y) = menu_spot((-10, 1580), (240, 200), area);
        assert!(x >= -2560 && x + 240 <= 0 && y >= 0 && y + 200 <= 1560, "{x} {y}");
    }

    #[cfg(not(windows))]
    #[test]
    fn graph_names_show_their_ampersands_in_the_systems_menu() {
        assert_eq!(native::literal("Rock & Roll"), "Rock && Roll");
    }
}
