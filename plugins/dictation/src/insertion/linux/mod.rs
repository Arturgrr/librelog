//! Linux insertion. AT-SPI's `EditableText` first: it needs no permission
//! and leaves the clipboard alone, but only native editable fields take it
//! (not terminals, Chromium, Electron or Firefox, nor apps without an
//! accessible field, nor Qt apps). Everything else, and a field that leaves
//! its text unchanged, gets the clipboard and the paste keys, as on macOS:
//! see [`paste`]. Only a password field refuses, and a field that took part
//! of the text is reported rather than pasted into again.

mod accessible;
mod paste;
mod remote;
mod route;
mod system;

pub use accessible::focused_field;
pub use remote::set_data_dir;

use route::{Chord, KeyPath, Target};

const FOCUS_CHANGED: &str =
    "The focused field changed. Copy your last dictation from Settings > Dictation.";
/// Pasting after a partial edit would insert part of the text twice.
const INCOMPLETE: &str = "The app may not have inserted all of the dictation. Check the field, or copy your last dictation from Settings > Dictation.";

async fn focused_target() -> Result<Target, String> {
    let focus = accessible::focus().await;
    tauri::async_runtime::spawn_blocking(move || {
        route::target(
            focus,
            system::x11_active_app,
            std::process::id(),
            system::exists,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn capture() -> Result<String, String> {
    let target = focused_target().await?;
    // The first paste may need the desktop's approval: ask while the user
    // speaks rather than when the transcript is ready.
    if matches!(target, Target::Paste { .. }) && system::key_paths().contains(&KeyPath::Portal) {
        remote::prepare();
    }
    serde_json::to_string(&target).map_err(|e| e.to_string())
}

pub async fn insert(target: String, text: String) -> Result<(), String> {
    let target: Target = serde_json::from_str(&target).map_err(|e| e.to_string())?;
    let chord = match target {
        Target::Paste { terminal, .. } => Chord::for_terminal(terminal),
        Target::Accessible { object } => {
            let object = serde_json::from_str(&object).map_err(|e| e.to_string())?;
            match accessible::insert(object, &text).await {
                accessible::Inserted::Yes => return Ok(()),
                accessible::Inserted::FocusChanged => return Err(FOCUS_CHANGED.into()),
                accessible::Inserted::Password => return Err(route::PASSWORD.into()),
                accessible::Inserted::Incomplete(reason) => {
                    tracing::warn!(%reason, "AT-SPI may not have inserted all of the dictation");
                    return Err(INCOMPLETE.into());
                }
                accessible::Inserted::Refused(reason) => {
                    tracing::info!(%reason, "AT-SPI did not insert the dictation; pasting it");
                    Chord::Paste
                }
            }
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut clipboard = match system::SystemClipboard::open() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                tracing::warn!(%error, "the clipboard is unavailable");
                return Err(paste::NO_CLIPBOARD.to_string());
            }
        };
        let paths = system::key_paths();
        let mut keys = system::Keys(paths);
        let keyboard = if keys.0.is_empty() {
            Err("no display session".to_string())
        } else {
            Ok(&mut keys)
        };
        paste::paste(&mut clipboard, keyboard, &text, chord, std::thread::sleep)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Presses Return where `target`'s insertion went, while it is still focused there. The portal
/// is used only once approved: its dialog would take the focus Return is meant for.
pub async fn press_return(target: String) -> Result<(), String> {
    let target: Target = serde_json::from_str(&target).map_err(|e| e.to_string())?;
    if !target.still_focused(&focused_target().await?) {
        return Err(FOCUS_CHANGED.into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let paths: Vec<_> = system::key_paths()
            .into_iter()
            .filter(|path| *path != KeyPath::Portal || remote::approved())
            .collect();
        if paths.is_empty() {
            return Err("no approved way to send keys in this session".to_string());
        }
        paste::Keyboard::send(&mut system::Keys(paths), Chord::Return)
    })
    .await
    .map_err(|e| e.to_string())?
}
