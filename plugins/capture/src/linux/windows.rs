//! The two windows Capture shows on Linux, as Tauri webviews: the card
//! stack in the screen's corner (`/app/capture-cards`) and the frozen-
//! screen area selector (`/app/capture-area`). They are frameless, on top
//! and out of the taskbar, like the app's other overlays, and talk to the
//! plugin through its `cardStack`, `cardPressed`, `cardHovered`,
//! `areaFrame` and `areaChosen` commands.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anlg_annotate_macos::{AnnotateError, codes};
use image::DynamicImage;
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_specta::Event;

use super::area::{self, Monitor, Region, Selection};
use super::cards::CardView;
use super::fail;
use super::gst::Rect;
use super::record::{AreaPicker, Cancel, Picked};
use crate::types::{AreaChoice, AreaFrame, AreaRect, CaptureCards};

pub const CARDS_LABEL: &str = "capture-cards";
pub const AREA_LABEL: &str = "capture-area";
/// The route's card: thumbnail and buttons, in logical pixels.
const CARD_WIDTH: f64 = 288.0;
const CARD_HEIGHT: f64 = 176.0;
const CARD_GAP: f64 = 12.0;
const INSET: f64 = 16.0;
const PICK_WITHIN: Duration = Duration::from_secs(5 * 60);
/// A selector window that never appeared counts as cancelled.
const SHOW_WITHIN: Duration = Duration::from_secs(20);
const FRAME_PREFIX: &str = "area-frame-";

type App = tauri::AppHandle<tauri::Wry>;

pub struct CardWindow {
    pub app: App,
}

fn stack_height(cards: usize) -> f64 {
    let cards = cards as f64;
    cards * CARD_HEIGHT + (cards - 1.0).max(0.0) * CARD_GAP
}

impl CardView for CardWindow {
    fn show(&self, cards: &CaptureCards) {
        let _ = cards.clone().emit(&self.app);
        let app = self.app.clone();
        let count = cards.cards.len();
        let shown = self.app.run_on_main_thread(move || {
            if count == 0 {
                if let Some(window) = app.get_webview_window(CARDS_LABEL) {
                    let _ = window.hide();
                }
                return;
            }
            let window = match app.get_webview_window(CARDS_LABEL) {
                Some(window) => window,
                None => match WebviewWindowBuilder::new(
                    &app,
                    CARDS_LABEL,
                    WebviewUrl::App("app/capture-cards".into()),
                )
                .title("Librelog")
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible_on_all_workspaces(true)
                .focused(false)
                .focusable(false)
                .visible(false)
                .disable_drag_drop_handler()
                .build()
                {
                    Ok(window) => window,
                    Err(error) => {
                        tracing::warn!(%error, "capture card window");
                        return;
                    }
                },
            };
            let height = stack_height(count);
            let _ = window.set_size(LogicalSize::new(CARD_WIDTH, height));
            let monitor = app.primary_monitor().ok().flatten().or_else(|| {
                app.available_monitors()
                    .ok()
                    .and_then(|monitors| monitors.into_iter().next())
            });
            if let Some(monitor) = monitor {
                let scale = monitor.scale_factor();
                let area = monitor.work_area();
                let origin = area.position.to_logical::<f64>(scale);
                let size = area.size.to_logical::<f64>(scale);
                let _ = window.set_position(LogicalPosition::new(
                    origin.x + size.width - CARD_WIDTH - INSET,
                    origin.y + size.height - height - INSET,
                ));
            }
            let _ = window.show();
        });
        if let Err(error) = shown {
            tracing::warn!(%error, "capture card window");
        }
    }
}

struct Pending {
    frame: PathBuf,
    size: (u32, u32),
    hint: String,
    /// Filled once the selector asked where it is.
    placed: Option<(Region, Monitor, usize)>,
    choice: Option<AreaChoice>,
}

pub struct AreaWindow {
    pub app: App,
    /// Where the frozen picture is written for the selector to show.
    pub frames: PathBuf,
    pending: Mutex<Option<Pending>>,
}

fn monitor(monitor: &tauri::Monitor) -> Monitor {
    Monitor {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
        scale: monitor.scale_factor(),
    }
}

impl AreaWindow {
    pub fn new(app: App, frames: PathBuf) -> Self {
        Self {
            app,
            frames,
            pending: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// What the selector shows: the frozen picture, the part of it under
    /// the selector's own monitor, and the hint.
    pub fn frame(&self) -> Option<AreaFrame> {
        let (path, size, hint) = {
            let pending = self.lock();
            let pending = pending.as_ref()?;
            (pending.frame.clone(), pending.size, pending.hint.clone())
        };
        let window = self.app.get_webview_window(AREA_LABEL);
        let monitors: Vec<Monitor> = self
            .app
            .available_monitors()
            .map(|monitors| monitors.iter().map(monitor).collect())
            .unwrap_or_default();
        let this = window
            .and_then(|window| window.current_monitor().ok().flatten())
            .map(|this| monitor(&this))
            .or_else(|| monitors.first().copied())
            .unwrap_or(Monitor {
                x: 0,
                y: 0,
                width: size.0,
                height: size.1,
                scale: 1.0,
            });
        let region = area::region(size, &monitors, this);
        if let Some(pending) = self.lock().as_mut() {
            pending.placed = Some((region, this, monitors.len().max(1)));
        }
        Some(AreaFrame {
            image_path: path.to_string_lossy().into_owned(),
            image_width: size.0,
            image_height: size.1,
            region: AreaRect {
                x: region.rect.x,
                y: region.rect.y,
                width: region.rect.width,
                height: region.rect.height,
            },
            hint,
        })
    }

    pub fn choose(&self, choice: AreaChoice) {
        if let Some(pending) = self.lock().as_mut() {
            pending.choice.get_or_insert(choice);
        }
    }

    fn sweep(&self) {
        let Ok(files) = std::fs::read_dir(&self.frames) else {
            return;
        };
        for file in files.filter_map(Result::ok) {
            if file.file_name().to_string_lossy().starts_with(FRAME_PREFIX) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }

    fn close_window(&self) {
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            if let Some(window) = app.get_webview_window(AREA_LABEL) {
                let _ = window.destroy();
            }
        });
    }
}

fn save_frame(frame: &DynamicImage, path: &Path) -> Result<(), AnnotateError> {
    // Fast over small: it is read once, at once, then deleted.
    let file = std::fs::File::create(path).map_err(|error| fail(codes::IO, error))?;
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    frame
        .to_rgba8()
        .write_with_encoder(encoder)
        .map_err(|error| fail(codes::IO, error))
}

impl AreaPicker for AreaWindow {
    fn pick(
        &self,
        frame: &DynamicImage,
        hint: &str,
        cancel: &Cancel,
    ) -> Result<Picked, AnnotateError> {
        anlg_capture_core::fs::ensure_private_dir(&self.frames)
            .map_err(|error| fail(codes::IO, error))?;
        self.sweep();
        let path = self
            .frames
            .join(format!("{FRAME_PREFIX}{}.png", uuid_like()));
        save_frame(frame, &path)?;
        *self.lock() = Some(Pending {
            frame: path.clone(),
            size: (frame.width(), frame.height()),
            hint: hint.to_owned(),
            placed: None,
            choice: None,
        });
        let app = self.app.clone();
        let built = self.app.run_on_main_thread(move || {
            if app.get_webview_window(AREA_LABEL).is_some() {
                return;
            }
            let built = WebviewWindowBuilder::new(
                &app,
                AREA_LABEL,
                WebviewUrl::App("app/capture-area".into()),
            )
            .title("Librelog")
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .fullscreen(true)
            .focused(true)
            .disable_drag_drop_handler()
            .build();
            if let Err(error) = built {
                tracing::warn!(%error, "capture area window");
            }
        });
        let asked = Instant::now();
        let mut seen = false;
        let choice = loop {
            std::thread::sleep(Duration::from_millis(50));
            if let Some(choice) = self
                .lock()
                .as_ref()
                .and_then(|pending| pending.choice.clone())
            {
                break choice;
            }
            let up = self.app.get_webview_window(AREA_LABEL).is_some();
            seen |= up;
            // Closed some other way, never shown, cancelled or forgotten.
            if built.is_err()
                || (seen && !up)
                || (!seen && asked.elapsed() >= SHOW_WITHIN)
                || cancel.cancelled()
                || asked.elapsed() >= PICK_WITHIN
            {
                break AreaChoice::Cancel;
            }
        };
        let pending = self.lock().take();
        self.close_window();
        let _ = std::fs::remove_file(&path);
        Ok(match choice {
            AreaChoice::Cancel => Picked::Cancelled,
            AreaChoice::Whole => Picked::Whole,
            AreaChoice::Window => Picked::Window,
            AreaChoice::Area {
                x,
                y,
                width,
                height,
            } => {
                let size = (frame.width(), frame.height());
                let (region, monitor, monitors) =
                    pending.and_then(|pending| pending.placed).unwrap_or((
                        Region {
                            rect: Rect {
                                x: 0,
                                y: 0,
                                width: size.0,
                                height: size.1,
                            },
                            trusted: false,
                        },
                        Monitor {
                            x: 0,
                            y: 0,
                            width: size.0,
                            height: size.1,
                            scale: 1.0,
                        },
                        1,
                    ));
                Picked::Area(Selection {
                    region,
                    rect: Rect {
                        x,
                        y,
                        width,
                        height,
                    },
                    monitor,
                    monitors,
                })
            }
        })
    }
}

fn uuid_like() -> String {
    format!(
        "{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    )
}
