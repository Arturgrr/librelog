//! The ink overlay (`/app/annotate-ink`) and the capsule
//! (`/app/annotate-capsule`) as Tauri windows over the recorded monitor:
//! frameless, on top and out of the taskbar, placed through XWayland. They
//! report the pointer and the capsule's buttons through the plugin's
//! `inkPointer`, `inkEscape` and `capsulePressed` commands, and show the
//! `annotateSurface` event.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_capture::recorder::Stream;
use tauri_specta::Event;

use crate::events::AnnotateSurface;

pub const INK_LABEL: &str = "annotate-ink";
pub const CAPSULE_LABEL: &str = "annotate-capsule";
/// The route's capsule, in logical pixels, as on macOS.
const CAPSULE: (f64, f64) = (216.0, 36.0);
const CAPSULE_TOP: f64 = 8.0;
/// A main thread this busy gets the windows closed instead.
const MAIN_WITHIN: Duration = Duration::from_secs(5);

/// A monitor as the windowing layer reports it, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Without the desktop's panels: `(x, y, width, height)`.
    pub work: (i32, i32, u32, u32),
}

impl Monitor {
    /// Its size in points, where strokes are measured.
    pub fn points(&self) -> (f64, f64) {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            f64::from(self.width) / scale,
            f64::from(self.height) / scale,
        )
    }
}

pub struct Opened {
    pub ink: Result<(), String>,
    pub capsule: Result<(), String>,
}

/// The recording's windows, behind a trait for tests.
pub trait Surfaces: Send + Sync {
    fn monitors(&self) -> Vec<Monitor>;
    /// The capsule, and the overlay over `monitor` only if it ignores the
    /// pointer.
    fn open(&self, monitor: &Monitor) -> Opened;
    /// The overlay's top-left corner on its monitor, in points.
    fn ink_origin(&self) -> (f64, f64);
    fn takes_pointer(&self, takes: bool) -> Result<(), String>;
    fn publish(&self, surface: &AnnotateSurface);
    fn close_ink(&self);
    fn close(&self);
}

/// The monitor the portal's stream shows: the only one, else the one at
/// the stream's position (physical or logical) and of its size, else the
/// only one at its position, else the only one of its size.
pub fn pick(stream: Option<&Stream>, monitors: &[Monitor]) -> Option<usize> {
    if monitors.len() == 1 {
        return Some(0);
    }
    let stream = stream?;
    let near = |a: i64, b: i64| (a - b).abs() <= 2;
    let logical = |monitor: &Monitor, value: i64| (value as f64 / monitor.scale).round() as i64;
    let at = |monitor: &Monitor| {
        stream.position.is_some_and(|(x, y)| {
            let (x, y) = (i64::from(x), i64::from(y));
            let (mx, my) = (i64::from(monitor.x), i64::from(monitor.y));
            (near(x, mx) && near(y, my))
                || (near(x, logical(monitor, mx)) && near(y, logical(monitor, my)))
        })
    };
    let sized = |monitor: &Monitor| {
        stream.size.is_some_and(|(w, h)| {
            let (w, h) = (i64::from(w), i64::from(h));
            let (mw, mh) = (i64::from(monitor.width), i64::from(monitor.height));
            (near(w, mw) && near(h, mh))
                || (near(w, logical(monitor, mw)) && near(h, logical(monitor, mh)))
        })
    };
    let only = |test: &dyn Fn(&Monitor) -> bool| {
        let found: Vec<usize> = (0..monitors.len())
            .filter(|&i| test(&monitors[i]))
            .collect();
        (found.len() == 1).then(|| found[0])
    };
    only(&|monitor| at(monitor) && sized(monitor))
        .or_else(|| only(&at))
        .or_else(|| only(&sized))
}

type App = tauri::AppHandle<tauri::Wry>;

pub struct Windows {
    pub app: App,
    origin: Arc<Mutex<(f64, f64)>>,
}

impl Windows {
    pub fn new(app: App) -> Self {
        Self {
            app,
            origin: Arc::default(),
        }
    }
}

/// Runs `work` on the main thread, where windows belong, and waits for it.
/// None when it did not run within `MAIN_WITHIN`; it may still run later.
fn on_main<T: Send + 'static>(
    app: &App,
    work: impl FnOnce(&App) -> T + Send + 'static,
) -> Option<T> {
    let (done, answer) = std::sync::mpsc::sync_channel(1);
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = done.send(work(&handle));
    })
    .ok()?;
    answer.recv_timeout(MAIN_WITHIN).ok()
}

fn monitor(monitor: &tauri::Monitor) -> Monitor {
    let work = monitor.work_area();
    Monitor {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale: monitor.scale_factor(),
        work: (
            work.position.x,
            work.position.y,
            work.size.width,
            work.size.height,
        ),
    }
}

fn lock(origin: &Mutex<(f64, f64)>) -> std::sync::MutexGuard<'_, (f64, f64)> {
    origin.lock().unwrap_or_else(|e| e.into_inner())
}

fn open_ink(app: &App, at: Monitor, origin: Arc<Mutex<(f64, f64)>>) -> Result<(), String> {
    let (width, height) = at.points();
    let window =
        WebviewWindowBuilder::new(app, INK_LABEL, WebviewUrl::App("app/annotate-ink".into()))
            .title("Librelog")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .closable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .focused(false)
            .visible(false)
            .position(f64::from(at.x) / at.scale, f64::from(at.y) / at.scale)
            .inner_size(width, height)
            .disable_drag_drop_handler()
            .build()
            .map_err(|error| error.to_string())?;
    let shown = (|| -> tauri::Result<()> {
        window.set_ignore_cursor_events(true)?;
        window.set_position(PhysicalPosition::new(at.x, at.y))?;
        window.set_size(PhysicalSize::new(at.width, at.height))?;
        window.show()?;
        // Again once mapped: nobody draws on it yet.
        window.set_ignore_cursor_events(true)
    })();
    if let Err(error) = shown {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    let place = move |position: PhysicalPosition<i32>| {
        (
            f64::from(position.x - at.x) / at.scale,
            f64::from(position.y - at.y) / at.scale,
        )
    };
    if let Ok(position) = window.inner_position() {
        *lock(&origin) = place(position);
    }
    // The desktop may keep it off its panels.
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(position) = event {
            *lock(&origin) = place(*position);
        }
    });
    Ok(())
}

fn open_capsule(app: &App, at: Monitor) -> Result<(), String> {
    let (work_x, work_y, work_width, _) = at.work;
    let x = f64::from(work_x) + (f64::from(work_width) - CAPSULE.0 * at.scale) / 2.0;
    let y = f64::from(work_y) + CAPSULE_TOP * at.scale;
    let window = WebviewWindowBuilder::new(
        app,
        CAPSULE_LABEL,
        WebviewUrl::App("app/annotate-capsule".into()),
    )
    .title("Librelog")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .inner_size(CAPSULE.0, CAPSULE.1)
    .disable_drag_drop_handler()
    .build()
    .map_err(|error| error.to_string())?;
    let shown = window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .and_then(|()| window.show());
    if let Err(error) = shown {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    Ok(())
}

fn destroy(app: &App, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.destroy();
    }
}

impl Surfaces for Windows {
    fn monitors(&self) -> Vec<Monitor> {
        self.app
            .available_monitors()
            .map(|monitors| monitors.iter().map(monitor).collect())
            .unwrap_or_default()
    }

    fn open(&self, at: &Monitor) -> Opened {
        let at = *at;
        let origin = self.origin.clone();
        // The capsule second, so it is above the overlay.
        let opened = on_main(&self.app, move |app| {
            destroy(app, INK_LABEL);
            destroy(app, CAPSULE_LABEL);
            Opened {
                ink: open_ink(app, at, origin),
                capsule: open_capsule(app, at),
            }
        });
        opened.unwrap_or_else(|| {
            // Closed after the opening, whenever that runs.
            self.close();
            let late = || Err("the app's windows did not answer in time".to_owned());
            Opened {
                ink: late(),
                capsule: late(),
            }
        })
    }

    fn ink_origin(&self) -> (f64, f64) {
        *lock(&self.origin)
    }

    fn takes_pointer(&self, takes: bool) -> Result<(), String> {
        on_main(&self.app, move |app| {
            app.get_webview_window(INK_LABEL)
                .ok_or_else(|| "the ink overlay is gone".to_owned())?
                .set_ignore_cursor_events(!takes)
                .map_err(|error| error.to_string())
        })
        .unwrap_or_else(|| Err("the app's windows did not answer in time".into()))
    }

    fn publish(&self, surface: &AnnotateSurface) {
        if let Err(error) = surface.clone().emit(&self.app) {
            tracing::warn!(%error, "annotate surface");
        }
    }

    fn close_ink(&self) {
        if on_main(&self.app, |app| destroy(app, INK_LABEL)).is_none() {
            tracing::warn!("annotate: the ink overlay did not close in time");
        }
    }

    fn close(&self) {
        let closed = on_main(&self.app, |app| {
            destroy(app, INK_LABEL);
            destroy(app, CAPSULE_LABEL);
        });
        if closed.is_none() {
            tracing::warn!("annotate: the recording's windows did not close in time");
        }
    }
}
