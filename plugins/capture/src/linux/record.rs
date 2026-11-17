//! Screen recordings on Linux: the ScreenCast portal's own dialog picks the
//! monitor or window (an area is drawn first on a frozen screenshot and
//! cropped out of the monitor's stream), and a `gst-launch-1.0` child
//! encodes the PipeWire stream. idle → selecting → recording → finishing →
//! idle. While recording, a watcher notices the stream ending by itself
//! (the desktop's own "stop sharing", or the pipeline dying) and the length
//! limit, and posts `record_stopped` so the plugin finishes the movie.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anlg_annotate_macos::{AnnotateError, codes};
use image::DynamicImage;
use serde::Deserialize;

use super::area::{self, Selection, Stream};
use super::gst::{self, Gst, Process, Recording, Shape};
use super::screenshot::{self, Leftovers, Shots};
use super::{Events, fail};

/// Asked to stop, a pipeline finishes its file within this, or is killed.
const STOP_WITHIN: Duration = Duration::from_secs(10);
const CANCEL_WITHIN: Duration = Duration::from_secs(3);
/// How long reading one frame to learn the stream's size may take.
const PROBE_WITHIN: Duration = Duration::from_secs(5);

/// Set to stop a selector or a portal dialog that is up.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Monitor,
    Window,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastRequest {
    pub source: Source,
    pub cursor: bool,
    /// The portal's token for the monitor picked last time.
    pub restore: Option<String>,
}

/// A started screen cast.
pub trait CastSession: Send {
    /// A fresh PipeWire remote for the session's stream: each pipeline
    /// gets its own.
    fn open_remote(&mut self) -> Result<OwnedFd, AnnotateError>;
    fn stream(&self) -> Stream;
    fn restore_token(&self) -> Option<String>;
    /// Whether the desktop ended it (its own "stop sharing").
    fn closed(&self) -> bool;
    fn close(&mut self);
}

/// The ScreenCast portal.
pub trait Cast: Send + Sync {
    /// Shows the desktop's dialog; `None` when the user (or `cancel`)
    /// cancelled it.
    fn start(
        &self,
        request: &CastRequest,
        cancel: &Cancel,
    ) -> Result<Option<Box<dyn CastSession>>, AnnotateError>;
}

pub enum Picked {
    Cancelled,
    /// The whole monitor.
    Whole,
    /// A window instead, picked in the portal's dialog.
    Window,
    Area(Selection),
}

/// The frozen-screen area selector.
pub trait AreaPicker: Send + Sync {
    fn pick(
        &self,
        frame: &DynamicImage,
        hint: &str,
        cancel: &Cancel,
    ) -> Result<Picked, AnnotateError>;
}

pub struct Parts {
    pub shots: Arc<dyn Shots>,
    pub leftovers: Leftovers,
    pub cast: Arc<dyn Cast>,
    pub gst: Arc<dyn Gst>,
    pub picker: Arc<dyn AreaPicker>,
    pub events: Arc<Events>,
    /// Where the monitor's restore token is kept.
    pub token_file: PathBuf,
    /// How long a new pipeline may take to write its first bytes before
    /// the next encoder is tried.
    pub probe: Duration,
    /// How often the watcher looks.
    pub tick: Duration,
}

struct Active {
    process: Box<dyn Process>,
    session: Box<dyn CastSession>,
    out: PathBuf,
    started: Instant,
    max: Duration,
    /// Why and when it ended by itself.
    ended: Option<(&'static str, Instant)>,
    size: (u32, u32),
    warnings: Vec<String>,
    generation: u64,
}

enum Phase {
    Idle,
    Selecting(Cancel),
    Recording(Box<Active>),
    Finishing,
}

pub struct Recorder {
    parts: Arc<Parts>,
    phase: Arc<Mutex<Phase>>,
    generation: AtomicU64,
    /// Lengths of the movies recorded since launch, for their GIFs.
    durations: Mutex<HashMap<PathBuf, u64>>,
}

#[derive(Deserialize)]
struct StartRequest {
    mode: String,
    out_path: PathBuf,
    #[serde(default = "fps")]
    fps: u32,
    #[serde(default)]
    show_cursor: bool,
    #[serde(default)]
    microphone: bool,
    #[serde(default)]
    system_audio: bool,
    #[serde(default)]
    countdown_s: u32,
    #[serde(default = "max_duration")]
    max_duration_s: u32,
    #[serde(default)]
    labels: Labels,
}

#[derive(Default, Deserialize)]
struct Labels {
    #[serde(default)]
    area_hint: String,
}

fn fps() -> u32 {
    30
}

fn max_duration() -> u32 {
    30 * 60
}

enum Begun {
    Cancelled,
    Started(Box<Active>),
}

fn cancelled() -> String {
    r#"{"status":"cancelled"}"#.into()
}

fn lock(phase: &Mutex<Phase>) -> std::sync::MutexGuard<'_, Phase> {
    phase.lock().unwrap_or_else(|e| e.into_inner())
}

impl Active {
    /// Stops the pipeline, ends the cast and deletes the movie.
    fn discard(mut self) {
        gst::finish(self.process.as_mut(), CANCEL_WITHIN);
        self.session.close();
        let _ = std::fs::remove_file(&self.out);
    }
}

/// The ScreenCast portal's token for the screen picked last, with where
/// that screen was, so an area drawn on another one asks again.
#[derive(Debug, PartialEq, serde::Serialize, Deserialize)]
struct Remembered {
    token: String,
    #[serde(default)]
    position: Option<(i32, i32)>,
    #[serde(default)]
    size: Option<(i32, i32)>,
}

impl Remembered {
    fn read(file: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(file).ok()?;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        // A plain token from before its screen was kept with it.
        Some(serde_json::from_str(text).unwrap_or_else(|_| Self {
            token: text.to_owned(),
            position: None,
            size: None,
        }))
    }

    fn write(&self, file: &Path) {
        let written = serde_json::to_vec(self)
            .map_err(std::io::Error::other)
            .and_then(|json| anlg_capture_core::fs::write_atomic(file, &json, true));
        if let Err(error) = written {
            tracing::warn!(%error, "capture screencast token");
        }
    }

    /// Whether the remembered screen is the one `selection` was drawn on;
    /// unknown is no.
    fn matches(&self, selection: &Selection) -> bool {
        selection.monitors <= 1 || area::on_monitor(selection, self.position)
    }
}

/// Forgets the screen the portal remembers, so the next recording asks.
pub fn forget_screen(token_file: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(token_file) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Waits for a new pipeline to write its first bytes to `out`: `Err` when
/// it exits, reports an error, `cancel` is set, or `within` passes first.
fn first_bytes(
    process: &mut dyn Process,
    out: &Path,
    within: Duration,
    cancel: &Cancel,
) -> Result<(), String> {
    let deadline = Instant::now() + within;
    loop {
        if process.try_wait().is_some() {
            return Err(process.errors());
        }
        let errors = process.errors();
        if errors.contains("ERROR") {
            return Err(errors);
        }
        if std::fs::metadata(out).is_ok_and(|metadata| metadata.len() > 0) {
            return Ok(());
        }
        if cancel.cancelled() {
            return Err("cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "nothing was written within {} s",
                within.as_secs_f32()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Recorder {
    /// The stream's size from one frame of it, when the portal gave none.
    fn probe_size(&self, session: &mut dyn CastSession, node: u32) -> Option<(u32, u32)> {
        let remote = session.open_remote().ok()?;
        let (read, write) = gst::pipe().ok()?;
        let args = gst::probe_args(remote.as_raw_fd(), node, write.as_raw_fd());
        let reader = std::thread::spawn(move || {
            use std::io::Read;
            let mut png = Vec::new();
            let _ = std::fs::File::from(read)
                .take(64 * 1024 * 1024)
                .read_to_end(&mut png);
            png
        });
        let ran = gst::run(
            self.parts.gst.as_ref(),
            &args,
            &[remote.as_raw_fd(), write.as_raw_fd()],
            PROBE_WITHIN,
        );
        drop(write);
        drop(remote);
        let png = reader.join().ok()?;
        if let Err(error) = ran {
            tracing::warn!(%error, "capture stream size probe");
        }
        gst::png_size(&png)
    }

    pub fn new(parts: Parts) -> Self {
        Self {
            parts: Arc::new(parts),
            phase: Arc::new(Mutex::new(Phase::Idle)),
            generation: AtomicU64::new(0),
            durations: Mutex::default(),
        }
    }

    pub fn duration_of(&self, movie: &Path) -> Option<u64> {
        self.durations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(movie)
            .copied()
    }

    /// The contract's `record_start`: blocks through the selector and the
    /// portal's dialog, and returns once frames are being written.
    pub fn start(&self, request: &str) -> Result<String, AnnotateError> {
        let request: StartRequest = super::parse(request)?;
        let cancel = Cancel::default();
        {
            let mut phase = lock(&self.phase);
            if !matches!(*phase, Phase::Idle) {
                return Err(fail(codes::BUSY, "a capture is already running"));
            }
            *phase = Phase::Selecting(cancel.clone());
        }
        let begun = self.begin(&request, &cancel);
        let mut phase = lock(&self.phase);
        let active = match begun {
            Ok(Begun::Started(active)) if !cancel.cancelled() => active,
            Ok(Begun::Started(active)) => {
                *phase = Phase::Idle;
                drop(phase);
                active.discard();
                return Ok(cancelled());
            }
            Ok(Begun::Cancelled) => {
                *phase = Phase::Idle;
                return Ok(cancelled());
            }
            Err(error) => {
                *phase = Phase::Idle;
                return Err(error);
            }
        };
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let reply = serde_json::json!({
            "status": "recording",
            "kind": request.mode,
            "display_id": 0,
            "rect": { "x": 0, "y": 0, "width": 0, "height": 0 },
            "width_px": active.size.0,
            "height_px": active.size.1,
            "scale": 1.0,
        })
        .to_string();
        *phase = Phase::Recording(Box::new(Active {
            generation,
            ..*active
        }));
        drop(phase);
        self.watch(generation);
        Ok(reply)
    }

    fn begin(&self, request: &StartRequest, cancel: &Cancel) -> Result<Begun, AnnotateError> {
        let parts = &self.parts;
        if !parts.gst.installed() {
            return Err(fail(codes::INTERNAL, gst::missing_launcher()));
        }
        let encoders = gst::encoders(|element| parts.gst.has(element))
            .map_err(|error| fail(codes::INTERNAL, error))?;
        if request.mode == "area" && !parts.gst.has("videocrop") {
            return Err(fail(
                codes::INTERNAL,
                format!(
                    "Recording an area needs the GStreamer element videocrop. {}",
                    gst::INSTALL_BASE
                ),
            ));
        }
        let mut warnings = Vec::new();
        if request.microphone || request.system_audio {
            warnings.push("Recordings on Linux have no sound yet: the microphone and system audio were not recorded.".to_owned());
        }
        if request.countdown_s > 0 {
            warnings.push(
                "There is no countdown on Linux yet: the recording started at once.".to_owned(),
            );
        }
        let mut source = if request.mode == "window" {
            Source::Window
        } else {
            Source::Monitor
        };
        let mut selection = None;
        if request.mode == "area" {
            match screenshot::grab(parts.shots.as_ref(), &parts.leftovers, false) {
                Ok(Some(frame)) => {
                    match parts
                        .picker
                        .pick(&frame, &request.labels.area_hint, cancel)?
                    {
                        Picked::Cancelled => return Ok(Begun::Cancelled),
                        Picked::Whole => {}
                        Picked::Window => source = Source::Window,
                        Picked::Area(chosen) => selection = Some(chosen),
                    }
                }
                Ok(None) => return Ok(Begun::Cancelled),
                Err(error) => warnings.push(format!(
                    "The area could not be chosen ({}), so the whole screen was recorded.",
                    error.message
                )),
            }
        }
        if cancel.cancelled() {
            return Ok(Begun::Cancelled);
        }
        let remembered = (source == Source::Monitor)
            .then(|| Remembered::read(&parts.token_file))
            .flatten();
        // A screen drawn on that is not the one remembered: ask again.
        let restore = remembered
            .filter(|remembered| {
                selection
                    .as_ref()
                    .is_none_or(|selection| remembered.matches(selection))
            })
            .map(|remembered| remembered.token);
        let cast = CastRequest {
            source,
            cursor: request.show_cursor,
            restore,
        };
        let Some(mut session) = parts.cast.start(&cast, cancel)? else {
            return Ok(Begun::Cancelled);
        };
        let stream = session.stream();
        if source == Source::Monitor
            && let Some(token) = session.restore_token()
        {
            Remembered {
                token,
                position: stream.position,
                size: stream.size,
            }
            .write(&parts.token_file);
        }
        let shape = match &selection {
            Some(selection) => {
                let (shape, warning) = area::crop(selection, &stream);
                warnings.extend(warning.map(str::to_owned));
                shape
            }
            None => Shape::Whole,
        };
        let size = match shape {
            Shape::Crop { rect, .. } => (rect.width, rect.height),
            Shape::Whole => match stream.size.filter(|(w, h)| *w > 0 && *h > 0) {
                Some((w, h)) => gst::even_size((w as u32, h as u32)),
                None => match self.probe_size(session.as_mut(), stream.node) {
                    Some(size) => gst::even_size(size),
                    None => {
                        warnings.push(format!(
                            "The screen's size could not be read, so the recording is {} × {}.",
                            gst::FALLBACK_SIZE.0,
                            gst::FALLBACK_SIZE.1
                        ));
                        gst::FALLBACK_SIZE
                    }
                },
            },
        };
        let mut failures = Vec::new();
        for encoder in encoders {
            if cancel.cancelled() {
                break;
            }
            let out = request.out_path.with_extension(encoder.extension());
            let file = match std::fs::File::create(&out) {
                Ok(file) => file,
                Err(error) => {
                    session.close();
                    return Err(fail(codes::IO, error));
                }
            };
            let remote = match session.open_remote() {
                Ok(remote) => remote,
                Err(error) => {
                    session.close();
                    let _ = std::fs::remove_file(&out);
                    return Err(error);
                }
            };
            let recording = Recording {
                remote: remote.as_raw_fd(),
                node: stream.node,
                fps: request.fps.clamp(1, 60),
                shape,
                encoder,
                out: file.as_raw_fd(),
                size,
            };
            let args = gst::record_args(&recording);
            // The movie's time starts with the pipeline that records it.
            let started = Instant::now();
            let launched = parts
                .gst
                .launch(&args, &[remote.as_raw_fd(), file.as_raw_fd()]);
            drop(file);
            drop(remote);
            let mut process = match launched {
                Ok(process) => process,
                Err(error) => {
                    failures.push(format!("{encoder:?}: {error}"));
                    let _ = std::fs::remove_file(&out);
                    continue;
                }
            };
            // Caps are negotiated on the first frame: recording once the
            // muxer wrote something, else the next encoder.
            if let Err(why) = first_bytes(process.as_mut(), &out, parts.probe, cancel) {
                process.kill();
                failures.push(format!("{encoder:?}: {why}"));
                let _ = std::fs::remove_file(&out);
                continue;
            }
            return Ok(Begun::Started(Box::new(Active {
                process,
                session,
                out,
                started,
                max: Duration::from_secs(u64::from(request.max_duration_s.max(1))),
                ended: None,
                size,
                warnings,
                generation: 0,
            })));
        }
        session.close();
        if cancel.cancelled() {
            return Ok(Begun::Cancelled);
        }
        Err(fail(
            codes::RECORDING,
            format!(
                "GStreamer could not record the screen: {}",
                failures.join("; ")
            ),
        ))
    }

    /// Watches recording `generation` until it is stopped or ends itself.
    fn watch(&self, generation: u64) {
        let phase = self.phase.clone();
        let events = self.parts.events.clone();
        let tick = self.parts.tick;
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(tick);
                let mut phase = lock(&phase);
                let Phase::Recording(active) = &mut *phase else {
                    return;
                };
                if active.generation != generation || active.ended.is_some() {
                    return;
                }
                let reason = if active.process.try_wait().is_some() {
                    Some("stream_error")
                } else if active.session.closed() {
                    // The desktop stopped sharing: finish what was written.
                    active.process.interrupt();
                    Some("stream_error")
                } else if active.started.elapsed() >= active.max {
                    active.process.interrupt();
                    Some("cap")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    active.ended = Some((reason, Instant::now()));
                    events.push(serde_json::json!({ "type": "record_stopped", "reason": reason }));
                    return;
                }
            }
        });
    }

    /// The contract's `record_stop`: finishes the movie.
    pub fn stop(&self) -> Result<String, AnnotateError> {
        let mut active = {
            let mut phase = lock(&self.phase);
            match std::mem::replace(&mut *phase, Phase::Finishing) {
                Phase::Recording(active) => active,
                other => {
                    *phase = other;
                    return Err(fail(codes::NOT_RECORDING, "no recording is running"));
                }
            }
        };
        let (ended_by, ended_at) = active.ended.unwrap_or(("stop", Instant::now()));
        let clean = gst::finish(active.process.as_mut(), STOP_WITHIN);
        active.session.close();
        let duration_ms = ended_at
            .saturating_duration_since(active.started)
            .as_millis() as u64;
        let bytes = std::fs::metadata(&active.out).map_or(0, |metadata| metadata.len());
        *lock(&self.phase) = Phase::Idle;
        if bytes == 0 {
            let _ = std::fs::remove_file(&active.out);
            return Err(fail(
                codes::RECORDING,
                format!("nothing was recorded: {}", active.process.errors()),
            ));
        }
        if clean != Some(true) && ended_by == "stop" {
            active
                .warnings
                .push("GStreamer did not finish the movie cleanly; its end may be missing.".into());
        }
        self.durations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(active.out.clone(), duration_ms);
        Ok(serde_json::json!({
            "video_path": active.out,
            "duration_ms": duration_ms,
            "width_px": active.size.0,
            "height_px": active.size.1,
            "ended_by": ended_by,
            "warnings": active.warnings,
        })
        .to_string())
    }

    /// The contract's `record_cancel`: stops and deletes the movie, or
    /// cancels the selector or the portal's dialog.
    pub fn cancel(&self) -> Result<(), AnnotateError> {
        let active = {
            let mut phase = lock(&self.phase);
            match std::mem::replace(&mut *phase, Phase::Finishing) {
                Phase::Recording(active) => active,
                Phase::Selecting(cancel) => {
                    cancel.cancel();
                    *phase = Phase::Selecting(cancel);
                    return Ok(());
                }
                other => {
                    *phase = other;
                    return Ok(());
                }
            }
        };
        active.discard();
        *lock(&self.phase) = Phase::Idle;
        Ok(())
    }

    pub fn status(&self) -> String {
        let (phase, elapsed) = match &*lock(&self.phase) {
            Phase::Idle => ("idle", 0),
            Phase::Selecting(_) => ("selecting", 0),
            Phase::Recording(active) => ("recording", active.started.elapsed().as_millis() as u64),
            Phase::Finishing => ("finishing", 0),
        };
        serde_json::json!({ "phase": phase, "elapsed_ms": elapsed }).to_string()
    }

    /// The stream of the recording that runs, as the portal reported it.
    pub fn stream(&self) -> Option<Stream> {
        match &*lock(&self.phase) {
            Phase::Recording(active) => Some(active.session.stream()),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::os::fd::RawFd;

    use super::*;
    use crate::linux::area::{Monitor, Region};
    use crate::linux::gst::Rect;
    use crate::linux::screenshot::tests::FakeShots;

    /// A pipeline that writes a byte into its output and then runs until
    /// stopped, or dies on cue.
    pub struct FakeProcess {
        pub state: Arc<Mutex<ProcessState>>,
    }

    #[derive(Default)]
    pub struct ProcessState {
        pub exited: Option<bool>,
        pub interrupted: bool,
        pub killed: bool,
        /// Ignores SIGINT.
        pub stubborn: bool,
    }

    impl Process for FakeProcess {
        fn try_wait(&mut self) -> Option<bool> {
            self.state.lock().unwrap().exited
        }
        fn interrupt(&mut self) {
            let mut state = self.state.lock().unwrap();
            state.interrupted = true;
            if !state.stubborn && state.exited.is_none() {
                state.exited = Some(true);
            }
        }
        fn kill(&mut self) {
            let mut state = self.state.lock().unwrap();
            if state.exited.is_none() {
                state.killed = true;
                state.exited = Some(false);
            }
        }
        fn errors(&self) -> String {
            "no element \"vah264enc\"".into()
        }
    }

    #[derive(Default)]
    pub struct FakeGst {
        pub elements: Vec<&'static str>,
        /// Encoders whose pipeline dies at once.
        pub broken: Vec<&'static str>,
        pub launched: Mutex<Vec<String>>,
        pub last: Mutex<Option<Arc<Mutex<ProcessState>>>>,
    }

    impl Gst for FakeGst {
        fn installed(&self) -> bool {
            !self.elements.is_empty()
        }
        fn has(&self, element: &str) -> bool {
            self.elements.contains(&element)
        }
        fn launch(&self, args: &[String], inherit: &[RawFd]) -> std::io::Result<Box<dyn Process>> {
            let line = args.join(" ");
            self.launched.lock().unwrap().push(line.clone());
            let out = inherit[1];
            // What the encoder writes before it is stopped.
            let written = unsafe { libc::write(out, b"x".as_ptr().cast(), 1) };
            assert_eq!(written, 1);
            let state = Arc::new(Mutex::new(ProcessState::default()));
            // Only recordings run here; decoders fail at once.
            if !line.contains("pipewiresrc")
                || self
                    .broken
                    .iter()
                    .any(|encoder| line.contains(&format!(" {encoder}")))
            {
                state.lock().unwrap().exited = Some(false);
            }
            *self.last.lock().unwrap() = Some(state.clone());
            Ok(Box::new(FakeProcess { state }))
        }
    }

    #[derive(Default)]
    pub struct CastState {
        pub requests: Vec<CastRequest>,
        pub closed_by_desktop: bool,
        pub closes: u32,
        /// PipeWire remotes opened.
        pub remotes: u32,
    }

    pub struct FakeCast {
        pub state: Arc<Mutex<CastState>>,
        /// Keeps the dialog up until cancelled.
        pub wait_for_cancel: bool,
        pub refuse: bool,
        pub stream: Stream,
    }

    struct FakeSession {
        state: Arc<Mutex<CastState>>,
        stream: Stream,
    }

    impl CastSession for FakeSession {
        fn open_remote(&mut self) -> Result<OwnedFd, AnnotateError> {
            self.state.lock().unwrap().remotes += 1;
            Ok(std::fs::File::open("/dev/null").unwrap().into())
        }
        fn stream(&self) -> Stream {
            self.stream
        }
        fn restore_token(&self) -> Option<String> {
            Some("token-1".into())
        }
        fn closed(&self) -> bool {
            self.state.lock().unwrap().closed_by_desktop
        }
        fn close(&mut self) {
            self.state.lock().unwrap().closes += 1;
        }
    }

    impl Cast for FakeCast {
        fn start(
            &self,
            request: &CastRequest,
            cancel: &Cancel,
        ) -> Result<Option<Box<dyn CastSession>>, AnnotateError> {
            self.state.lock().unwrap().requests.push(request.clone());
            if self.wait_for_cancel {
                while !cancel.cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                return Ok(None);
            }
            if self.refuse {
                return Ok(None);
            }
            Ok(Some(Box::new(FakeSession {
                state: self.state.clone(),
                stream: self.stream,
            })))
        }
    }

    pub struct FakePicker(pub Mutex<Option<Picked>>);

    impl AreaPicker for FakePicker {
        fn pick(
            &self,
            frame: &DynamicImage,
            _hint: &str,
            _cancel: &Cancel,
        ) -> Result<Picked, AnnotateError> {
            assert_eq!(frame.width(), 64);
            Ok(self.0.lock().unwrap().take().unwrap_or(Picked::Whole))
        }
    }

    pub struct Rig {
        pub dir: tempfile::TempDir,
        pub recorder: Recorder,
        pub gst: Arc<FakeGst>,
        pub cast: Arc<Mutex<CastState>>,
        pub events: Arc<Events>,
    }

    pub fn rig(gst: FakeGst, cast: FakeCast, picked: Option<Picked>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let gst = Arc::new(gst);
        let events = Arc::new(Events::default());
        let state = cast.state.clone();
        let recorder = Recorder::new(Parts {
            shots: Arc::new(FakeShots::new(dir.path())),
            leftovers: Leftovers::default(),
            cast: Arc::new(cast),
            gst: gst.clone(),
            picker: Arc::new(FakePicker(Mutex::new(picked))),
            events: events.clone(),
            token_file: dir.path().join("screencast-token"),
            probe: Duration::from_millis(10),
            tick: Duration::from_millis(5),
        });
        Rig {
            dir,
            recorder,
            gst,
            cast: state,
            events,
        }
    }

    pub fn h264() -> FakeGst {
        FakeGst {
            elements: [
                &gst::RECORD_ELEMENTS[..],
                &[
                    "videocrop",
                    "vah264enc",
                    "openh264enc",
                    "h264parse",
                    "mp4mux",
                    "vp8enc",
                    "webmmux",
                ],
            ]
            .concat(),
            ..FakeGst::default()
        }
    }

    pub fn cast() -> FakeCast {
        FakeCast {
            state: Arc::default(),
            wait_for_cancel: false,
            refuse: false,
            stream: Stream {
                node: 42,
                size: Some((32, 16)),
                position: Some((0, 0)),
            },
        }
    }

    fn start_json(rig: &Rig, mode: &str) -> String {
        serde_json::json!({
            "mode": mode, "out_path": rig.dir.path().join(".capture-x.mp4"), "fps": 30,
            "show_cursor": true, "microphone": true, "system_audio": false, "countdown_s": 3,
            "max_duration_s": 1800, "labels": { "area_hint": "Drag" },
        })
        .to_string()
    }

    fn value(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    fn wait_for(rig: &Rig, kind: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = rig
                .events
                .drain()
                .into_iter()
                .find(|event| event["type"] == kind)
            {
                return event;
            }
            assert!(Instant::now() < deadline, "no {kind}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn an_area_recording_crops_the_monitor_falls_back_past_a_broken_encoder_and_stops_cleanly() {
        let monitor = Monitor {
            x: 0,
            y: 0,
            width: 64,
            height: 32,
            scale: 1.0,
        };
        let picked = Picked::Area(Selection {
            region: Region {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 32,
                },
                trusted: true,
            },
            rect: Rect {
                x: 4,
                y: 2,
                width: 33,
                height: 20,
            },
            monitor,
            monitors: 1,
        });
        let rig = rig(
            FakeGst {
                broken: vec!["vah264enc"],
                ..h264()
            },
            FakeCast {
                stream: Stream {
                    node: 42,
                    size: Some((64, 32)),
                    position: None,
                },
                ..cast()
            },
            Some(picked),
        );
        let started = value(&rig.recorder.start(&start_json(&rig, "area")).unwrap());
        assert_eq!(started["status"], "recording");
        assert_eq!(
            (started["width_px"].as_u64(), started["height_px"].as_u64()),
            (Some(32), Some(20))
        );
        let launched = rig.gst.launched.lock().unwrap().clone();
        assert_eq!(launched.len(), 2);
        assert!(launched[0].contains(" vah264enc"));
        assert!(launched[1].contains("path=42"), "{}", launched[1]);
        assert!(
            launched[1].contains("width=64,height=32,pixel-aspect-ratio=1/1 ! videocrop left=4 top=2 right=28 bottom=10"),
            "{}",
            launched[1]
        );
        assert!(launched[1].contains(" openh264enc"));
        let request = rig.cast.lock().unwrap().requests[0].clone();
        assert_eq!(
            request,
            CastRequest {
                source: Source::Monitor,
                cursor: true,
                restore: None
            }
        );
        // Each encoder tried got its own PipeWire remote.
        assert_eq!(rig.cast.lock().unwrap().remotes, 2);
        assert_eq!(
            Remembered::read(&rig.dir.path().join("screencast-token")).unwrap(),
            Remembered {
                token: "token-1".into(),
                position: None,
                size: Some((64, 32)),
            }
        );
        assert_eq!(value(&rig.recorder.status())["phase"], "recording");
        // Busy while recording.
        assert_eq!(
            rig.recorder
                .start(&start_json(&rig, "area"))
                .unwrap_err()
                .code,
            codes::BUSY
        );

        let stopped = value(&rig.recorder.stop().unwrap());
        assert_eq!(stopped["ended_by"], "stop");
        let movie = PathBuf::from(stopped["video_path"].as_str().unwrap());
        assert!(movie.ends_with(".capture-x.mp4") && movie.is_file());
        let warnings = stopped["warnings"].to_string();
        assert!(
            warnings.contains("no sound") && warnings.contains("countdown"),
            "{warnings}"
        );
        let process = rig.gst.last.lock().unwrap().clone().unwrap();
        assert!(process.lock().unwrap().interrupted && !process.lock().unwrap().killed);
        assert_eq!(rig.cast.lock().unwrap().closes, 1);
        assert_eq!(value(&rig.recorder.status())["phase"], "idle");
        assert!(rig.recorder.duration_of(&movie).is_some());

        // The next screen recording reuses the portal's token.
        rig.recorder.start(&start_json(&rig, "fullscreen")).unwrap();
        assert_eq!(
            rig.cast.lock().unwrap().requests[1].restore.as_deref(),
            Some("token-1")
        );
        rig.recorder.cancel().unwrap();
        assert!(!rig.dir.path().join(".capture-x.mp4").exists());
    }

    #[test]
    fn a_recording_without_h264_is_webm_and_one_the_desktop_ends_is_finished() {
        let rig = rig(
            FakeGst {
                elements: [&gst::RECORD_ELEMENTS[..], &["vp9enc", "webmmux"]].concat(),
                ..FakeGst::default()
            },
            cast(),
            None,
        );
        rig.recorder.start(&start_json(&rig, "window")).unwrap();
        assert_eq!(rig.cast.lock().unwrap().requests[0].source, Source::Window);
        assert_eq!(rig.cast.lock().unwrap().requests[0].restore, None);
        // The desktop's own "stop sharing".
        rig.cast.lock().unwrap().closed_by_desktop = true;
        assert_eq!(wait_for(&rig, "record_stopped")["reason"], "stream_error");
        let stopped = value(&rig.recorder.stop().unwrap());
        assert_eq!(stopped["ended_by"], "stream_error");
        assert!(
            stopped["video_path"]
                .as_str()
                .unwrap()
                .ends_with(".capture-x.webm")
        );

        // The pipeline dying by itself ends it the same way.
        rig.recorder.start(&start_json(&rig, "window")).unwrap();
        rig.gst
            .last
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .lock()
            .unwrap()
            .exited = Some(false);
        assert_eq!(wait_for(&rig, "record_stopped")["reason"], "stream_error");
        assert_eq!(
            value(&rig.recorder.stop().unwrap())["ended_by"],
            "stream_error"
        );
    }

    #[test]
    fn the_length_limit_stops_a_recording_and_a_stubborn_pipeline_is_killed() {
        let rig = rig(h264(), cast(), None);
        let json = start_json(&rig, "fullscreen")
            .replace("\"max_duration_s\":1800", "\"max_duration_s\":1");
        rig.recorder.start(&json).unwrap();
        assert_eq!(wait_for(&rig, "record_stopped")["reason"], "cap");
        assert_eq!(value(&rig.recorder.stop().unwrap())["ended_by"], "cap");

        rig.recorder.start(&start_json(&rig, "fullscreen")).unwrap();
        let process = rig.gst.last.lock().unwrap().clone().unwrap();
        process.lock().unwrap().stubborn = true;
        // Cancelled while recording: stopped (killed, as it ignores SIGINT)
        // and deleted.
        rig.recorder.cancel().unwrap();
        assert!(process.lock().unwrap().killed);
        assert!(!rig.dir.path().join(".capture-x.mp4").exists());
        assert_eq!(value(&rig.recorder.status())["phase"], "idle");
    }

    #[test]
    fn cancelling_the_portals_dialog_or_the_selector_records_nothing() {
        let rig = Arc::new(rig(
            h264(),
            FakeCast {
                wait_for_cancel: true,
                ..cast()
            },
            None,
        ));
        let starting = {
            let rig = rig.clone();
            std::thread::spawn(move || rig.recorder.start(&start_json(&rig, "fullscreen")))
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while value(&rig.recorder.status())["phase"] != "selecting"
            || rig.cast.lock().unwrap().requests.is_empty()
        {
            assert!(Instant::now() < deadline, "the dialog never came up");
            std::thread::sleep(Duration::from_millis(5));
        }
        rig.recorder.cancel().unwrap();
        assert_eq!(starting.join().unwrap().unwrap(), cancelled());
        assert_eq!(value(&rig.recorder.status())["phase"], "idle");
        assert!(rig.gst.launched.lock().unwrap().is_empty());

        let rig = rig_with_picker(Picked::Cancelled);
        assert_eq!(
            rig.recorder.start(&start_json(&rig, "area")).unwrap(),
            cancelled()
        );
        assert!(rig.cast.lock().unwrap().requests.is_empty());
        let rig = rig_with_picker(Picked::Window);
        rig.recorder.start(&start_json(&rig, "area")).unwrap();
        assert_eq!(rig.cast.lock().unwrap().requests[0].source, Source::Window);
        rig.recorder.cancel().unwrap();
    }

    fn rig_with_picker(picked: Picked) -> Rig {
        rig(h264(), cast(), Some(picked))
    }

    #[test]
    fn without_gstreamer_a_recording_fails_before_the_portal_with_what_to_install() {
        let rig = rig(FakeGst::default(), cast(), None);
        let error = rig
            .recorder
            .start(&start_json(&rig, "fullscreen"))
            .unwrap_err();
        assert!(error.message.contains("gstreamer1"), "{error}");
        assert!(rig.cast.lock().unwrap().requests.is_empty());
        assert_eq!(value(&rig.recorder.status())["phase"], "idle");
    }

    #[test]
    fn the_remembered_screen_is_reused_only_for_an_area_drawn_on_it_and_can_be_forgotten() {
        let left = Monitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        };
        let right = Monitor { x: 1920, ..left };
        let on = |monitor: Monitor| Selection {
            region: Region {
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                trusted: true,
            },
            rect: Rect {
                x: 10,
                y: 10,
                width: 100,
                height: 100,
            },
            monitor,
            monitors: 2,
        };
        let remembered = Remembered {
            token: "t".into(),
            position: Some((1920, 0)),
            size: Some((1920, 1080)),
        };
        assert!(remembered.matches(&on(right)));
        assert!(!remembered.matches(&on(left)));
        let unknown = Remembered {
            position: None,
            ..remembered
        };
        assert!(!unknown.matches(&on(right)));

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("screencast-token");
        // A token kept before its screen was.
        std::fs::write(&file, "old-token\n").unwrap();
        assert_eq!(Remembered::read(&file).unwrap().token, "old-token");
        forget_screen(&file).unwrap();
        assert!(Remembered::read(&file).is_none());
        forget_screen(&file).unwrap();
    }

    #[test]
    fn an_area_recording_without_videocrop_says_what_to_install_before_the_portal() {
        let rig = rig(
            FakeGst {
                elements: h264()
                    .elements
                    .into_iter()
                    .filter(|e| *e != "videocrop")
                    .collect(),
                ..FakeGst::default()
            },
            cast(),
            None,
        );
        let error = rig.recorder.start(&start_json(&rig, "area")).unwrap_err();
        assert!(error.message.contains("videocrop"), "{error}");
        assert!(rig.cast.lock().unwrap().requests.is_empty());
    }
}
