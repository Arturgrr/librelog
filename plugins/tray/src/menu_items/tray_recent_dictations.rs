use std::sync::Mutex;

use tauri::{
    AppHandle, Result,
    menu::{MenuId, MenuItem, Submenu},
};

/// The newest dictations the tray menu offers to copy, as previews, with
/// labels in the app language. With no items the submenu holds `empty`,
/// disabled.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TrayRecentDictations {
    pub title: String,
    pub empty: String,
    pub items: Vec<TrayRecentDictation>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TrayRecentDictation {
    /// The history entry's id, sent back when it is chosen.
    pub id: String,
    pub preview: String,
}

/// A recent dictation was chosen in the tray menu: copy its text.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type, tauri_specta::Event)]
pub struct CopyRecentDictationEvent {
    pub id: String,
}

static RECENT: Mutex<Option<TrayRecentDictations>> = Mutex::new(None);

const ID_PREFIX: &str = "anlg_tray_recent_dictation:";
const EMPTY_ID: &str = "anlg_tray_recent_dictations_empty";

/// Replaces the dictations shown; returns whether they changed.
pub(crate) fn set(recent: Option<TrayRecentDictations>) -> bool {
    let mut current = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    if *current == recent {
        return false;
    }
    *current = recent;
    true
}

/// The submenu's items as (id, label, enabled).
fn rows(recent: &TrayRecentDictations) -> Vec<(String, String, bool)> {
    if recent.items.is_empty() {
        return vec![(EMPTY_ID.into(), escape(&recent.empty), false)];
    }
    recent
        .items
        .iter()
        .map(|item| {
            (
                format!("{ID_PREFIX}{}", item.id),
                escape(&item.preview),
                true,
            )
        })
        .collect()
}

// Menus read `&` as a mnemonic marker; dictated text means it literally.
fn escape(label: &str) -> String {
    label.replace('&', "&&")
}

pub(crate) fn build(app: &AppHandle<tauri::Wry>) -> Result<Option<Submenu<tauri::Wry>>> {
    let Some(recent) = RECENT.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return Ok(None);
    };
    let submenu = Submenu::new(app, escape(&recent.title), true)?;
    for (id, label, enabled) in rows(&recent) {
        submenu.append(&MenuItem::with_id(app, id, label, enabled, None::<&str>)?)?;
    }
    Ok(Some(submenu))
}

fn dictation_id(id: &MenuId) -> Option<&str> {
    id.0.strip_prefix(ID_PREFIX).filter(|id| !id.is_empty())
}

/// Handles a click on one of the dictations: asks the frontend, which owns
/// the history, to copy its current text. Other ids are ignored.
pub(crate) fn handle_menu_event(app: &AppHandle<tauri::Wry>, id: &MenuId) {
    use tauri_specta::Event;

    if let Some(id) = dictation_id(id) {
        let _ = CopyRecentDictationEvent { id: id.into() }.emit(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(items: &[(&str, &str)]) -> TrayRecentDictations {
        TrayRecentDictations {
            title: "Copy recent dictation".into(),
            empty: "No recent dictations".into(),
            items: items
                .iter()
                .map(|(id, preview)| TrayRecentDictation {
                    id: (*id).into(),
                    preview: (*preview).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn without_dictations_the_submenu_holds_one_disabled_item() {
        let rows = rows(&recent(&[]));

        assert_eq!(
            rows,
            vec![(EMPTY_ID.to_string(), "No recent dictations".into(), false)]
        );
        assert_eq!(dictation_id(&MenuId::new(&rows[0].0)), None);
    }

    #[test]
    fn a_chosen_preview_names_its_dictation_and_shows_ampersands() {
        let rows = rows(&recent(&[("a1", "Rock & roll"), ("b2", "Second")]));

        assert_eq!(rows[0].1, "Rock && roll");
        assert!(rows.iter().all(|(_, _, enabled)| *enabled));
        let ids: Vec<_> = rows
            .iter()
            .map(|(id, _, _)| dictation_id(&MenuId::new(id)).map(str::to_owned))
            .collect();
        assert_eq!(ids, vec![Some("a1".into()), Some("b2".into())]);
        assert_eq!(dictation_id(&MenuId::new("anlg_tray_settings")), None);
    }
}
