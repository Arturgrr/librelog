use crate::{AppSounds, SfxPluginExt};

/// The longest `play` waits for a sound to start.
const MAX_START: std::time::Duration = std::time::Duration::from_millis(500);

// Resolves once the sound plays, so the caller knows when it is heard, or
// once it never will; at the latest after `MAX_START`.
#[tauri::command]
#[specta::specta]
pub async fn play<R: tauri::Runtime>(app: tauri::AppHandle<R>, sfx: AppSounds) {
    let started = app.sfx().play(sfx);
    let _ = tauri::async_runtime::spawn_blocking(move || started.recv_timeout(MAX_START)).await;
}

#[tauri::command]
#[specta::specta]
pub async fn stop<R: tauri::Runtime>(app: tauri::AppHandle<R>, sfx: AppSounds) {
    app.sfx().stop(sfx)
}

#[tauri::command]
#[specta::specta]
pub async fn set_volume<R: tauri::Runtime>(app: tauri::AppHandle<R>, sfx: AppSounds, volume: f32) {
    app.sfx().set_volume(sfx, volume)
}
