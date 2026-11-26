use std::sync::Arc;

use tauri::Manager;

use crate::{
    events::HudEvent,
    ext::DictationPluginExt,
    recorder::{Feedback, RecordedAudio, Recorder},
};

/// Fails fast when text could not be inserted where the focus is now. Skipped
/// while an insertion is in flight, which it must not disturb.
#[tauri::command]
#[specta::specta]
pub(crate) async fn check_insertion() -> Result<(), String> {
    crate::insertion::check().await
}

/// Inserts `text` where the focus is when its turn comes: insertions run one
/// at a time, in the order they were asked for.
#[tauri::command]
#[specta::specta]
pub(crate) async fn insert_text(text: String) -> Result<(), String> {
    if text.contains('\0') {
        return Err("Dictation text contains an unsupported NUL character".into());
    }
    if text.is_empty() || text.len() > 100_000 {
        return Err("Dictation text is empty or too long".into());
    }
    crate::insertion::insert(text).await
}

/// Inserts a dictation where the focus is when its turn comes, fitted to the text before the
/// caret or over the selection a voice edit read; returns what was typed. `exclusions` keeps
/// the field of an excluded app unread.
#[tauri::command]
#[specta::specta]
pub(crate) async fn insert_dictation(
    text: String,
    mode: crate::insertion::InsertMode,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
) -> Result<String, String> {
    if text.contains('\0') {
        return Err("Dictation text contains an unsupported NUL character".into());
    }
    if text.is_empty() || text.len() > 100_000 {
        return Err("Dictation text is empty or too long".into());
    }
    crate::insertion::insert_dictation(text, mode, exclusions).await
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

#[tauri::command]
#[specta::specta]
pub(crate) async fn start_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    owner: String,
) -> Result<(), String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    app.state::<Recorder>()
        .start(audio, microphone_device, owner)
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

/// Records for system-wide dictation: the HUD follows the microphone, the
/// live preview's words arrive over `updates`, and `environment` quiets other
/// audio and keeps the computer awake until the recording ends.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_system_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    owner: String,
    preview: Option<crate::preview::PreviewConfig>,
    updates: tauri::ipc::Channel<crate::preview::RecordingUpdate>,
    environment: Option<crate::environment::RecordingEnvironment>,
) -> Result<(), String> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
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
            },
        )
        .map_err(|error| error.to_string())
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
