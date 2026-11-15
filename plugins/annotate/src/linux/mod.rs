//! Annotate's native layer on Linux, answering with the macOS layer's JSON
//! so the session and the bundle stay shared. Capture's recorder records
//! the monitor the desktop's screen-sharing dialog picks (remembered with
//! the portal's restore token), two Tauri windows show the ink and the
//! capsule over it, and GStreamer children read the movie back.
//!
//! Failing safe comes first: the overlay ignores the pointer unless ink is
//! on. Ink goes off with Escape or a right click on the overlay, the
//! shortcut, the capsule's pen, and after 30 s without pointer movement.
//! Both windows close on stop, cancel, the stream ending by itself and
//! before every start. An overlay that cannot be made to ignore the pointer
//! is closed, and the recording goes on without ink.

mod frames;
mod ink;
mod windows;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anlg_annotate_macos::{AnnotateError, codes};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri_plugin_capture::recorder::{
    AreaPicker, Cancel, Desktop, Events, Gst, Leftovers, Parts, Picked, Recorder, Stream, Tools,
};

use crate::audio::{Clock, HostClock};
use crate::bundle::CANDIDATES_DIR;
use crate::commands::InkMode;
use crate::events::AnnotateSurface;
use crate::native::{CANCELLED, Native};
use crate::session::CapsuleLabels;

use frames::{Decoder, Gstreamer, Movie};
use ink::{Machine, Press, Strokes};
use windows::{Surfaces, Windows};

const FPS: u32 = 30;
/// A recording stops by itself after 20 minutes, as on macOS.
const MAX_DURATION_S: u32 = 20 * 60;

const NO_PLACING: &str = "Librelog runs as a native Wayland app here, where it can't keep a window over the screen, so this annotation has no ink and no capsule. Start Librelog without LIBRELOG_WAYLAND=1 to draw.";
const NO_MONITOR: &str = "Librelog could not tell which screen the desktop shared, so this annotation has no ink and no capsule.";
const NO_INK: &str = "The drawing layer could not be shown, so this annotation has no ink.";
const INK_LOST: &str =
    "The drawing layer stopped responding and was closed, so ink stopped part way.";

pub(crate) fn fail(code: &str, message: impl std::fmt::Display) -> AnnotateError {
    AnnotateError {
        code: code.to_owned(),
        message: message.to_string(),
    }
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, AnnotateError> {
    serde_json::from_str(json).map_err(|error| fail(codes::INVALID_ARGUMENT, error))
}

/// Capture's recorder, behind a trait for tests.
pub trait Screen: Send + Sync {
    fn start(&self, request: &str) -> Result<String, AnnotateError>;
    fn stop(&self) -> Result<String, AnnotateError>;
    fn cancel(&self) -> Result<(), AnnotateError>;
    /// How long the movie that records has been recording.
    fn elapsed_ms(&self) -> u64;
    fn stream(&self) -> Option<Stream>;
    fn events(&self) -> Vec<Value>;
}

struct Capture {
    recorder: Recorder,
    events: Arc<Events>,
}

impl Screen for Capture {
    fn start(&self, request: &str) -> Result<String, AnnotateError> {
        self.recorder.start(request)
    }

    fn stop(&self) -> Result<String, AnnotateError> {
        self.recorder.stop()
    }

    fn cancel(&self) -> Result<(), AnnotateError> {
        self.recorder.cancel()
    }

    fn elapsed_ms(&self) -> u64 {
        serde_json::from_str::<Value>(&self.recorder.status())
            .ok()
            .and_then(|status| status["elapsed_ms"].as_u64())
            .unwrap_or(0)
    }

    fn stream(&self) -> Option<Stream> {
        self.recorder.stream()
    }

    fn events(&self) -> Vec<Value> {
        self.events.drain()
    }
}

/// Annotate records whole screens: the selector never shows.
struct WholeScreen;

impl AreaPicker for WholeScreen {
    fn pick(
        &self,
        _frame: &image::DynamicImage,
        _hint: &str,
        _cancel: &Cancel,
    ) -> Result<Picked, AnnotateError> {
        Ok(Picked::Whole)
    }
}

/// Whether windows can be placed and kept on top: X11, or XWayland.
fn placeable() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    match std::env::var("GDK_BACKEND") {
        Ok(backend) if !backend.is_empty() => backend.split(',').next() == Some("x11"),
        _ => !set("WAYLAND_DISPLAY") && set("DISPLAY"),
    }
}

fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

#[derive(Default, Deserialize)]
struct LabelsRequest {
    discard_question: Option<String>,
    discard: Option<String>,
    keep: Option<String>,
}

#[derive(Deserialize)]
struct StartRequest {
    dir: PathBuf,
    #[serde(default)]
    labels: Option<LabelsRequest>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct StopReply {
    video_path: PathBuf,
    duration_ms: u64,
    ended_by: String,
    warnings: Vec<String>,
}

struct Current {
    dir: PathBuf,
    /// The movie's start on the host clock.
    zero_ns: u64,
    wall_ms: i64,
    /// `(width_pt, height_pt, scale)`.
    display: (f64, f64, f64),
    machine: Machine,
    strokes: Strokes,
    /// The overlay is up, ignoring the pointer while ink is off.
    ink: bool,
    labels: CapsuleLabels,
    warnings: Vec<String>,
    /// The recorder stopped by itself; `stop` collects it.
    ended: bool,
}

enum State {
    Idle,
    Starting { cancelled: bool },
    Recording(Box<Current>),
}

pub struct Linux {
    screen: Arc<dyn Screen>,
    decoder: Arc<dyn Decoder>,
    surfaces: Arc<dyn Surfaces>,
    clock: Arc<dyn Clock>,
    placeable: bool,
    state: Mutex<State>,
    /// The last stopped movie, for its key frames.
    movie: Mutex<Option<Movie>>,
}

impl Linux {
    /// The real one; `root` is the annotations folder, where the portal's
    /// token for the chosen screen is kept.
    pub fn desktop(app: tauri::AppHandle<tauri::Wry>, root: &Path) -> std::io::Result<Self> {
        let desktop = Arc::new(Desktop::new()?);
        let gst: Arc<dyn Gst> = Arc::new(Tools::default());
        let events = Arc::new(Events::default());
        let recorder = Recorder::new(Parts {
            shots: desktop.clone(),
            leftovers: Leftovers::default(),
            cast: desktop,
            gst: gst.clone(),
            picker: Arc::new(WholeScreen),
            events: events.clone(),
            token_file: root.join("screencast-token"),
            // As Capture's: the longest wait for an encoder's first bytes.
            probe: Duration::from_secs(5),
            tick: Duration::from_millis(100),
        });
        Ok(Self::new(
            Arc::new(Capture { recorder, events }),
            Arc::new(Gstreamer(gst)),
            Arc::new(Windows::new(app)),
            Arc::new(HostClock),
            placeable(),
        ))
    }

    fn new(
        screen: Arc<dyn Screen>,
        decoder: Arc<dyn Decoder>,
        surfaces: Arc<dyn Surfaces>,
        clock: Arc<dyn Clock>,
        placeable: bool,
    ) -> Self {
        Self {
            screen,
            decoder,
            surfaces,
            clock,
            placeable,
            state: Mutex::new(State::Idle),
            movie: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn movie(&self) -> std::sync::MutexGuard<'_, Option<Movie>> {
        self.movie.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now(&self, current: &Current) -> u64 {
        self.clock.now_ns().saturating_sub(current.zero_ns) / 1_000_000
    }

    fn surface_of(current: &Current) -> AnnotateSurface {
        let ink = current.machine.ink();
        AnnotateSurface {
            ink,
            pen_on: ink == InkMode::Drag,
            asking: current.machine.asking(),
            started_wall_ms: current.wall_ms,
            labels: current.labels.clone(),
        }
    }

    /// What the overlay and the capsule show while recording.
    pub fn surface(&self) -> Option<AnnotateSurface> {
        match &*self.lock() {
            State::Recording(current) if !current.ended => Some(Self::surface_of(current)),
            _ => None,
        }
    }

    /// The live stroke ends, the overlay takes or gives back the pointer,
    /// and the windows show the new mode. An overlay that can't take the
    /// pointer leaves ink off; one that can't give it back is closed.
    fn ink_changed(&self, current: &mut Current) {
        current.strokes.end();
        if current.ink {
            let takes = current.machine.ink() != InkMode::Off;
            if let Err(error) = self.surfaces.takes_pointer(takes) {
                tracing::warn!(%error, takes, "annotate ink overlay");
                current.machine.escape();
                if !takes || self.surfaces.takes_pointer(false).is_err() {
                    self.surfaces.close_ink();
                    current.ink = false;
                    current.warnings.push(INK_LOST.into());
                }
            }
        }
        self.surfaces.publish(&Self::surface_of(current));
    }

    fn begin(&self, request: &StartRequest) -> Result<String, AnnotateError> {
        self.surfaces.close();
        *self.movie() = None;
        let video = request.dir.join(anlg_annotate_core::VIDEO_FILE);
        if video.exists() {
            return Err(fail(
                codes::INVALID_ARGUMENT,
                format!("{} already exists", video.display()),
            ));
        }
        let recording = json!({
            "mode": "fullscreen", "out_path": video, "fps": FPS, "show_cursor": true,
            "microphone": false, "system_audio": false, "countdown_s": 0,
            "max_duration_s": MAX_DURATION_S,
        });
        let reply: Value = serde_json::from_str(&self.screen.start(&recording.to_string())?)
            .map_err(|error| fail(codes::INTERNAL, error))?;
        if reply["status"] != "recording" {
            return Err(fail(CANCELLED, "the screen-sharing dialog was cancelled"));
        }
        let elapsed = self.screen.elapsed_ms();
        let zero_ns = self
            .clock
            .now_ns()
            .saturating_sub(elapsed.saturating_mul(1_000_000));
        let wall_ms = wall_ms() - elapsed as i64;
        let stream = self.screen.stream();
        let monitors = if self.placeable {
            self.surfaces.monitors()
        } else {
            Vec::new()
        };
        let monitor = windows::pick(stream.as_ref(), &monitors).map(|i| monitors[i]);
        let display = match monitor {
            Some(monitor) => {
                let (width, height) = monitor.points();
                (width, height, monitor.scale)
            }
            None => {
                let size = stream.and_then(|stream| stream.size).map_or_else(
                    || {
                        (
                            reply["width_px"].as_f64().unwrap_or(0.0),
                            reply["height_px"].as_f64().unwrap_or(0.0),
                        )
                    },
                    |(width, height)| (f64::from(width), f64::from(height)),
                );
                (size.0, size.1, 1.0)
            }
        };
        let mut warnings = Vec::new();
        let mut ink = false;
        match monitor {
            _ if !self.placeable => warnings.push(NO_PLACING.to_owned()),
            None => warnings.push(NO_MONITOR.to_owned()),
            Some(monitor) => {
                let opened = self.surfaces.open(&monitor);
                if let Err(error) = &opened.capsule {
                    tracing::warn!(%error, "annotate capsule");
                }
                match opened.ink {
                    Ok(()) => ink = true,
                    Err(error) => {
                        tracing::warn!(%error, "annotate ink overlay");
                        warnings.push(NO_INK.to_owned());
                    }
                }
            }
        }
        let labels = request
            .labels
            .as_ref()
            .map_or_else(CapsuleLabels::default, |labels| CapsuleLabels {
                discard_question: labels.discard_question.clone(),
                discard: labels.discard.clone(),
                keep: labels.keep.clone(),
            });
        let current = Current {
            dir: request.dir.clone(),
            zero_ns,
            wall_ms,
            display,
            machine: Machine::new(0),
            strokes: Strokes::new((display.0, display.1)),
            ink,
            labels,
            warnings,
            ended: false,
        };
        let mut state = self.lock();
        if matches!(*state, State::Starting { cancelled: true }) {
            drop(state);
            self.surfaces.close();
            let _ = self.screen.cancel();
            return Err(fail(
                CANCELLED,
                "the recording was cancelled while starting",
            ));
        }
        self.surfaces.publish(&Self::surface_of(&current));
        *state = State::Recording(Box::new(current));
        Ok(json!({
            "started_host_ns": zero_ns,
            "started_wall_ms": wall_ms,
            "display": { "width_pt": display.0, "height_pt": display.1, "scale": display.2 },
        })
        .to_string())
    }

    /// The overlay's pointer, in its own points: `move`, `down`, `up` or
    /// `leave`.
    pub fn pointer(&self, kind: &str, x: f64, y: f64) {
        let mut state = self.lock();
        let State::Recording(current) = &mut *state else {
            return;
        };
        if !current.ink || current.ended {
            return;
        }
        let now = self.now(current);
        current.machine.pointer_moved(now);
        let (left, top) = self.surfaces.ink_origin();
        let (x, y) = (x + left, y + top);
        let strokes = &mut current.strokes;
        match (current.machine.ink(), kind) {
            (InkMode::Hover, "leave") => strokes.end(),
            (InkMode::Hover, _) if strokes.live() => strokes.add(x, y, now),
            (InkMode::Hover, _) => strokes.begin(InkMode::Hover, x, y, now),
            (InkMode::Drag, "down") => strokes.begin(InkMode::Drag, x, y, now),
            (InkMode::Drag, "move") => strokes.add(x, y, now),
            (InkMode::Drag, "up") => {
                strokes.add(x, y, now);
                strokes.end();
            }
            _ => {}
        }
    }

    /// Escape, or a right click, on the overlay: ink off.
    pub fn escape(&self) {
        let mut state = self.lock();
        if let State::Recording(current) = &mut *state
            && !current.ended
            && current.machine.escape()
        {
            self.ink_changed(current);
        }
    }

    pub fn press(&self, target: &str) {
        let Some(press) = Press::parse(target) else {
            return;
        };
        let mut state = self.lock();
        let State::Recording(current) = &mut *state else {
            return;
        };
        if current.ended {
            return;
        }
        let now = self.now(current);
        if current.machine.press(press, now) {
            self.ink_changed(current);
        } else {
            self.surfaces.publish(&Self::surface_of(current));
        }
    }
}

impl Native for Linux {
    /// Nothing to grant ahead: the desktop asks when a recording starts.
    fn probe_screen_recording(&self) -> Result<String, AnnotateError> {
        Ok("granted".into())
    }

    fn request_screen_recording(&self) -> Result<bool, AnnotateError> {
        Ok(true)
    }

    fn open_screen_recording_settings(&self) -> Result<(), AnnotateError> {
        Err(fail(
            codes::UNSUPPORTED,
            "Linux has no Screen Recording setting: the desktop asks when a recording starts",
        ))
    }

    fn start(&self, request: &str) -> Result<String, AnnotateError> {
        let request: StartRequest = parse(request)?;
        if !request.dir.is_absolute() {
            return Err(fail(
                codes::INVALID_ARGUMENT,
                "dir must be an absolute path",
            ));
        }
        {
            let mut state = self.lock();
            if !matches!(*state, State::Idle) {
                return Err(fail(codes::BUSY, "an annotation is already recording"));
            }
            *state = State::Starting { cancelled: false };
        }
        let started = self.begin(&request);
        if started.is_err() {
            *self.lock() = State::Idle;
            self.surfaces.close();
        }
        started
    }

    fn stop(&self) -> Result<String, AnnotateError> {
        let current = {
            let mut state = self.lock();
            match std::mem::replace(&mut *state, State::Idle) {
                State::Recording(current) => current,
                other => {
                    *state = other;
                    return Err(fail(codes::NOT_RECORDING, "nothing is recording"));
                }
            }
        };
        self.surfaces.close();
        let Current {
            dir,
            zero_ns,
            wall_ms,
            display,
            strokes,
            mut warnings,
            ..
        } = *current;
        let strokes = strokes.finish();
        let reply: StopReply = serde_json::from_str(&self.screen.stop()?)
            .map_err(|error| fail(codes::INTERNAL, error))?;
        warnings.extend(reply.warnings);
        let ended_by = match reply.ended_by.as_str() {
            "cap" => "max_duration",
            "" => "stop",
            other => other,
        };
        let duration_ms = reply.duration_ms as i64;
        let spans: Vec<(u64, u64)> = strokes.iter().map(|s| (s.t0(), s.t1())).collect();
        let sampled = frames::sample(
            self.decoder.as_ref(),
            &reply.video_path,
            &dir.join(CANDIDATES_DIR),
            duration_ms,
            frames::episodes(&spans),
        );
        let (candidates, offset) = match sampled {
            Ok(sampled) => {
                let offset = sampled.movie.offset;
                *self.movie() = Some(sampled.movie);
                (sampled.candidates, offset)
            }
            Err(error) => {
                warnings.push(format!(
                    "No key frames could be read from the screen recording: {error}"
                ));
                (Vec::new(), 0)
            }
        };
        Ok(json!({
            "video_path": reply.video_path,
            "started_host_ns": zero_ns,
            "started_wall_ms": wall_ms,
            "duration_ms": duration_ms,
            "video_offset_ms": offset.max(0),
            "display": { "width_pt": display.0, "height_pt": display.1, "scale": display.2 },
            "front_app": {},
            "candidates": candidates
                .iter()
                .map(|(t, path)| json!({ "t_ms": t, "path": path }))
                .collect::<Vec<_>>(),
            "strokes": strokes.iter().map(ink::Stroke::to_json).collect::<Vec<_>>(),
            "ended_by": ended_by,
            "warnings": warnings,
        })
        .to_string())
    }

    fn cancel(&self) -> Result<(), AnnotateError> {
        let dir = {
            let mut state = self.lock();
            match std::mem::replace(&mut *state, State::Idle) {
                State::Starting { .. } => {
                    *state = State::Starting { cancelled: true };
                    None
                }
                State::Recording(current) => Some(current.dir),
                State::Idle => None,
            }
        };
        self.surfaces.close();
        // Also takes down the desktop's dialog while it is up.
        let cancelled = self.screen.cancel();
        if let Some(dir) = dir {
            let _ = std::fs::remove_dir_all(dir.join(CANDIDATES_DIR));
        }
        *self.movie() = None;
        cancelled
    }

    fn set_ink_mode(&self, mode: &str) -> Result<(), AnnotateError> {
        let mode = match mode {
            "off" => InkMode::Off,
            "hover" => InkMode::Hover,
            "drag" => InkMode::Drag,
            other => {
                return Err(fail(
                    codes::INVALID_ARGUMENT,
                    format!("no ink mode {other}"),
                ));
            }
        };
        let mut state = self.lock();
        let State::Recording(current) = &mut *state else {
            return Err(fail(codes::NOT_RECORDING, "nothing is recording"));
        };
        if current.ended {
            return Err(fail(codes::NOT_RECORDING, "the recording ended"));
        }
        let now = self.now(current);
        if current.machine.set_ink(mode, now) {
            self.ink_changed(current);
        }
        Ok(())
    }

    /// Also where the safety timers run: the session polls every 100 ms
    /// while it records.
    fn poll_events(&self) -> Result<String, AnnotateError> {
        let mut state = self.lock();
        let State::Recording(current) = &mut *state else {
            return Ok("[]".into());
        };
        for event in self.screen.events() {
            if event["type"] == "record_stopped" && !current.ended {
                current.ended = true;
                current.strokes.end();
                self.surfaces.close();
                current.ink = false;
                current.machine.stopped(match event["reason"].as_str() {
                    Some("cap") => "max_duration",
                    _ => "stream_error",
                });
            }
        }
        if !current.ended {
            let now = self.now(current);
            let effects = current.machine.tick(now);
            if effects.ink_off {
                self.ink_changed(current);
            } else if effects.question_closed {
                self.surfaces.publish(&Self::surface_of(current));
            }
        }
        Ok(Value::Array(current.machine.drain()).to_string())
    }

    fn extract_frames(&self, request: &str) -> Result<String, AnnotateError> {
        let movie = self.movie().clone();
        frames::extract(self.decoder.as_ref(), request, movie.as_ref())
    }
}
