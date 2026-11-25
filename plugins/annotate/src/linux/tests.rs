use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::windows::{Monitor, Opened};
use super::*;

#[derive(Default)]
struct FakeScreen {
    dir: Mutex<Option<PathBuf>>,
    /// What `start` answers instead of recording.
    start_reply: Mutex<Option<Result<String, AnnotateError>>>,
    duration_ms: AtomicU64,
    ended_by: Mutex<String>,
    events: Mutex<Vec<Value>>,
    cancels: AtomicUsize,
    stream: Mutex<Option<Stream>>,
}

impl Screen for FakeScreen {
    fn start(&self, request: &str) -> Result<String, AnnotateError> {
        let request: Value = serde_json::from_str(request).unwrap();
        assert_eq!(request["mode"], "fullscreen");
        assert_eq!(request["microphone"], false);
        assert_eq!(request["max_duration_s"], 1200);
        let out = PathBuf::from(request["out_path"].as_str().unwrap());
        *self.dir.lock().unwrap() = out.parent().map(Path::to_path_buf);
        if let Some(reply) = self.start_reply.lock().unwrap().take() {
            return reply;
        }
        std::fs::write(&out, b"movie").unwrap();
        Ok(json!({ "status": "recording", "width_px": 1920, "height_px": 1080 }).to_string())
    }
    fn stop(&self) -> Result<String, AnnotateError> {
        let dir = self.dir.lock().unwrap().clone().unwrap();
        let ended_by = self.ended_by.lock().unwrap().clone();
        Ok(json!({
            "video_path": dir.join("recording.mp4"),
            "duration_ms": self.duration_ms.load(Ordering::SeqCst),
            "ended_by": if ended_by.is_empty() { "stop".to_owned() } else { ended_by },
            "warnings": [],
        })
        .to_string())
    }
    fn cancel(&self) -> Result<(), AnnotateError> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn elapsed_ms(&self) -> u64 {
        200
    }
    fn stream(&self) -> Option<Stream> {
        *self.stream.lock().unwrap()
    }
    fn events(&self) -> Vec<Value> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

/// A movie of `frames` frames, frame k a flat grey of `k`, at any size.
struct FakeDecoder {
    frames: usize,
    size: (u32, u32),
    asked: Mutex<Vec<(u32, u32)>>,
}

impl Decoder for FakeDecoder {
    fn size(&self, _video: &Path) -> Result<(u32, u32), String> {
        Ok(self.size)
    }
    fn frames(
        &self,
        _video: &Path,
        size: (u32, u32),
        _within: Duration,
        frame: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<usize, String> {
        self.asked.lock().unwrap().push(size);
        let len = size.0 as usize * size.1 as usize * 4;
        for k in 0..self.frames {
            let mut rgba = vec![k as u8; len];
            for alpha in rgba.iter_mut().skip(3).step_by(4) {
                *alpha = 255;
            }
            if !frame(&rgba) {
                return Ok(k + 1);
            }
        }
        Ok(self.frames)
    }
}

#[derive(Default)]
struct FakeSurfaces {
    monitors: Vec<Monitor>,
    fail_ink: bool,
    /// Taking (`true`) or giving back (`false`) the pointer fails.
    fail_taking: Option<bool>,
    opened: AtomicUsize,
    taken: Mutex<Vec<bool>>,
    closed: AtomicUsize,
    ink_closed: AtomicUsize,
    published: Mutex<Vec<AnnotateSurface>>,
    scrolled: Mutex<Vec<u8>>,
}

impl Surfaces for FakeSurfaces {
    fn monitors(&self) -> Vec<Monitor> {
        self.monitors.clone()
    }
    fn open(&self, _monitor: &Monitor) -> Opened {
        self.opened.fetch_add(1, Ordering::SeqCst);
        Opened {
            ink: if self.fail_ink {
                Err("no input shape".into())
            } else {
                Ok(())
            },
            capsule: Ok(()),
        }
    }
    fn loaded(&self, _label: &str) {}
    fn ink_origin(&self) -> (f64, f64) {
        (0.0, 10.0)
    }
    fn takes_pointer(&self, takes: bool) -> Result<(), String> {
        self.taken.lock().unwrap().push(takes);
        if self.fail_taking == Some(takes) {
            return Err("X error".into());
        }
        Ok(())
    }
    fn publish(&self, surface: &AnnotateSurface) {
        self.published.lock().unwrap().push(surface.clone());
    }
    fn scroll(&self, button: u8) -> Result<(), String> {
        self.scrolled.lock().unwrap().push(button);
        Ok(())
    }
    fn close_ink(&self) {
        self.ink_closed.fetch_add(1, Ordering::SeqCst);
    }
    fn close(&self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct FakeClock(AtomicU64);

impl FakeClock {
    fn at_ms(&self, ms: u64) {
        self.0
            .store(1_000_000_000 + ms * 1_000_000, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn monitor() -> Monitor {
    Monitor {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
        work: (0, 32, 1920, 1048),
    }
}

struct Rig {
    dir: tempfile::TempDir,
    linux: Linux,
    screen: Arc<FakeScreen>,
    surfaces: Arc<FakeSurfaces>,
    clock: Arc<FakeClock>,
}

fn rig_with(surfaces: FakeSurfaces, placeable: bool, frames: usize) -> Rig {
    let screen = Arc::new(FakeScreen::default());
    let surfaces = Arc::new(surfaces);
    let clock = Arc::new(FakeClock::default());
    // The recording's zero is 200 ms before the start returns.
    clock.at_ms(200);
    let linux = Linux::new(
        screen.clone(),
        Arc::new(FakeDecoder {
            frames,
            size: (64, 36),
            asked: Mutex::default(),
        }),
        surfaces.clone(),
        clock.clone(),
        placeable,
        PathBuf::from("/nonexistent/screencast-token"),
    );
    Rig {
        dir: tempfile::tempdir().unwrap(),
        linux,
        screen,
        surfaces,
        clock,
    }
}

fn rig() -> Rig {
    rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            ..FakeSurfaces::default()
        },
        true,
        8,
    )
}

fn start(rig: &Rig) -> Value {
    let request = json!({ "dir": rig.dir.path(), "labels": { "keep": "Manter" } });
    serde_json::from_str(&rig.linux.start(&request.to_string()).unwrap()).unwrap()
}

fn events(rig: &Rig) -> Vec<String> {
    let events: Vec<Value> = serde_json::from_str(&rig.linux.poll_events().unwrap()).unwrap();
    events
        .iter()
        .map(|event| match event["reason"].as_str() {
            Some(reason) => format!("{}:{reason}", event["type"].as_str().unwrap()),
            None => event["type"].as_str().unwrap().to_owned(),
        })
        .collect()
}

fn last_stage(rig: &Rig) -> CapsuleStage {
    rig.surfaces.published.lock().unwrap().last().unwrap().stage
}

fn stop(rig: &Rig) -> Value {
    serde_json::from_str(&rig.linux.stop().unwrap()).unwrap()
}

#[test]
fn a_recording_draws_on_the_monitor_shared_and_ends_with_its_strokes_and_frames() {
    let rig = rig();
    let started = start(&rig);
    assert_eq!(started["started_host_ns"], 1_000_000_000_u64);
    assert_eq!(started["display"]["width_pt"], 1920.0);
    assert_eq!(rig.surfaces.opened.load(Ordering::SeqCst), 1);
    let surface = rig.linux.surface().unwrap();
    assert_eq!(surface.ink, InkMode::Off);
    assert_eq!(surface.labels.keep.as_deref(), Some("Manter"));

    // Ink is off: the pointer belongs to the apps underneath.
    rig.linux.pointer("move", 5.0, 5.0);
    rig.linux.set_ink_mode("hover").unwrap();
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true]);
    for (ms, x) in [(500, 100.0), (520, 120.0), (540, 140.0)] {
        rig.clock.at_ms(ms);
        rig.linux.pointer("move", x, 50.0);
    }
    rig.linux.set_ink_mode("off").unwrap();
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);

    rig.screen.duration_ms.store(2_000, Ordering::SeqCst);
    let stopped = stop(&rig);
    // The overlay goes; the capsule stays to show the annotation being prepared.
    assert_eq!(rig.surfaces.ink_closed.load(Ordering::SeqCst), 1);
    assert_eq!(last_stage(&rig), CapsuleStage::Processing);
    assert!(rig.linux.surface().is_none());
    assert_eq!(stopped["ended_by"], "stop");
    assert_eq!(stopped["duration_ms"], 2_000);
    let points = stopped["strokes"][0]["points"].as_array().unwrap();
    assert_eq!(points.len(), 3);
    // In the monitor's points: the overlay sits 10 pt below its top.
    assert_eq!(
        (points[0]["x"].as_f64(), points[0]["y"].as_f64()),
        (Some(100.0), Some(60.0))
    );
    assert_eq!(points[0]["t_ms"], 500);
    // Eight frames of a 2 s movie: frame 0 at 0 ms.
    assert_eq!(stopped["video_offset_ms"], 0);
    let candidates = stopped["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 8);
    assert!(
        candidates
            .iter()
            .all(|c| Path::new(c["path"].as_str().unwrap()).is_file())
    );
    assert!(rig.linux.stop().is_err());
}

#[test]
fn candidates_taken_while_ink_showed_show_the_screen_from_before_the_ink() {
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            ..FakeSurfaces::default()
        },
        true,
        16,
    );
    start(&rig);
    rig.linux.press("pen");
    // A dot at 1.2 s: its ink can show from 0.9 s to 2.9 s.
    rig.clock.at_ms(1_200);
    rig.linux.pointer("down", 300.0, 300.0);
    rig.linux.pointer("up", 300.0, 300.0);
    rig.linux.press("pen");
    rig.screen.duration_ms.store(4_000, Ordering::SeqCst);
    let stopped = stop(&rig);
    let candidates = stopped["candidates"].as_array().unwrap();
    let grey = |t: i64| {
        let candidate = candidates.iter().find(|c| c["t_ms"] == t).unwrap();
        let image = image::open(candidate["path"].as_str().unwrap()).unwrap();
        image.to_luma8().get_pixel(10, 10).0[0]
    };
    // Frame k shows grey k, a frame every 250 ms from 0.
    assert!(grey(500).abs_diff(2) <= 1);
    for t in [750, 1_000, 2_000, 2_750] {
        assert!(grey(t).abs_diff(3) <= 1, "{t} ms");
    }
    assert!(grey(3_000).abs_diff(12) <= 1);

    // Key frames come from the same moments, at full size.
    let request = json!({
        "video": rig.dir.path().join("recording.mp4"), "video_offset_ms": 0,
        "times_ms": [1_000, 3_000], "dir": rig.dir.path().join(".extract"),
    });
    let reply: Value =
        serde_json::from_str(&rig.linux.extract_frames(&request.to_string()).unwrap()).unwrap();
    let frames = reply["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(
        (frames[0]["t_ms"].as_i64(), frames[0]["video_ms"].as_i64()),
        (Some(1_000), Some(750))
    );
    assert_eq!(frames[1]["video_ms"], 3_000);
    let png = image::open(frames[0]["path"].as_str().unwrap())
        .unwrap()
        .to_luma8();
    assert_eq!(png.dimensions(), (64, 36));
    assert_eq!(png.get_pixel(0, 0).0[0], 3);
}

#[test]
fn hover_ink_ends_without_word_from_the_shortcut_or_on_a_click_and_the_pen_after_idling() {
    let rig = rig();
    start(&rig);
    // A desktop that never reports the shortcut's release.
    rig.linux.set_ink_mode("hover").unwrap();
    for ms in (300..8_200).step_by(100) {
        rig.clock.at_ms(ms);
        rig.linux.pointer("move", ms as f64 / 10.0, 10.0);
    }
    assert!(events(&rig).is_empty());
    rig.clock.at_ms(200 + 8_000);
    assert_eq!(events(&rig), ["ink_off:idle"]);
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);

    // Held again: a click gives the pointer back.
    rig.linux.set_ink_mode("hover").unwrap();
    rig.linux.pointer("down", 10.0, 10.0);
    assert_eq!(events(&rig), ["ink_off:escape"]);

    // The pen idles 30 s after its last stroke; moving without drawing
    // doesn't count.
    rig.clock.at_ms(10_000);
    rig.linux.press("pen");
    rig.clock.at_ms(11_000);
    rig.linux.pointer("down", 10.0, 10.0);
    rig.linux.pointer("up", 20.0, 10.0);
    rig.clock.at_ms(40_000);
    rig.linux.pointer("move", 50.0, 50.0);
    assert_eq!(events(&rig), ["pen_on"]);
    rig.clock.at_ms(41_000);
    assert_eq!(events(&rig), ["ink_off:idle"]);

    rig.linux.press("pen");
    rig.linux.escape();
    rig.linux.press("pen");
    rig.linux.press("pen");
    assert_eq!(
        events(&rig),
        ["pen_on", "ink_off:escape", "pen_on", "pen_off"]
    );
    assert_eq!(rig.surfaces.taken.lock().unwrap().len(), 10);
}

#[test]
fn the_capsule_stops_and_discards_only_once_asked_and_confirmed() {
    let rig = rig();
    start(&rig);
    rig.linux.press("confirm_discard");
    rig.linux.press("discard");
    assert!(rig.linux.surface().unwrap().asking);
    rig.clock.at_ms(200 + 4_100);
    assert!(events(&rig).is_empty());
    assert!(!rig.linux.surface().unwrap().asking);
    rig.linux.press("discard");
    rig.linux.press("confirm_discard");
    rig.linux.press("stop");
    rig.linux.press("stop");
    assert_eq!(events(&rig), ["discard", "stop"]);
}

#[test]
fn an_overlay_that_cannot_ignore_the_pointer_is_never_used_and_the_recording_goes_on() {
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            fail_ink: true,
            ..FakeSurfaces::default()
        },
        true,
        4,
    );
    start(&rig);
    rig.linux.set_ink_mode("hover").unwrap();
    rig.linux.pointer("move", 10.0, 10.0);
    rig.linux.pointer("move", 50.0, 10.0);
    assert!(rig.surfaces.taken.lock().unwrap().is_empty());
    // The capsule's pen doesn't come on with nothing to draw on.
    rig.linux.set_ink_mode("off").unwrap();
    rig.linux.press("pen");
    let surface = rig.linux.surface().unwrap();
    assert!(!surface.drawable && !surface.pen_on);
    let stopped = stop(&rig);
    assert_eq!(stopped["strokes"].as_array().unwrap().len(), 0);
    assert!(stopped["warnings"][0].as_str().unwrap().contains("no ink"));
}

#[test]
fn an_overlay_that_cannot_give_the_pointer_back_is_closed_at_once() {
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            fail_taking: Some(false),
            ..FakeSurfaces::default()
        },
        true,
        4,
    );
    start(&rig);
    rig.linux.set_ink_mode("hover").unwrap();
    rig.linux.set_ink_mode("off").unwrap();
    assert_eq!(rig.surfaces.ink_closed.load(Ordering::SeqCst), 1);
    // Gone for good: ink can't come back.
    rig.linux.set_ink_mode("hover").unwrap();
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);
    let stopped = stop(&rig);
    assert!(
        stopped["warnings"]
            .to_string()
            .contains("stopped responding")
    );
}

#[test]
fn an_overlay_that_cannot_take_the_pointer_leaves_ink_off() {
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            fail_taking: Some(true),
            ..FakeSurfaces::default()
        },
        true,
        4,
    );
    start(&rig);
    rig.linux.press("pen");
    assert_eq!(rig.linux.surface().unwrap().ink, InkMode::Off);
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);
    assert_eq!(events(&rig), ["pen_on", "ink_off:escape"]);
    assert_eq!(rig.surfaces.ink_closed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_stream_the_desktop_ends_closes_the_windows_at_once_and_waits_for_stop() {
    let rig = rig();
    start(&rig);
    rig.linux.set_ink_mode("hover").unwrap();
    let closed = rig.surfaces.closed.load(Ordering::SeqCst);
    rig.screen
        .events
        .lock()
        .unwrap()
        .push(json!({ "type": "record_stopped", "reason": "stream_error" }));
    assert_eq!(events(&rig), ["stopped:stream_error"]);
    assert_eq!(rig.surfaces.closed.load(Ordering::SeqCst), closed);
    assert_eq!(rig.surfaces.ink_closed.load(Ordering::SeqCst), 1);
    assert_eq!(last_stage(&rig), CapsuleStage::Processing);
    assert!(rig.linux.surface().is_none());
    assert!(rig.linux.set_ink_mode("hover").is_err());
    rig.linux.press("pen");
    assert!(events(&rig).is_empty());
    *rig.screen.ended_by.lock().unwrap() = "stream_error".into();
    assert_eq!(stop(&rig)["ended_by"], "stream_error");

    // The length limit.
    std::fs::remove_file(rig.dir.path().join("recording.mp4")).unwrap();
    start(&rig);
    rig.screen
        .events
        .lock()
        .unwrap()
        .push(json!({ "type": "record_stopped", "reason": "cap" }));
    assert_eq!(events(&rig), ["stopped:max_duration"]);
    *rig.screen.ended_by.lock().unwrap() = "cap".into();
    assert_eq!(stop(&rig)["ended_by"], "max_duration");
}

#[test]
fn a_cancelled_dialog_or_a_cancel_while_starting_records_nothing_and_says_cancelled() {
    let rig = rig();
    *rig.screen.start_reply.lock().unwrap() = Some(Ok(r#"{"status":"cancelled"}"#.into()));
    let request = json!({ "dir": rig.dir.path() }).to_string();
    let error = rig.linux.start(&request).unwrap_err();
    assert!(matches!(crate::Error::from(error), crate::Error::Cancelled));
    assert_eq!(rig.surfaces.opened.load(Ordering::SeqCst), 0);
    assert!(rig.linux.surface().is_none());

    // A discard while the desktop's dialog is up takes the dialog down, and
    // a recording that started anyway is cancelled.
    *rig.linux.lock() = State::Starting { cancelled: false };
    rig.linux.cancel().unwrap();
    assert_eq!(rig.screen.cancels.load(Ordering::SeqCst), 1);
    let error = rig.linux.begin(&parse(&request).unwrap()).unwrap_err();
    assert_eq!(error.code, CANCELLED);
    assert_eq!(rig.screen.cancels.load(Ordering::SeqCst), 2);
    assert!(rig.linux.surface().is_none());
}

#[test]
fn without_xwayland_or_a_known_monitor_it_records_without_windows() {
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor()],
            ..FakeSurfaces::default()
        },
        false,
        4,
    );
    start(&rig);
    assert_eq!(rig.surfaces.opened.load(Ordering::SeqCst), 0);
    assert!(
        stop(&rig)["warnings"][0]
            .as_str()
            .unwrap()
            .contains("native Wayland")
    );

    let right = Monitor {
        x: 1920,
        ..monitor()
    };
    let rig = rig_with(
        FakeSurfaces {
            monitors: vec![monitor(), right],
            ..FakeSurfaces::default()
        },
        true,
        4,
    );
    start(&rig);
    assert_eq!(rig.surfaces.opened.load(Ordering::SeqCst), 0);
    let stopped = stop(&rig);
    assert!(
        stopped["warnings"][0]
            .as_str()
            .unwrap()
            .contains("which screen")
    );
    // What the portal said instead of a monitor.
    assert_eq!(stopped["display"]["width_pt"], 1920.0);
}

#[test]
fn the_shared_stream_is_matched_to_its_monitor() {
    let left = monitor();
    let right = Monitor {
        x: 1920,
        ..monitor()
    };
    let hidpi = Monitor {
        x: 3840,
        width: 2880,
        height: 1800,
        scale: 2.0,
        ..monitor()
    };
    let monitors = [left, right, hidpi];
    let stream = |position: Option<(i32, i32)>, size: Option<(i32, i32)>| Stream {
        node: 1,
        position,
        size,
    };
    assert_eq!(
        windows::pick(
            Some(&stream(Some((1920, 0)), Some((1920, 1080)))),
            &monitors
        ),
        Some(1)
    );
    // Logical: the HiDPI one at 3840 physical is at 1920 logical too, so
    // its size tells.
    assert_eq!(
        windows::pick(Some(&stream(Some((1920, 0)), Some((1440, 900)))), &monitors),
        Some(2)
    );
    assert_eq!(
        windows::pick(Some(&stream(None, Some((1440, 900)))), &monitors),
        Some(2)
    );
    assert_eq!(
        windows::pick(Some(&stream(None, Some((1920, 1080)))), &monitors),
        None
    );
    assert_eq!(windows::pick(None, &monitors), None);
    assert_eq!(windows::pick(None, &[left]), Some(0));
}

#[test]
fn strokes_are_clamped_thinned_and_hover_strokes_that_never_moved_are_dropped() {
    let mut strokes = ink::Strokes::new((100.0, 50.0));
    strokes.begin(InkMode::Hover, 10.0, 10.0, 5);
    strokes.end();
    strokes.begin(InkMode::Drag, 10.0, 10.0, 7);
    strokes.end();
    strokes.begin(InkMode::Hover, -5.0, 80.0, 10);
    // Clamped onto the last point: dropped.
    strokes.add(-5.2, 80.0, 12);
    strokes.add(200.0, f64::NAN, 9);
    let strokes = strokes.finish();
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[0].points, [(10.0, 10.0, 7)]);
    assert_eq!(strokes[1].points, [(0.0, 50.0, 10), (100.0, 0.0, 10)]);
}

#[test]
fn ink_episodes_widen_strokes_merge_and_frames_stand_in_from_before_else_after() {
    let episodes = frames::episodes(&[(5_000, 5_200), (1_000, 1_500)]);
    assert_eq!(episodes, [(700, 3_200), (4_700, 6_900)]);
    assert_eq!(
        frames::episodes(&[(5_000, 5_200), (1_000, 1_500), (3_400, 3_600)]),
        [(700, 6_900)]
    );
    // Frame 8 (2 s) shows ink: frame 2 (500 ms) is from before it.
    assert_eq!(frames::shown(8, &episodes, 40), 2);
    assert_eq!(frames::shown(16, &episodes, 40), 16);
    // However long the ink lasted.
    let long = frames::episodes(&[(1_000, 60_000)]);
    assert_eq!(frames::shown(200, &long, 400), 2);
    // Ink from the very start: the first frame after it, if there is one.
    let first = frames::episodes(&[(100, 500)]);
    assert_eq!(frames::shown(2, &first, 20), 9);
    assert_eq!(frames::shown(2, &first, 9), 2);
    assert_eq!(frames::fit((2880, 1800), 480), (480, 300));
    assert_eq!(frames::fit((641, 401), 2560), (640, 400));
}

#[derive(Default)]
struct FakeServer {
    passes: Mutex<Option<bool>>,
    released: AtomicUsize,
    unmapped: AtomicUsize,
}

impl x11::Shape for FakeServer {
    fn passes_pointer(&self, _xid: u32) -> Result<bool, String> {
        (*self.passes.lock().unwrap()).ok_or_else(|| "no X server".into())
    }
    fn release(&self, _xid: u32) -> Result<(), String> {
        self.released.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn unmap(&self, _xid: u32) -> Result<(), String> {
        self.unmapped.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn scroll(&self, _button: u8) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn the_pointer_is_let_go_through_x_whenever_gtk_does_not_confirm_it() {
    let server = FakeServer::default();
    *server.passes.lock().unwrap() = Some(true);
    assert!(x11::settle(Some(Ok(())), false, 7, &server).is_ok());
    assert!(x11::settle(Some(Ok(())), true, 7, &server).is_err());
    assert_eq!(server.released.load(Ordering::SeqCst), 0);
    // A hung main thread, a GTK error, or a shape that didn't change.
    assert!(x11::settle(None, false, 7, &server).is_err());
    assert!(x11::settle(Some(Err("gone".into())), false, 7, &server).is_err());
    *server.passes.lock().unwrap() = Some(false);
    assert!(x11::settle(Some(Ok(())), false, 7, &server).is_err());
    assert_eq!(server.released.load(Ordering::SeqCst), 3);
    // Taking it never forces anything.
    assert!(x11::settle(None, true, 7, &server).is_err());
    assert_eq!(server.released.load(Ordering::SeqCst), 3);

    x11::closed(Some(()), Some(7), &server);
    x11::closed(None, None, &server);
    assert_eq!(server.unmapped.load(Ordering::SeqCst), 0);
    x11::closed(None, Some(7), &server);
    assert_eq!(server.unmapped.load(Ordering::SeqCst), 1);
}

#[test]
fn a_stopped_capsule_goes_by_itself_but_never_takes_the_next_recordings_windows() {
    let mut rig = rig();
    let short = Duration::from_millis(50);
    rig.linux.lingers = Lingers {
        ready: short,
        failed: short,
        processing: Duration::from_millis(400),
    };
    let margin = Duration::from_secs(1);
    let closed = || rig.surfaces.closed.load(Ordering::SeqCst);
    start(&rig);
    stop(&rig);
    let before = closed();
    rig.linux.capsule("ready").unwrap();
    assert_eq!(last_stage(&rig), CapsuleStage::Ready);
    std::thread::sleep(short + margin);
    // Ready closed it once; the processing limit that was pending did not.
    assert_eq!(closed(), before + 1);

    // A failure, then a new recording before the capsule goes.
    std::fs::remove_file(rig.dir.path().join("recording.mp4")).unwrap();
    start(&rig);
    stop(&rig);
    rig.linux.capsule("failed").unwrap();
    assert_eq!(last_stage(&rig), CapsuleStage::Failed);
    std::fs::remove_file(rig.dir.path().join("recording.mp4")).unwrap();
    start(&rig);
    let before = closed();
    std::thread::sleep(Duration::from_millis(400) + margin);
    assert_eq!(closed(), before);
    assert!(rig.linux.surface().is_some());

    // Nobody says how it came out: the processing limit closes it.
    stop(&rig);
    let before = closed();
    std::thread::sleep(Duration::from_millis(400) + margin);
    assert_eq!(closed(), before + 1);
}

#[test]
fn a_scroll_on_the_pen_goes_through_to_the_app_and_the_pen_takes_the_pointer_back_after() {
    let rig = rig();
    start(&rig);
    // Ink off: the overlay never sees the wheel, and asks nothing.
    rig.linux.scroll(0.0, 1.0);
    rig.linux.press("pen");
    rig.clock.at_ms(1_000);
    // A part click lets the pointer through without replaying anything; a
    // burst replays at most three clicks.
    rig.linux.scroll(0.0, 0.0);
    rig.linux.scroll(0.0, 40.0);
    rig.linux.scroll(0.0, -1.0);
    assert_eq!(*rig.surfaces.scrolled.lock().unwrap(), [5, 5, 5, 4]);
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);
    rig.clock.at_ms(1_500);
    events(&rig);
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false]);
    rig.clock.at_ms(1_700);
    events(&rig);
    assert_eq!(*rig.surfaces.taken.lock().unwrap(), [true, false, true]);
    assert_eq!(rig.linux.surface().unwrap().ink, InkMode::Drag);

    // A stroke being drawn keeps the pointer.
    rig.linux.pointer("down", 10.0, 10.0);
    rig.linux.scroll(1.0, 0.0);
    assert_eq!(rig.surfaces.scrolled.lock().unwrap().len(), 4);
    assert_eq!(x11::wheel_button(-30.0, 2.0), Some(6));
    assert_eq!(x11::wheel_button(0.0, 0.0), None);
}
