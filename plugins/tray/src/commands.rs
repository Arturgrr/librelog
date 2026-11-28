use crate::TrayPluginExt;

#[tauri::command]
#[specta::specta]
pub async fn set_tray_icon_visible(
    app: tauri::AppHandle<tauri::Wry>,
    visible: bool,
) -> Result<(), String> {
    app.tray().set_visible(visible).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_tray_labels(
    app: tauri::AppHandle<tauri::Wry>,
    labels: crate::labels::TrayLabels,
) -> Result<(), String> {
    app.tray()
        .set_labels(labels)
        .map_err(|error| error.to_string())
}

/// Shows "Paste last dictation" in the menu bar icon's menu on macOS while
/// dictation and its paste-last shortcut are enabled. Other platforms never
/// show it.
#[tauri::command]
#[specta::specta]
pub async fn set_tray_paste_last_visible(
    app: tauri::AppHandle<tauri::Wry>,
    visible: bool,
) -> Result<(), String> {
    app.tray()
        .set_paste_last_visible(visible)
        .map_err(|error| error.to_string())
}

/// Shows "Capture Area" and "Record Screen" in the menu bar icon's menu
/// while Capture is on.
#[tauri::command]
#[specta::specta]
pub async fn set_tray_capture_visible(
    app: tauri::AppHandle<tauri::Wry>,
    visible: bool,
) -> Result<(), String> {
    app.tray()
        .set_capture_visible(visible)
        .map_err(|error| error.to_string())
}

/// Shows the dictation presets in a submenu of the tray menu, the one in use
/// checked, so one can be picked there; `null` or fewer than two hides it.
#[tauri::command]
#[specta::specta]
pub async fn set_tray_dictation_presets(
    app: tauri::AppHandle<tauri::Wry>,
    presets: Option<crate::TrayDictationPresets>,
) -> Result<(), String> {
    app.tray()
        .set_dictation_presets(presets)
        .map_err(|error| error.to_string())
}

/// Shows the newest dictations in a submenu of the tray menu, to copy one;
/// with none it says so, disabled, and `null` hides it.
#[tauri::command]
#[specta::specta]
pub async fn set_tray_recent_dictations(
    app: tauri::AppHandle<tauri::Wry>,
    recent: Option<crate::TrayRecentDictations>,
) -> Result<(), String> {
    app.tray()
        .set_recent_dictations(recent)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn set_tray_recording_title(
    app: tauri::AppHandle<tauri::Wry>,
    title: Option<String>,
) -> Result<(), String> {
    app.tray()
        .set_recording_title(title)
        .map_err(|error| error.to_string())
}

/// Claims a "Start a new meeting" click from the menu bar icon. Returns true
/// once per click, so it starts one recording however many times it is asked.
#[tauri::command]
#[specta::specta]
pub async fn take_pending_tray_start() -> Result<bool, String> {
    Ok(crate::menu_items::TrayStart::take_pending_start())
}

/// Whether the desktop shows tray icons at all: GNOME shows none without
/// the AppIndicator extension. True elsewhere; false while it cannot tell.
#[tauri::command]
#[specta::specta]
pub async fn tray_icon_shown_by_desktop() -> Result<bool, String> {
    Ok(crate::ext::desktop_shows_tray_icons_async()
        .await
        .unwrap_or(false))
}
