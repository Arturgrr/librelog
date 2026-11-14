//! The plugin's session on top of the Linux layer, with fake portals,
//! GStreamer and windows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anlg_capture_core::Layout;

use super::clipboard::Clipboard;
use super::record::tests::{FakeGst, cast, h264};
use super::record::{Parts, Recorder};
use super::screenshot::tests::FakeShots;
use super::*;
use crate::session::Session;
use crate::{
    CaptureEvent, CaptureKind, CaptureLabels, CaptureMode, CaptureOutcome, ImageFormat,
    RecordingOutput, RecordingRequest, RecordingStart, ScreenshotRequest,
};

#[derive(Default)]
struct FakeClipboard(Mutex<Vec<Vec<String>>>);

impl Clipboard for FakeClipboard {
    fn set(&self, formats: Vec<(String, Vec<u8>)>) -> Result<(), AnnotateError> {
        self.0
            .lock()
            .unwrap()
            .push(formats.into_iter().map(|(mime, _)| mime).collect());
        Ok(())
    }
}

struct NoView;

impl cards::CardView for NoView {
    fn show(&self, _: &CaptureCards) {}
}

struct Setup {
    dir: tempfile::TempDir,
    session: Arc<Session>,
    cards: Arc<cards::Cards>,
    clipboard: Arc<FakeClipboard>,
    events: Arc<Mutex<Vec<CaptureEvent>>>,
}

fn setup(gst: FakeGst) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let shots = Arc::new(FakeShots::new(dir.path()));
    let gst: Arc<dyn gst::Gst> = Arc::new(gst);
    let events = Arc::new(Events::default());
    let cards = Arc::new(cards::Cards::new(Box::new(NoView), events.clone()));
    let clipboard = Arc::new(FakeClipboard::default());
    let recorder = Recorder::new(Parts {
        shots: shots.clone(),
        leftovers: Default::default(),
        cast: Arc::new(cast()),
        gst: gst.clone(),
        picker: Arc::new(super::record::tests::FakePicker(Mutex::new(None))),
        events: events.clone(),
        token_file: dir.path().join("token"),
        probe: Duration::from_millis(10),
        tick: Duration::from_millis(5),
    });
    let linux = Arc::new(Linux {
        shots,
        leftovers: Default::default(),
        gst,
        recorder,
        clipboard: clipboard.clone(),
        cards: cards.clone(),
        events,
    });
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let session = Arc::new(Session::new(
        linux,
        Layout::new(dir.path().join("captures")),
        Some(dir.path().to_path_buf()),
        Arc::new(move |event| sink.lock().unwrap().push(event)),
    ));
    Setup {
        dir,
        session,
        cards,
        clipboard,
        events: emitted,
    }
}

fn recording(output: RecordingOutput) -> RecordingRequest {
    RecordingRequest {
        mode: CaptureMode::Fullscreen,
        output,
        fps: 30,
        show_cursor: true,
        microphone: false,
        system_audio: false,
        countdown_s: 0,
        gif_fps: 15,
        gif_max_width: 720,
        copy: false,
        save_dir: Some(String::new()),
        folder: None,
        show_card: false,
        labels: CaptureLabels::default(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linux_webm_recording_is_named_and_indexed_as_webm() {
    let s = setup(FakeGst {
        elements: [&gst::RECORD_ELEMENTS[..], &["vp8enc", "webmmux"]].concat(),
        ..FakeGst::default()
    });
    let started = s
        .session
        .start_recording(recording(RecordingOutput::Mp4))
        .await
        .unwrap();
    assert!(matches!(started, RecordingStart::Recording { .. }));
    let CaptureOutcome::Captured { entry } = s.session.stop_recording().await.unwrap() else {
        panic!("cancelled");
    };
    assert_eq!(entry.kind, CaptureKind::Video);
    assert!(entry.path.ends_with(".webm"), "{}", entry.path);
    assert!(entry.path.contains("Librelog Recording "), "{}", entry.path);
    assert!(std::path::Path::new(&entry.path).starts_with(s.dir.path()));
    assert!(entry.bytes > 0 && entry.duration_ms < 60_000);
    assert_eq!(s.session.list_captures().unwrap(), vec![entry]);
    // No hidden movie is left behind in the library.
    let leftovers: Vec<_> = std::fs::read_dir(s.dir.path().join("captures/files"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cards_buttons_on_linux_reach_the_plugin_through_its_event_poll() {
    let s = setup(h264());
    let request = ScreenshotRequest {
        mode: CaptureMode::Area,
        format: ImageFormat::Png,
        show_cursor: false,
        copy: false,
        save_dir: None,
        folder: None,
        show_card: true,
        labels: CaptureLabels::default(),
    };
    let CaptureOutcome::Captured { entry } = s.session.screenshot(request).await.unwrap() else {
        panic!("cancelled");
    };
    let shown = s.cards.current();
    assert_eq!(shown.cards.len(), 1);
    assert_eq!(shown.cards[0].id, entry.id);
    assert!(!shown.cards[0].saved);
    assert_eq!(shown.labels.copy, "Copy");

    assert!(s.cards.press(&entry.id, "copy"));
    assert!(s.cards.press(&entry.id, "edit"));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while s.clipboard.0.lock().unwrap().is_empty()
        || !s
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, CaptureEvent::CardAction { .. }))
    {
        assert!(std::time::Instant::now() < deadline, "the poll never acted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        s.clipboard.0.lock().unwrap()[0],
        ["image/png", "text/uri-list", "x-special/gnome-copied-files"]
    );

    assert!(s.cards.press(&entry.id, "close"));
    while !s.cards.current().cards.is_empty() {
        assert!(std::time::Instant::now() < deadline, "the card stayed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
