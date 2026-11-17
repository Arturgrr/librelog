//! What the Wayland desktop did with Librelog's shortcuts, for Settings:
//! whether its dialog is pending, dismissed or failed, and the key it bound
//! for each shortcut, which the user may have changed in its dialog or its
//! keyboard settings.

use tauri_specta::Event;

use crate::{DesktopShortcut, DesktopShortcutsEvent, DesktopShortcutsState};

#[derive(Default)]
pub struct DesktopStatus {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    event: Option<DesktopShortcutsEvent>,
    #[cfg(target_os = "linux")]
    status: Option<super::desktop::Status>,
    #[cfg(target_os = "linux")]
    labels: std::collections::HashMap<tauri_plugin_global_shortcut::Shortcut, String>,
}

impl DesktopStatus {
    pub fn current(&self) -> DesktopShortcutsEvent {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .event
            .clone()
            .unwrap_or(DesktopShortcutsEvent {
                state: DesktopShortcutsState::Unused,
                shortcuts: Vec::new(),
            })
    }
}

#[cfg(target_os = "linux")]
impl DesktopStatus {
    pub fn label(&self, key: tauri_plugin_global_shortcut::Shortcut, shortcut: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .labels
            .insert(key, shortcut.to_owned());
    }

    pub(super) fn report(&self, app: &tauri::AppHandle, status: super::desktop::Status) {
        let event = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let event = event(&status, &inner.labels);
            inner.status = Some(status);
            inner.event = Some(event.clone());
            event
        };
        let _ = event.emit(app);
    }

    pub(super) fn triggers_changed(
        &self,
        app: &tauri::AppHandle,
        triggers: super::portal::Triggers,
    ) {
        let status = {
            let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match &inner.status {
                Some(super::desktop::Status::Bound(bindings, known)) => {
                    let mut known = known.clone();
                    known.extend(triggers);
                    super::desktop::Status::Bound(bindings.clone(), known)
                }
                _ => return,
            }
        };
        self.report(app, status);
    }
}

#[cfg(target_os = "linux")]
fn event(
    status: &super::desktop::Status,
    labels: &std::collections::HashMap<tauri_plugin_global_shortcut::Shortcut, String>,
) -> DesktopShortcutsEvent {
    use super::desktop::Status;
    let entries = |bindings: &[super::bindings::Binding],
                   triggers: Option<&super::portal::Triggers>| {
        bindings
            .iter()
            .map(|binding| {
                let trigger = triggers
                    .and_then(|triggers| triggers.get(&binding.id))
                    .map(|trigger| trigger.trim().to_owned())
                    .filter(|trigger| !trigger.is_empty());
                DesktopShortcut {
                    id: binding.id.clone(),
                    shortcut: binding.key.and_then(|key| labels.get(&key).cloned()),
                    differs: trigger.as_deref().is_some_and(|trigger| {
                        !super::bindings::same_trigger(&binding.trigger, trigger)
                    }),
                    trigger,
                }
            })
            .collect()
    };
    let (state, shortcuts) = match status {
        Status::Unused => (DesktopShortcutsState::Unused, Vec::new()),
        Status::Waiting(bindings) => (DesktopShortcutsState::Waiting, entries(bindings, None)),
        Status::Bound(bindings, triggers) => (
            DesktopShortcutsState::Bound,
            entries(bindings, Some(triggers)),
        ),
        Status::Dismissed(bindings) => (DesktopShortcutsState::Dismissed, entries(bindings, None)),
        Status::Failed(message) => (
            DesktopShortcutsState::Failed {
                message: message.clone(),
            },
            Vec::new(),
        ),
    };
    DesktopShortcutsEvent { state, shortcuts }
}

/// What the desktop did with the shortcuts on Wayland. `unused` elsewhere,
/// and while no shortcut is set.
#[tauri::command]
#[specta::specta]
pub fn desktop_shortcuts(app: tauri::AppHandle) -> DesktopShortcutsEvent {
    use tauri::Manager;
    app.state::<DesktopStatus>().current()
}

/// Shows the desktop's shortcut dialog again after it was dismissed or
/// failed. Nothing happens outside Wayland.
#[tauri::command]
#[specta::specta]
pub fn ask_desktop_again(app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    if super::uses_portal() {
        use tauri::Manager;
        app.state::<std::sync::Arc<super::desktop::Binder>>()
            .ask_again();
        super::wanted_changed(&app);
    }
    let _ = app;
}
