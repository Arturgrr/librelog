use std::sync::Mutex;

use tauri::{
    AppHandle, Result,
    menu::{CheckMenuItem, MenuId, Submenu},
};

/// The dictation presets the tray menu offers, with labels in the app
/// language. Hidden while there are fewer than two.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TrayDictationPresets {
    pub title: String,
    pub items: Vec<String>,
    /// The preset dictation uses now, if any.
    pub active: Option<u32>,
}

static PRESETS: Mutex<Option<TrayDictationPresets>> = Mutex::new(None);

const ID_PREFIX: &str = "anlg_tray_dictation_preset_";

/// Replaces the presets shown; returns whether they changed.
pub(crate) fn set(presets: Option<TrayDictationPresets>) -> bool {
    let presets = presets.filter(|presets| presets.items.len() > 1);
    let mut current = PRESETS.lock().unwrap_or_else(|e| e.into_inner());
    if *current == presets {
        return false;
    }
    *current = presets;
    true
}

pub(crate) fn build(app: &AppHandle<tauri::Wry>) -> Result<Option<Submenu<tauri::Wry>>> {
    let Some(presets) = PRESETS.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return Ok(None);
    };
    let submenu = Submenu::new(app, &presets.title, true)?;
    for (index, label) in presets.items.iter().enumerate() {
        let checked = presets.active == Some(index as u32);
        submenu.append(&CheckMenuItem::with_id(
            app,
            format!("{ID_PREFIX}{index}"),
            label,
            true,
            checked,
            None::<&str>,
        )?)?;
    }
    Ok(Some(submenu))
}

fn preset_index(id: &MenuId) -> Option<u32> {
    id.0.strip_prefix(ID_PREFIX)?.parse().ok()
}

/// Handles a click on one of the presets: the same event as the preset
/// shortcut, naming the preset chosen. Other ids are ignored.
pub fn handle_menu_event(app: &AppHandle<tauri::Wry>, id: &MenuId) {
    use tauri_specta::Event;

    if let Some(preset) = preset_index(id) {
        let _ = tauri_plugin_shortcut::DictationPresetEvent {
            preset: Some(preset),
        }
        .emit(app);
    }
}
