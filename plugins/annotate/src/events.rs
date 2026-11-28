use serde::{Deserialize, Serialize};

/// A recording that stopped and waits for its transcript: pass `id` to
/// `finalize`.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub id: String,
    pub dir: String,
    /// `audio.wav`, aligned to the capture's start; absent when the
    /// microphone failed.
    pub audio_path: Option<String>,
    pub duration_ms: u32,
    /// `stop`, `max_duration` or `stream_error`.
    pub ended_by: String,
    /// What the recording missed, the narration's gaps included.
    pub warnings: Vec<String>,
}

/// What the recording did by itself or through its capsule.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AnnotateEvent {
    /// The capsule's stop, the 20-minute cap or a failed stream ended the
    /// recording; it waits for `finalize`.
    Stopped { stopped: Stopped },
    /// The capsule's Discard: the recording and its folder are gone.
    Discarded,
    /// Ink came on or went off from the capsule's pen, Escape or idling.
    Ink { on: bool },
    /// Ending the recording failed; the session is idle again.
    Failed { message: String },
    /// The narration's microphone stopped (`heard` false) while recording,
    /// or opened again after stopping or failing to open. The screen keeps
    /// recording either way.
    Narration { heard: bool },
}

/// What the capsule shows: the recording, then, once it stopped, that the
/// annotation is being prepared, and whether it came out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CapsuleStage {
    #[default]
    Recording,
    Processing,
    /// Its summary and contact sheet are ready to paste.
    Ready,
    Failed,
}

/// What the ink overlay and the capsule show while recording on Linux.
#[derive(Clone, Debug, Default, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateSurface {
    pub stage: CapsuleStage,
    pub ink: crate::commands::InkMode,
    pub pen_on: bool,
    /// The overlay is up: the pen can draw.
    pub drawable: bool,
    /// The capsule asks "Discard?".
    pub asking: bool,
    pub started_wall_ms: i64,
    /// The narration's microphone isn't recording: it was off, failed to
    /// open or stopped.
    pub unheard: bool,
    pub labels: crate::session::CapsuleLabels,
}

/// The narration's level on a meter, 0 to 1, about 25 times a second while
/// the microphone hears: for the Linux capsule.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct AnnotateLevel {
    pub level: f32,
}
