//! Capture's native layer on Linux, answering the same JSON as the macOS
//! one (`anlg_annotate_macos::capture`), so the plugin's session runs
//! unchanged on top of it: the desktop's portals for screenshots and screen
//! sharing, GStreamer children for movies, GIFs and video thumbnails, the
//! X11 clipboard (bridged to Wayland by the desktop) and Tauri windows for
//! the cards and the area selector.

mod area;
mod cards;
mod clipboard;
mod gif;
mod gst;
mod media;
mod portal;
mod record;
mod screenshot;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anlg_annotate_macos::{AnnotateError, codes};
use serde::Deserialize;

use crate::native::Native;
use crate::types::{AreaChoice, AreaFrame, CaptureCard, CaptureCards, CardLabels};

pub(crate) fn fail(code: &str, message: impl std::fmt::Display) -> AnnotateError {
    AnnotateError {
        code: code.to_owned(),
        message: message.to_string(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, AnnotateError> {
    serde_json::from_str(json).map_err(|error| fail(codes::INVALID_ARGUMENT, error))
}

/// What `poll_events` hands the plugin next.
#[derive(Default)]
pub struct Events(Mutex<Vec<serde_json::Value>>);

impl Events {
    pub fn push(&self, event: serde_json::Value) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }

    pub fn drain(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub struct Linux {
    shots: Arc<dyn screenshot::Shots>,
    leftovers: screenshot::Leftovers,
    gst: Arc<dyn gst::Gst>,
    recorder: record::Recorder,
    clipboard: Arc<dyn clipboard::Clipboard>,
    cards: Arc<cards::Cards>,
    events: Arc<Events>,
}

/// What the overlay windows' commands reach.
pub struct Overlays {
    cards: Arc<cards::Cards>,
    area: Arc<windows::AreaWindow>,
}

impl Overlays {
    pub fn cards(&self) -> CaptureCards {
        self.cards.current()
    }

    pub fn press(&self, id: &str, action: &str) -> bool {
        self.cards.press(id, action)
    }

    pub fn hover(&self, id: &str, hovered: bool) {
        self.cards.hover(id, hovered, Instant::now());
    }

    pub fn area_frame(&self) -> Option<AreaFrame> {
        self.area.frame()
    }

    pub fn choose_area(&self, choice: AreaChoice) {
        self.area.choose(choice);
    }
}

impl Linux {
    /// The real one. `root` is the captures folder: the screen-sharing
    /// token and the selector's frozen picture live there.
    pub fn desktop(
        app: tauri::AppHandle<tauri::Wry>,
        root: &Path,
        pictures: Option<PathBuf>,
    ) -> std::io::Result<(Arc<Self>, Overlays)> {
        let desktop = Arc::new(portal::Desktop::new()?);
        let events = Arc::new(Events::default());
        let cards = Arc::new(cards::Cards::new(
            Box::new(windows::CardWindow { app: app.clone() }),
            events.clone(),
        ));
        let area = Arc::new(windows::AreaWindow::new(app, root.join("thumbs")));
        let leftovers = screenshot::Leftovers {
            temp: Some(std::env::temp_dir()),
            pictures,
        };
        let gst: Arc<dyn gst::Gst> = Arc::new(gst::Tools::default());
        let recorder = record::Recorder::new(record::Parts {
            shots: desktop.clone(),
            leftovers: leftovers.clone(),
            cast: desktop.clone(),
            gst: gst.clone(),
            picker: area.clone(),
            events: events.clone(),
            token_file: root.join("screencast-token"),
            probe: Duration::from_millis(1500),
            tick: Duration::from_millis(100),
        });
        let linux = Arc::new(Self {
            shots: desktop,
            leftovers,
            gst,
            recorder,
            clipboard: Arc::new(clipboard::Desktop),
            cards: cards.clone(),
            events,
        });
        Ok((linux, Overlays { cards, area }))
    }
}

#[derive(Deserialize)]
struct CardRequest {
    id: String,
    kind: String,
    #[serde(default)]
    thumbnail_path: String,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    saved: bool,
    #[serde(default)]
    labels: CardLabels,
}

#[derive(Deserialize)]
struct CloseRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
struct CopyRequest {
    path: PathBuf,
    kind: String,
}

impl Native for Linux {
    fn available(&self) -> bool {
        true
    }

    fn screenshot(&self, request: &str) -> Result<String, AnnotateError> {
        // Our own cards are not in the picture.
        self.cards.close(None);
        screenshot::take(self.shots.as_ref(), &self.leftovers, request)
    }

    fn record_start(&self, request: &str) -> Result<String, AnnotateError> {
        // Nor in the movie: nothing of ours can be left out of it.
        self.cards.close(None);
        self.recorder.start(request)
    }

    fn record_stop(&self) -> Result<String, AnnotateError> {
        self.recorder.stop()
    }

    fn record_cancel(&self) -> Result<(), AnnotateError> {
        self.recorder.cancel()
    }

    fn record_status(&self) -> Result<String, AnnotateError> {
        Ok(self.recorder.status())
    }

    fn poll_events(&self) -> Result<String, AnnotateError> {
        self.cards.expire(Instant::now());
        Ok(serde_json::Value::Array(self.events.drain()).to_string())
    }

    fn make_gif(&self, request: &str) -> Result<String, AnnotateError> {
        let video = serde_json::from_str::<serde_json::Value>(request)
            .ok()
            .and_then(|request| request["video"].as_str().map(PathBuf::from));
        let duration = video.and_then(|video| self.recorder.duration_of(&video));
        media::make_gif(self.gst.as_ref(), request, duration)
    }

    fn thumbnail(&self, request: &str) -> Result<String, AnnotateError> {
        media::thumbnail(self.gst.as_ref(), request)
    }

    fn copy(&self, request: &str) -> Result<(), AnnotateError> {
        let request: CopyRequest = parse(request)?;
        self.clipboard
            .set(clipboard::formats(&request.path, &request.kind)?)
    }

    fn card_show(&self, request: &str) -> Result<(), AnnotateError> {
        let request: CardRequest = parse(request)?;
        self.cards.show(
            CaptureCard {
                id: request.id,
                kind: request.kind,
                thumbnail_path: request.thumbnail_path,
                saved: request.saved,
                duration_ms: request.duration_ms,
            },
            request.labels,
            Instant::now(),
        );
        Ok(())
    }

    fn card_close(&self, request: &str) -> Result<(), AnnotateError> {
        let request: CloseRequest = parse(request)?;
        match (request.all, request.id) {
            (true, _) => self.cards.close(None),
            (false, Some(id)) => self.cards.close(Some(&id)),
            (false, None) => {}
        }
        Ok(())
    }

    fn reveal(&self, path: &Path) -> Result<(), AnnotateError> {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| fail(codes::IO, error))
    }

    /// Through GIO's Trash when `gio` is installed (it is on GNOME).
    fn trash(&self, path: &Path) -> Result<(), AnnotateError> {
        if !gst::installed("gio") {
            return Err(fail(codes::UNSUPPORTED, "the Trash needs gio"));
        }
        let path = path.to_string_lossy();
        crate::process::run(
            "gio",
            &["trash", "--", &path],
            None,
            Duration::from_secs(20),
        )
        .map(drop)
        .map_err(|error| fail(codes::IO, error))
    }
}

#[cfg(test)]
mod tests;
