//! The ink overlay and the capsule as Tauri windows over the recorded
//! monitor: frameless, on top, out of the taskbar and never focused, placed
//! through XWayland. Both render outside the app's router and gates
//! (`main.tsx` picks them by label), report `surfaceReady` once mounted,
//! and are shown only then. The overlay's input shape is set through GTK on
//! the main thread and confirmed with the X server; when the main thread
//! doesn't answer, the X server is asked to let the pointer go directly.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_capture::recorder::Stream;
use tauri_specta::Event;

use super::x11::{self, Shape};
use crate::events::AnnotateSurface;

pub const INK_LABEL: &str = "annotate-ink";
pub const CAPSULE_LABEL: &str = "annotate-capsule";
/// The capsule, in logical pixels, as on macOS.
const CAPSULE: (f64, f64) = (216.0, 36.0);
const CAPSULE_TOP: f64 = 8.0;
/// Around the capsule, the overlay never takes the pointer.
const CAPSULE_MARGIN: f64 = 4.0;
/// A main thread this busy gets the windows closed instead.
const MAIN_WITHIN: Duration = Duration::from_secs(5);
/// How long a window's page may take to load before it is given up.
const READY_WITHIN: Duration = Duration::from_secs(10);
const LATE: &str = "the app's windows did not answer in time";

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
    fn scale(&self) -> f64 {
        if self.scale > 0.0 { self.scale } else { 1.0 }
    }

    /// Its size in points, where strokes are measured.
    pub fn points(&self) -> (f64, f64) {
        (
            f64::from(self.width) / self.scale(),
            f64::from(self.height) / self.scale(),
        )
    }

    /// The capsule's top-left corner, physical: centred, under the panels.
    fn capsule(&self) -> (i32, i32) {
        let (x, y, width, _) = self.work;
        (
            (f64::from(x) + (f64::from(width) - CAPSULE.0 * self.scale()) / 2.0).round() as i32,
            (f64::from(y) + CAPSULE_TOP * self.scale()).round() as i32,
        )
    }

    /// The overlay's size, physical: a pixel short of the monitor, so the
    /// desktop never takes it for a full-screen window.
    fn overlay(&self) -> (u32, u32) {
        (self.width, self.height.saturating_sub(1).max(1))
    }

    /// Where the overlay takes the pointer, in its logical pixels: all of
    /// it but the capsule, which stays clickable whatever the stacking.
    fn taking(&self) -> ((i32, i32), (i32, i32, i32, i32)) {
        let scale = self.scale();
        let (width, height) = self.overlay();
        let (x, y) = self.capsule();
        let logical = |value: f64| (value / scale).round() as i32;
        (
            (logical(f64::from(width)), logical(f64::from(height))),
            (
                logical(f64::from(x - self.x)) - CAPSULE_MARGIN as i32,
                logical(f64::from(y - self.y)) - CAPSULE_MARGIN as i32,
                (CAPSULE.0 + 2.0 * CAPSULE_MARGIN) as i32,
                (CAPSULE.1 + 2.0 * CAPSULE_MARGIN) as i32,
            ),
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
    /// The capsule, and the overlay over `monitor` only if the X server
    /// shows that it lets the pointer through.
    fn open(&self, monitor: &Monitor) -> Opened;
    /// A window's page mounted.
    fn loaded(&self, label: &str);
    /// The overlay's top-left corner on its monitor, in points.
    fn ink_origin(&self) -> (f64, f64);
    /// Ok only once the X server shows the change.
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

#[derive(Clone, Copy)]
struct Ink {
    xid: u32,
    monitor: Monitor,
}

pub struct Windows {
    app: App,
    x11: Arc<dyn Shape>,
    origin: Arc<Mutex<(f64, f64)>>,
    ink: Mutex<Option<Ink>>,
    ready: Arc<(Mutex<HashSet<String>>, Condvar)>,
}

impl Windows {
    pub fn new(app: App) -> Self {
        Self {
            app,
            x11: Arc::new(x11::Server),
            origin: Arc::default(),
            ink: Mutex::new(None),
            ready: Arc::default(),
        }
    }

    fn ink(&self) -> std::sync::MutexGuard<'_, Option<Ink>> {
        self.ink.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Waits until `deadline` for `label`'s page.
    fn wait_ready(&self, label: &str, deadline: Instant) -> bool {
        let (ready, changed) = &*self.ready;
        let mut ready = ready.lock().unwrap_or_else(|e| e.into_inner());
        while !ready.contains(label) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            ready = changed
                .wait_timeout(ready, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
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

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The overlay's input shape through GTK, kept across realizes, and sent
/// to the X server before this returns. `taking` None lets everything
/// through: an empty region, not tao's 1 × 1 one.
fn input_shape(
    window: &tauri::WebviewWindow,
    taking: Option<((i32, i32), (i32, i32, i32, i32))>,
) -> Result<(), String> {
    use gtk::cairo::{RectangleInt, Region};
    let gtk_window = window.gtk_window().map_err(text)?;
    let region = match taking {
        None => Region::create(),
        Some(((width, height), (x, y, w, h))) => {
            let region = Region::create_rectangle(&RectangleInt::new(0, 0, width, height));
            region
                .subtract_rectangle(&RectangleInt::new(x, y, w, h))
                .map_err(text)?;
            region
        }
    };
    gtk_window.input_shape_combine_region(Some(&region));
    gtk_window
        .window()
        .ok_or("the overlay has no window yet")?
        .display()
        .sync();
    Ok(())
}

fn xid(window: &tauri::WebviewWindow) -> Result<u32, String> {
    match window.window_handle().map_err(text)?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).map_err(text),
        RawWindowHandle::Xcb(handle) => Ok(handle.window.get()),
        _ => Err("the overlay is not an X11 window".into()),
    }
}

/// Hidden, realized, never focused and letting the pointer through before
/// anything can show it.
fn build_ink(app: &App, at: Monitor) -> Result<u32, String> {
    let (width, height) = at.overlay();
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
            .focusable(false)
            .visible(false)
            .position(f64::from(at.x) / at.scale(), f64::from(at.y) / at.scale())
            .inner_size(
                f64::from(width) / at.scale(),
                f64::from(height) / at.scale(),
            )
            .disable_drag_drop_handler()
            .build()
            .map_err(text)?;
    let built = (|| {
        let gtk_window = window.gtk_window().map_err(text)?;
        gtk_window.set_accept_focus(false);
        gtk_window.set_focus_on_map(false);
        gtk_window.realize();
        input_shape(&window, None)?;
        xid(&window)
    })();
    if built.is_err() {
        let _ = window.destroy();
    }
    built
}

fn show_ink(app: &App, at: Monitor, origin: Arc<Mutex<(f64, f64)>>) -> Result<(), String> {
    let window = app
        .get_webview_window(INK_LABEL)
        .ok_or("the ink overlay is gone")?;
    let (width, height) = at.overlay();
    let shown = (|| {
        window
            .set_position(PhysicalPosition::new(at.x, at.y))
            .map_err(text)?;
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(text)?;
        window.show().map_err(text)?;
        input_shape(&window, None)
    })();
    if let Err(error) = shown {
        let _ = window.destroy();
        return Err(error);
    }
    let place = move |position: PhysicalPosition<i32>| {
        (
            f64::from(position.x - at.x) / at.scale(),
            f64::from(position.y - at.y) / at.scale(),
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

fn build_capsule(app: &App) -> Result<(), String> {
    WebviewWindowBuilder::new(
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
    .map(drop)
    .map_err(text)
}

fn show_capsule(app: &App, at: Monitor) -> Result<(), String> {
    let window = app
        .get_webview_window(CAPSULE_LABEL)
        .ok_or("the capsule is gone")?;
    let (x, y) = at.capsule();
    let shown = window
        .set_position(PhysicalPosition::new(x, y))
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
        on_main(&self.app, |app| {
            app.available_monitors()
                .map(|monitors| monitors.iter().map(monitor).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
    }

    fn open(&self, at: &Monitor) -> Opened {
        let at = *at;
        *lock(&self.origin) = (0.0, 0.0);
        *self.ink() = None;
        self.ready
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let late = || Opened {
            ink: Err(LATE.to_owned()),
            capsule: Err(LATE.to_owned()),
        };
        let built = on_main(&self.app, move |app| {
            destroy(app, INK_LABEL);
            destroy(app, CAPSULE_LABEL);
            (build_ink(app, at), build_capsule(app))
        });
        let Some((ink, capsule)) = built else {
            self.close();
            return late();
        };
        // Shown only once their pages mounted: never the app's loading screen.
        let deadline = Instant::now() + READY_WITHIN;
        let ink_ready = ink.is_ok() && self.wait_ready(INK_LABEL, deadline);
        let capsule_ready = capsule.is_ok() && self.wait_ready(CAPSULE_LABEL, deadline);
        let origin = self.origin.clone();
        fn unready<T>() -> Result<T, String> {
            Err("its page did not load in time".to_owned())
        }
        let shown = on_main(&self.app, move |app| {
            // The capsule second, so it starts above the overlay.
            let ink = match ink {
                Ok(xid) if ink_ready => show_ink(app, at, origin).map(|()| xid),
                Ok(_) => {
                    destroy(app, INK_LABEL);
                    unready()
                }
                Err(error) => Err(error),
            };
            let capsule = match capsule {
                Ok(()) if capsule_ready => show_capsule(app, at),
                Ok(()) => {
                    destroy(app, CAPSULE_LABEL);
                    unready()
                }
                Err(error) => Err(error),
            };
            (ink, capsule)
        });
        let Some((ink, capsule)) = shown else {
            self.close();
            return late();
        };
        let ink = ink.and_then(|xid| {
            let verified = match self.x11.passes_pointer(xid) {
                Ok(true) => Ok(xid),
                Ok(false) => Err("the overlay would catch the pointer".to_owned()),
                Err(error) => Err(error),
            };
            if verified.is_err() {
                let answer = on_main(&self.app, |app| destroy(app, INK_LABEL));
                x11::closed(answer, Some(xid), self.x11.as_ref());
            }
            verified
        });
        if let Ok(xid) = ink {
            *self.ink() = Some(Ink { xid, monitor: at });
        }
        Opened {
            ink: ink.map(drop),
            capsule,
        }
    }

    fn loaded(&self, label: &str) {
        let (ready, changed) = &*self.ready;
        ready
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(label.to_owned());
        changed.notify_all();
    }

    fn ink_origin(&self) -> (f64, f64) {
        *lock(&self.origin)
    }

    fn takes_pointer(&self, takes: bool) -> Result<(), String> {
        let Some(ink) = *self.ink() else {
            return Err("the ink overlay is gone".into());
        };
        let taking = takes.then(|| ink.monitor.taking());
        let answer = on_main(&self.app, move |app| {
            let window = app
                .get_webview_window(INK_LABEL)
                .ok_or("the ink overlay is gone")?;
            input_shape(&window, taking)
        });
        x11::settle(answer, takes, ink.xid, self.x11.as_ref())
    }

    fn publish(&self, surface: &AnnotateSurface) {
        if let Err(error) = surface.clone().emit(&self.app) {
            tracing::warn!(%error, "annotate surface");
        }
    }

    fn close_ink(&self) {
        let xid = self.ink().take().map(|ink| ink.xid);
        let answer = on_main(&self.app, |app| destroy(app, INK_LABEL));
        x11::closed(answer, xid, self.x11.as_ref());
    }

    fn close(&self) {
        let xid = self.ink().take().map(|ink| ink.xid);
        let answer = on_main(&self.app, |app| {
            destroy(app, INK_LABEL);
            destroy(app, CAPSULE_LABEL);
        });
        x11::closed(answer, xid, self.x11.as_ref());
    }
}
