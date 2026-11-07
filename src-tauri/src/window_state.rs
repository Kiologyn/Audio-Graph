//! The main window between launches: where it stood, how big it was and whether it was maximized.
//! It is written shortly after every change and not on exit: an app living in the tray usually
//! ends with the computer, without an exit of its own.

use crate::storage::write_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

/// A burst of moves and resizes is written this long after it began
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// On Linux the desktop places windows, only their size and maximized state come back: Wayland doesn't
/// let an app choose, and on X11 a window is reported with its shadow, so going back there would creep
const KEEPS_PLACE: bool = cfg!(windows);

/// The window when it isn't maximized: its corner and the size of what is inside it, in physical pixels
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Bounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Placement {
    #[serde(flatten)]
    bounds: Bounds,
    maximized: bool,
}

/// The part of a monitor windows go on
#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl From<&Monitor> for Area {
    fn from(monitor: &Monitor) -> Self {
        let area = monitor.work_area();
        Self { x: area.position.x, y: area.position.y, width: area.size.width, height: area.size.height }
    }
}

/// Where a window goes so that all of it can be reached: on the monitor that shows most of it, or
/// on the first one if none does, and no bigger than that monitor. A window that is on a monitor stays as
/// it was. Frame is what the title bar and borders add to the size
fn on_screen(bounds: Bounds, frame: (u32, u32), areas: &[Area]) -> Bounds {
    let (width, height) = (bounds.width + frame.0, bounds.height + frame.1);
    let shown = |area: &Area| {
        let across = (bounds.x + width as i32).min(area.x + area.width as i32) - bounds.x.max(area.x);
        let down = (bounds.y + height as i32).min(area.y + area.height as i32) - bounds.y.max(area.y);
        across.max(0) as i64 * down.max(0) as i64
    };
    let most = areas.iter().max_by_key(|area| shown(area)).filter(|area| shown(area) > 0);
    let Some(area) = most.or(areas.first()) else { return bounds };
    let (width, height) = (width.min(area.width), height.min(area.height));
    Bounds {
        x: bounds.x.clamp(area.x, area.x + (area.width - width) as i32),
        y: bounds.y.clamp(area.y, area.y + (area.height - height) as i32),
        width: width - frame.0.min(width),
        height: height - frame.1.min(height),
    }
}

/// What the file says, if it makes sense
fn load(path: &Path) -> Option<Placement> {
    let placement: Placement = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (placement.bounds.width > 0 && placement.bounds.height > 0).then_some(placement)
}

/// The window as it stands, if it stands anywhere: a minimized one has no place of its own
fn bounds_of(window: &WebviewWindow) -> Option<Bounds> {
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    (size.width > 0 && size.height > 0).then_some(Bounds { x: position.x, y: position.y, width: size.width, height: size.height })
}

pub struct WindowKeeper {
    path: PathBuf,
    placement: Mutex<Option<Placement>>,
    /// A save is on its way
    pending: AtomicBool,
    /// The app started minimized: a maximized window can't be shown so, it is maximized when it comes up
    maximize_when_shown: AtomicBool,
}

impl WindowKeeper {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("window.json");
        Self {
            placement: Mutex::new(load(&path)),
            path,
            pending: AtomicBool::new(false),
            maximize_when_shown: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Placement>> {
        self.placement.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Puts the window where it was. Call while it is hidden: it then comes up in its place at once
    pub fn restore(&self, window: &WebviewWindow, minimized: bool) {
        let saved = *self.lock();
        let Some(saved) = saved else {
            // nothing to go back to, but the window may be maximized before anything is saved
            let bounds = bounds_of(window);
            *self.lock() = bounds.map(|bounds| Placement { bounds, maximized: false });
            return;
        };
        let primary = window.primary_monitor().ok().flatten();
        let others = window.available_monitors().unwrap_or_default();
        let areas: Vec<Area> = primary.iter().chain(others.iter()).map(Area::from).collect();
        let (outer, inner) = (window.outer_size(), window.inner_size());
        let frame = match (outer, inner) {
            (Ok(outer), Ok(inner)) => (outer.width.saturating_sub(inner.width), outer.height.saturating_sub(inner.height)),
            _ => (0, 0),
        };
        let wanted = match bounds_of(window) {
            Some(Bounds { x, y, .. }) if !KEEPS_PLACE => Bounds { x, y, ..saved.bounds },
            _ => saved.bounds,
        };
        let bounds = on_screen(wanted, frame, &areas);
        // the place first: the size is then counted for that monitor's scale
        if KEEPS_PLACE {
            let _ = window.set_position(PhysicalPosition::new(bounds.x, bounds.y));
        }
        let _ = window.set_size(PhysicalSize::new(bounds.width, bounds.height));
        if saved.maximized {
            if minimized {
                self.maximize_when_shown.store(true, Ordering::Relaxed);
            } else {
                let _ = window.maximize();
            }
        }
        *self.lock() = Some(Placement { bounds, ..saved });
    }

    /// The window moved or changed size
    pub fn changed(&self, window: &WebviewWindow) {
        let up = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(true);
        if up && self.maximize_when_shown.swap(false, Ordering::Relaxed) {
            let _ = window.maximize();
        }
        if self.pending.swap(true, Ordering::Relaxed) {
            return;
        }
        let app: AppHandle = window.app_handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            let keeper = app.state::<WindowKeeper>();
            keeper.pending.store(false, Ordering::Relaxed);
            if let Some(window) = app.get_webview_window("main") {
                keeper.save(&window);
            }
        });
    }

    /// Writes where the window is now. Its place while maximized is the one it returns to, on the monitor it is
    /// maximized on. Asked after a burst of events: during one, what the system reports isn't settled yet
    pub fn save(&self, window: &WebviewWindow) {
        if window.is_minimized().unwrap_or(true) {
            return;
        }
        // the window answers from the main thread, and that thread may be waiting for the lock here: ask first
        let maximized = window.is_maximized().unwrap_or(false);
        let monitor = window.current_monitor().ok().flatten().filter(|_| maximized);
        let standing = bounds_of(window).filter(|_| !maximized);
        let areas: Vec<Area> = monitor.iter().map(Area::from).collect();
        let mut placement = self.lock();
        let Some(bounds) = standing.or(placement.map(|p| on_screen(p.bounds, (0, 0), &areas))) else { return };
        let now = Placement { bounds, maximized };
        if *placement == Some(now) {
            return;
        }
        *placement = Some(now);
        let written = serde_json::to_vec_pretty(&now).map_err(|e| e.to_string()).and_then(|data| write_atomic(&self.path, &data).map_err(|e| e.to_string()));
        if let Err(e) = written {
            crate::log::write(format!("failed to save the window's place: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: (u32, u32) = (16, 39);
    const MAIN: Area = Area { x: 0, y: 0, width: 1920, height: 1040 };
    const LEFT: Area = Area { x: -1280, y: 100, width: 1280, height: 984 };

    fn bounds(x: i32, y: i32, width: u32, height: u32) -> Bounds {
        Bounds { x, y, width, height }
    }

    #[test]
    fn a_window_on_a_monitor_stays_as_it_was() {
        assert_eq!(on_screen(bounds(100, 50, 1280, 800), FRAME, &[MAIN, LEFT]), bounds(100, 50, 1280, 800));
        assert_eq!(on_screen(bounds(-1200, 150, 900, 600), FRAME, &[MAIN, LEFT]), bounds(-1200, 150, 900, 600));
    }

    #[test]
    fn a_window_of_a_monitor_that_is_gone_comes_to_the_primary_one() {
        assert_eq!(on_screen(bounds(-1200, 150, 900, 600), FRAME, &[MAIN]), bounds(0, 150, 900, 600));
        assert_eq!(on_screen(bounds(5000, 3000, 900, 600), FRAME, &[MAIN, LEFT]), bounds(1004, 401, 900, 600));
    }

    #[test]
    fn a_window_hanging_off_a_monitor_comes_back_whole() {
        assert_eq!(on_screen(bounds(1500, 900, 800, 500), FRAME, &[MAIN]), bounds(1104, 501, 800, 500));
    }

    #[test]
    fn a_window_too_big_for_the_monitor_fits_it() {
        // saved on a big monitor, opened on a laptop
        assert_eq!(on_screen(bounds(200, 100, 3000, 1800), FRAME, &[MAIN]), bounds(0, 0, 1904, 1001));
    }

    #[test]
    fn a_window_across_two_monitors_goes_where_more_of_it_is() {
        assert_eq!(on_screen(bounds(-300, 200, 1000, 600), FRAME, &[MAIN, LEFT]).x, 0);
        assert_eq!(on_screen(bounds(-800, 200, 1000, 600), FRAME, &[MAIN, LEFT]).x, -1016);
    }

    #[test]
    fn without_monitors_the_window_stays() {
        assert_eq!(on_screen(bounds(1, 2, 3, 4), FRAME, &[]), bounds(1, 2, 3, 4));
    }

    #[test]
    fn a_broken_file_is_no_placement() {
        let dir = std::env::temp_dir().join(format!("ag-window-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("window.json");
        let read = |text: &str| {
            fs::write(&path, text).unwrap();
            load(&path)
        };
        assert_eq!(
            read(r#"{"x":-8,"y":10,"width":1280,"height":800,"maximized":true}"#),
            Some(Placement { bounds: bounds(-8, 10, 1280, 800), maximized: true })
        );
        assert_eq!(read("{not json"), None);
        assert_eq!(read(r#"{"x":0,"y":0,"width":0,"height":800,"maximized":false}"#), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
