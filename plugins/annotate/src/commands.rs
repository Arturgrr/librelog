use std::sync::Arc;

use anlg_annotate_core::retention::Preset;
use tauri::Manager;

use crate::bundle::{Finalized, TranscriptInput};
use crate::events::Stopped;
use crate::session::{CapsuleLabels, Session, Started, Status};
use crate::{Error, blocking_native};

type Result<T> = std::result::Result<T, String>;

fn text(error: Error) -> String {
    error.to_string()
}

fn session<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Arc<Session> {
    app.state::<Arc<Session>>().inner().clone()
}

/// `granted`, `denied`, `not_determined` or `needs_restart` (granted, but
/// only a relaunch lets this process record).
#[tauri::command]
#[specta::specta]
pub(crate) async fn screen_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<String> {
    let native = session(&app).native().clone();
    blocking_native(move || native.probe_screen_recording())
        .await
        .map_err(text)
}

/// Shows the system prompt, which macOS shows once.
#[tauri::command]
#[specta::specta]
pub(crate) async fn request_screen_recording<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<bool> {
    let native = session(&app).native().clone();
    blocking_native(move || native.request_screen_recording())
        .await
        .map_err(text)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn open_screen_recording_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<()> {
    let native = session(&app).native().clone();
    blocking_native(move || native.open_screen_recording_settings())
        .await
        .map_err(text)
}

/// Starts recording the display under the pointer and the microphone.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    microphone_device: Option<String>,
    labels: Option<CapsuleLabels>,
) -> Result<Started> {
    let audio = app
        .state::<Arc<dyn anlg_audio::AudioProvider>>()
        .inner()
        .clone();
    session(&app)
        .start(audio, microphone_device, labels)
        .await
        .map_err(text)
}

/// Stops the screen and the microphone; the recording waits for `finalize`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn stop<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<Stopped> {
    session(&app).stop().await.map_err(text)
}

/// Writes the bundle of the stopped recording `id`. `transcript` is null when
/// transcription failed or found no speech.
#[tauri::command]
#[specta::specta]
pub(crate) async fn finalize<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    transcript: Option<TranscriptInput>,
) -> Result<Finalized> {
    session(&app).finalize(&id, transcript).await.map_err(text)
}

/// Throws away the recording, or the stopped one waiting for `finalize`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cancel<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<()> {
    session(&app).cancel().await.map_err(text)
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "camelCase")]
pub enum InkMode {
    #[default]
    Off,
    /// The pointer inks as it moves: while the shortcut is held.
    Hover,
    /// The pointer inks while its button is down.
    Drag,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn set_ink_mode<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    mode: InkMode,
) -> Result<()> {
    let mode = match mode {
        InkMode::Off => "off",
        InkMode::Hover => "hover",
        InkMode::Drag => "drag",
    };
    session(&app).set_ink_mode(mode).map_err(text)
}

/// Drawing on or off, as the capsule's pen: what the pen shortcut does.
#[tauri::command]
#[specta::specta]
pub(crate) async fn toggle_pen<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<()> {
    session(&app).toggle_pen().map_err(text)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn status<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<Status> {
    Ok(session(&app).status())
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Retention {
    Week,
    Month,
    Forever,
}

/// Sets how long annotations are kept and applies it now; it is applied again
/// every hour, never while recording.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_retention<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    retention: Retention,
) -> Result<()> {
    let session = session(&app);
    session.set_retention(match retention {
        Retention::Week => Preset::Week,
        Retention::Month => Preset::Month,
        Retention::Forever => Preset::Forever,
    });
    session.run_retention().await;
    Ok(())
}

/// The folder that holds the annotations, created when missing.
#[tauri::command]
#[specta::specta]
pub(crate) async fn annotations_dir<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<String> {
    let root = session(&app).root().to_path_buf();
    crate::session::ensure_root(&root).map_err(|error| text(error.into()))?;
    Ok(root.to_string_lossy().into_owned())
}

/// The newest finished annotation.
#[tauri::command]
#[specta::specta]
pub(crate) async fn latest<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<Finalized>> {
    let root = session(&app).root().to_path_buf();
    tauri::async_runtime::spawn_blocking(move || crate::bundle::latest(&root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(text)
}

#[cfg(target_os = "linux")]
fn linux<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<Arc<crate::linux::Linux>> {
    app.try_state::<Arc<crate::linux::Linux>>()
        .map(|linux| linux.inner().clone())
}

/// Only the Linux overlay's and capsule's own windows may speak for them.
fn from<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, labels: &[&str]) -> Result<()> {
    if labels.contains(&window.label()) {
        Ok(())
    } else {
        Err(format!("not for the {} window", window.label()))
    }
}

const INK: &str = "annotate-ink";
const CAPSULE: &str = "annotate-capsule";

/// What the Linux ink overlay and capsule show; null while nothing records
/// there, and always elsewhere.
#[tauri::command]
#[specta::specta]
pub(crate) async fn surface<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
) -> Result<Option<crate::events::AnnotateSurface>> {
    from(&window, &[INK, CAPSULE])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        return Ok(linux.surface());
    }
    let _ = app;
    Ok(None)
}

/// The Linux overlay's or capsule's page mounted: it may be shown.
#[tauri::command]
#[specta::specta]
pub(crate) async fn surface_ready<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
) -> Result<()> {
    from(&window, &[INK, CAPSULE])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        linux.loaded(window.label());
    }
    let _ = app;
    Ok(())
}

/// The Linux ink overlay's pointer, in its own points: `move`, `down`, `up`
/// or `leave`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn ink_pointer<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    kind: String,
    x: f64,
    y: f64,
) -> Result<()> {
    from(&window, &[INK])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        linux.pointer(&kind, x, y);
    }
    let _ = (app, kind, x, y);
    Ok(())
}

/// A scroll on the Linux ink overlay, as whole wheel clicks along each axis
/// (0 while the overlay gathers a smooth scroll): it goes through to the
/// app underneath.
#[tauri::command]
#[specta::specta]
pub(crate) async fn ink_scroll<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    dx: f64,
    dy: f64,
) -> Result<()> {
    from(&window, &[INK])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        tauri::async_runtime::spawn_blocking(move || linux.scroll(dx, dy))
            .await
            .map_err(|error| error.to_string())?;
    }
    let _ = (app, dx, dy);
    Ok(())
}

/// A right click on the Linux ink overlay: drawing stops.
#[tauri::command]
#[specta::specta]
pub(crate) async fn ink_escape<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
) -> Result<()> {
    from(&window, &[INK])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        linux.escape();
    }
    let _ = app;
    Ok(())
}

/// A button of the Linux capsule: `pen`, `stop`, `discard`,
/// `confirm_discard` or `keep`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn capsule_pressed<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    target: String,
) -> Result<()> {
    from(&window, &[CAPSULE])?;
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        linux.press(&target);
    }
    let _ = (app, target);
    Ok(())
}

/// Linux: the next annotation asks which screen to share again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn forget_screen_choice<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(linux) = linux(&app) {
        return linux.forget_screen().map_err(|error| text(error.into()));
    }
    let _ = app;
    Ok(())
}
