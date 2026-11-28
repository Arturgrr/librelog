use std::sync::Arc;

use tauri::Manager;

use crate::{
    events::HudEvent,
    ext::DictationPluginExt,
    level::{LevelSource, LevelUpdate},
    microphone::MicrophoneChoice,
    recorder::{Feedback, RecordedAudio, Recorder},
};

/// Fails fast when text could not be inserted where the focus is now. Skipped
/// while an insertion is in flight, which it must not disturb.
#[tauri::command]
#[specta::specta]
pub(crate) async fn check_insertion() -> Result<(), String> {
    crate::insertion::check().await
}

/// The most a dictation or paste-last inserts: hours of speech, well within what a paste or an
/// Accessibility edit takes at once.
const MAX_INSERT_BYTES: usize = 2_000_000;

fn insertable(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        return Err("Dictation text contains an unsupported NUL character".into());
    }
    if text.is_empty() {
        return Err("Dictation text is empty".into());
    }
    if text.len() > MAX_INSERT_BYTES {
        return Err(
            "This dictation is too long to insert. Copy your last dictation from Settings > Dictation."
                .into(),
        );
    }
    Ok(())
}

/// Inserts `text` where the focus is when its turn comes: insertions run one
/// at a time, in the order they were asked for.
#[tauri::command]
#[specta::specta]
pub(crate) async fn insert_text(text: String) -> Result<(), String> {
    insertable(&text)?;
    crate::insertion::insert(text).await
}

/// Inserts a dictation where the focus is when its turn comes, fitted to the text before the
/// caret or over the selection a voice edit read; returns what was typed. `exclusions` keeps
/// the field of an excluded app unread. `send` presses Return once the whole dictation is in,
/// while the field it went into still has focus.
#[tauri::command]
#[specta::specta]
pub(crate) async fn insert_dictation(
    text: String,
    mode: crate::insertion::InsertMode,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
    send: Option<crate::insertion::SendAfterInsert>,
) -> Result<String, String> {
    insertable(&text)?;
    crate::insertion::insert_dictation(text, mode, exclusions, send).await
}

/// The focused field as a dictation starts: its text around the caret, the selection a voice
/// edit may rewrite, and the window's context for cleanup, each only where the privacy decision
/// and `request` allow.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_dictation_field(
    request: crate::insertion::field::FieldRequest,
) -> crate::insertion::field::DictationField {
    crate::insertion::read_field(request).await
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn hud<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    event: HudEvent,
) -> Result<(), String> {
    app.dictation().hud(event).await.map_err(|e| e.to_string())
}

/// Records for in-app dictation, from the same microphone as system-wide
/// dictation: without a chosen one, a Bluetooth default gives way to a wired
/// one unless `avoid_bluetooth_microphone` is false. `updates` hears when the
/// microphone fails before the recording is stopped.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    owner: String,
    avoid_bluetooth_microphone: Option<bool>,
    updates: tauri::ipc::Channel<crate::preview::RecordingUpdate>,
) -> Result<(), String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    let microphone_device =
        choose_microphone(&audio, microphone_device, avoid_bluetooth_microphone)
            .await?
            .device;
    app.state::<Recorder>()
        .start_with_feedback(
            audio,
            microphone_device,
            owner,
            Feedback {
                updates: Some(updates),
                ..Feedback::default()
            },
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    owner: String,
) -> Result<RecordedAudio, String> {
    app.state::<Recorder>()
        .stop(&owner)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    owner: String,
) -> Result<(), String> {
    app.state::<Recorder>()
        .cancel(&owner)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn discard_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    file_path: String,
) -> Result<(), String> {
    app.state::<Recorder>()
        .discard(file_path)
        .map_err(|error| error.to_string())
}

/// Keeps the dictation microphone open between dictations, so each one
/// includes a moment from before its shortcut, or closes it. A change waits
/// for a dictation reading the open microphone to end. Without a chosen
/// microphone it follows the system default, and a Bluetooth default gives
/// way to a wired one unless `avoid_bluetooth_microphone` is false.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_warm_microphone<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    enabled: bool,
    microphone_device: Option<String>,
    avoid_bluetooth_microphone: Option<bool>,
) -> Result<(), String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    let avoid = avoid_bluetooth_microphone.unwrap_or(true);
    let recorder = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        recorder
            .state::<Recorder>()
            .set_warm_microphone(audio, microphone_device, avoid, enabled)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

/// When the start cue began and stopped playing, in milliseconds since the
/// Unix epoch.
#[derive(Debug, Clone, Copy, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CueSpan {
    pub start_ms: f64,
    pub end_ms: f64,
}

fn epoch_ms(ms: f64) -> Option<std::time::SystemTime> {
    (ms.is_finite() && ms >= 0.0)
        .then(|| std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(ms / 1_000.0))
}

/// Records for system-wide dictation: the HUD follows the microphone, the
/// live preview's words arrive over `updates`, and `environment` quiets other
/// audio and keeps the computer awake until the recording ends. `pressed_at_ms`
/// is when the shortcut went down, in milliseconds since the Unix epoch, and
/// `start_cue` when the start cue played, which the open microphone silences
/// unless someone talks over it.
/// Without a chosen microphone, a Bluetooth default gives way to a wired one
/// unless `avoid_bluetooth_microphone` is false.
// The webview passes each argument by name.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_system_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    owner: String,
    preview: Option<crate::preview::PreviewConfig>,
    updates: tauri::ipc::Channel<crate::preview::RecordingUpdate>,
    environment: Option<crate::environment::RecordingEnvironment>,
    pressed_at_ms: Option<f64>,
    avoid_bluetooth_microphone: Option<bool>,
    start_cue: Option<CueSpan>,
) -> Result<(), String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    let microphone_device =
        choose_microphone(&audio, microphone_device, avoid_bluetooth_microphone)
            .await?
            .device;
    let hud = app.clone();
    let meter: crate::recorder::Meter = Arc::new(move |rms| hud.dictation().meter(rms));
    app.state::<Recorder>()
        .start_with_feedback(
            audio,
            microphone_device,
            owner,
            Feedback {
                preview,
                updates: Some(updates),
                meter: Some(meter),
                environment,
                pressed_at: pressed_at_ms.and_then(epoch_ms),
                start_cue: start_cue
                    .and_then(|cue| epoch_ms(cue.start_ms).zip(epoch_ms(cue.end_ms))),
            },
        )
        .map_err(|error| error.to_string())
}

/// The microphone a dictation records from given the chosen one, and the
/// Bluetooth default it passes over. `avoid_bluetooth_microphone` is on
/// unless a build says otherwise.
async fn choose_microphone(
    audio: &Arc<dyn anlg_audio::AudioProvider>,
    microphone_device: Option<String>,
    avoid_bluetooth_microphone: Option<bool>,
) -> Result<MicrophoneChoice, String> {
    let audio = audio.clone();
    let avoid = avoid_bluetooth_microphone.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        crate::microphone::choose(audio.as_ref(), microphone_device, avoid)
    })
    .await
    .map_err(|error| error.to_string())
}

/// Which microphone dictation records from with these settings, for the
/// note in Settings.
#[tauri::command]
#[specta::specta]
pub(crate) async fn dictation_microphone<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    avoid_bluetooth_microphone: bool,
) -> Result<MicrophoneChoice, String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    choose_microphone(&audio, microphone_device, Some(avoid_bluetooth_microphone)).await
}

/// Sends the level of the microphone dictation would record from over
/// `levels` until `stop_input_level`, a minute, or a dictation that needs the
/// microphone; never records. Callers check microphone access first, so this
/// never prompts for it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_input_level<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    avoid_bluetooth_microphone: bool,
    levels: tauri::ipc::Channel<LevelUpdate>,
) -> Result<LevelSource, String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    let microphone_device =
        choose_microphone(&audio, microphone_device, Some(avoid_bluetooth_microphone))
            .await?
            .device;
    // Opening a microphone blocks.
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Recorder>().monitor_level(
            audio.as_ref(),
            microphone_device,
            move |update| levels.send(update).is_ok(),
            crate::level::MAX_LEVEL_MONITOR,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn stop_input_level<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    app.state::<Recorder>().stop_level();
}

/// Asks macOS to let "Pause media" control each known media player that is
/// running, prompting for any it has not asked about yet, and says where each
/// stands. Empty on other platforms.
#[tauri::command]
#[specta::specta]
pub(crate) async fn request_media_control()
-> Result<Vec<anlg_recording_environment::PlayerControl>, String> {
    // Each prompt blocks until it is answered.
    tauri::async_runtime::spawn_blocking(anlg_recording_environment::request_media_control)
        .await
        .map_err(|error| error.to_string())
}

/// The application in front right now, for the dictation history.
#[tauri::command]
#[specta::specta]
pub(crate) async fn frontmost_app() -> Result<Option<crate::target_app::TargetApp>, String> {
    Ok(crate::target_app::frontmost().await)
}

/// Moves a finished recording into the dictation history and returns its
/// file name there; the temporary recording is gone afterwards.
#[tauri::command]
#[specta::specta]
pub(crate) async fn keep_history_audio<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    file_path: String,
) -> Result<String, String> {
    let directory = crate::history::directory(&app).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::history::keep(&directory, &id, file_path, |path| {
            app.state::<Recorder>().discard(path)
        })
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn history_audio_path<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    file_name: String,
) -> Result<String, String> {
    let directory = crate::history::directory(&app).map_err(|error| error.to_string())?;
    crate::history::resolve(&directory, &file_name)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn delete_history_audio<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    file_names: Vec<String>,
) -> Result<(), String> {
    let directory = crate::history::directory(&app).map_err(|error| error.to_string())?;
    crate::history::delete(&directory, &file_names).map_err(|error| error.to_string())
}

/// Deletes kept audio that no history entry refers to.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prune_history_audio<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    referenced: Vec<String>,
) -> Result<(), String> {
    let directory = crate::history::directory(&app).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || crate::history::prune(&directory, &referenced))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// `text` with each spoken phrase the person saved swapped for what is typed in its place.
#[tauri::command]
#[specta::specta]
pub(crate) async fn apply_dictation_replacements(
    text: String,
    replacements: Vec<anlg_dictation_core::Replacement>,
) -> String {
    anlg_dictation_core::apply_replacements(&text, &replacements)
}

/// `text` spaced and cased to be typed after `before`, the field's text before the caret, or
/// as given when `before` is `None` because the field could not be read.
#[tauri::command]
#[specta::specta]
pub(crate) async fn fit_dictation_text(
    text: String,
    before: Option<String>,
    can_join: bool,
) -> anlg_dictation_core::CaretFit {
    anlg_dictation_core::fit_to_caret(&text, before.as_deref(), can_join)
}

/// `next`, a newly transcribed phrase, appended to `committed`, the dictation's text so far.
#[tauri::command]
#[specta::specta]
pub(crate) async fn join_dictation_transcript(committed: String, next: String) -> String {
    anlg_dictation_core::join_transcript(&committed, &next)
}

/// Whether the dictation needs a model call (cleanup or voice edit) and what it says. A `done`
/// plan already holds what to type.
#[tauri::command]
#[specta::specta]
pub(crate) async fn prepare_dictation_refinement(
    request: anlg_dictation_core::RefineRequest,
) -> anlg_dictation_core::RefinePlan {
    anlg_dictation_core::prepare_refinement(request)
}

/// What to type once the model call from `prepare_dictation_refinement` settled.
#[tauri::command]
#[specta::specta]
pub(crate) async fn resolve_dictation_refinement(
    pending: anlg_dictation_core::PendingRefinement,
    reply: anlg_dictation_core::ModelReply,
) -> anlg_dictation_core::RefineOutcome {
    anlg_dictation_core::resolve_refinement(pending, reply)
}

/// Whether a dictation may read this window's title and text for context. `exclusions` is
/// `None` while the person's exclusions can't be read, which denies.
#[tauri::command]
#[specta::specta]
pub(crate) async fn decide_dictation_context(
    enabled: bool,
    facts: anlg_dictation_core::context::WindowFacts,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
) -> anlg_dictation_core::context::ContextAccess {
    anlg_dictation_core::context::decide_context(enabled, &facts, exclusions.as_ref())
}

/// Whether the text around the caret may be read, on this device only, to fit a dictation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn may_read_dictation_for_fitting(
    facts: anlg_dictation_core::context::WindowFacts,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
) -> bool {
    anlg_dictation_core::context::may_read_for_fitting(&facts, exclusions.as_ref())
}

/// The distinctive terms of `text` (names, products, handles), most frequent first. `heading`
/// reads every word as mid-sentence, for a title or an app name.
#[tauri::command]
#[specta::specta]
pub(crate) async fn extract_dictation_terms(
    text: String,
    heading: bool,
    limit: Option<u32>,
) -> Vec<String> {
    let limit = limit.map_or(anlg_dictation_core::context::MAX_CONTEXT_TERMS, |limit| {
        (limit as usize).min(anlg_dictation_core::terms::MAX_TERMS)
    });
    anlg_dictation_core::terms::distinctive_terms(
        &text,
        crate::insertion::field::ordinary_words(),
        heading,
        limit,
    )
}

/// The terms speech-to-text hears for one dictation, highest first.
#[tauri::command]
#[specta::specta]
pub(crate) async fn rank_dictation_terms(request: anlg_dictation_core::RankRequest) -> Vec<String> {
    anlg_dictation_core::rank_terms(&request)
}

/// The weights blob after `request.event`, for the caller to store in place of the old one.
#[tauri::command]
#[specta::specta]
pub(crate) async fn record_dictation_terms(request: anlg_dictation_core::RecordRequest) -> String {
    anlg_dictation_core::record_terms(request)
}

/// Dictionary terms worth suggesting from how the person corrected what a dictation typed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn suggest_dictation_terms(
    request: anlg_dictation_core::SuggestRequest,
) -> Vec<String> {
    anlg_dictation_core::suggest_terms(&request)
}

/// What the person kept of a dictation, from the field around the caret when it started and
/// now; `None` when the field was sent, cleared, or rewritten around it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn kept_dictation_text(
    start: anlg_dictation_core::corrections::FieldText,
    now: anlg_dictation_core::corrections::FieldText,
) -> Option<String> {
    anlg_dictation_core::corrections::kept_text(&start, &now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_dictation_inserts_and_only_an_empty_or_huge_one_is_refused() {
        assert!(insertable(&"palavra ".repeat(50_000)).is_ok());
        assert_eq!(insertable(""), Err("Dictation text is empty".into()));
        assert!(
            insertable(&"a".repeat(MAX_INSERT_BYTES + 1))
                .unwrap_err()
                .contains("too long to insert")
        );
        assert!(insertable("a\0b").is_err());
    }
}
