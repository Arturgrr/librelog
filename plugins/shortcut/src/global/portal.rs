use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tauri::Manager;
use tauri_specta::Event;

use super::Action;
use super::bindings::{Binding, Does, RELEASE_WAIT, ReleaseGate};

use crate::{
    AnnotateShortcutEvent, CaptureShortcutEvent, PasteLastAnnotationEvent, PasteLastEvent,
    ShortcutEvent, annotate, press,
};

pub struct Registration {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tauri::async_runtime::JoinHandle<()>,
    ended: Arc<AtomicBool>,
}

impl Registration {
    /// Closes the session. A deliberate close cancels nothing: the next
    /// session takes over the same shortcuts.
    pub async fn close(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }

    /// The desktop closed the session, or it could not be watched.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }
}

/// Binds `bindings` in a new portal session and acts on each activation and
/// deactivation until the registration closes or the desktop ends the
/// session, which cancels a dictation in progress.
pub async fn bind(app: tauri::AppHandle, bindings: Vec<Binding>) -> Result<Registration, String> {
    let shortcuts: Vec<NewShortcut> = bindings
        .iter()
        .map(|binding| {
            NewShortcut::new(binding.id.clone(), binding.description.clone())
                .preferred_trigger(binding.trigger.as_str())
        })
        .collect();
    let does: HashMap<String, Does> = bindings
        .into_iter()
        .map(|binding| (binding.id, binding.does))
        .collect();

    let connection = anlg_desktop_portal::connection().await.map_err(|e| {
        format!("Librelog could not reach the desktop portals for global shortcuts: {e}")
    })?;
    let portal = GlobalShortcuts::with_connection(connection)
        .await
        .map_err(|e| {
            format!("This Wayland desktop does not provide the Global Shortcuts portal: {e}")
        })?;
    let session = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        portal.create_session(Default::default()),
    )
    .await
    .map_err(|_| "The desktop's Global Shortcuts portal did not answer.".to_string())?
    .map_err(|e| e.to_string())?;
    let setup = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        let pressed = portal
            .receive_activated()
            .await
            .map_err(|e| e.to_string())?;
        let released = portal
            .receive_deactivated()
            .await
            .map_err(|e| e.to_string())?;
        let bound = portal
            .bind_shortcuts(&session, &shortcuts, None, Default::default())
            .await
            .and_then(|request| request.response())
            .map_err(|e| e.to_string())?;
        for shortcut in bound.shortcuts() {
            tracing::info!(
                id = shortcut.id(),
                trigger = shortcut.trigger_description(),
                "the desktop bound a Librelog shortcut"
            );
        }
        Ok::<_, String>((pressed, released))
    })
    .await;
    let (mut pressed, mut released) = match setup {
        Ok(Ok(streams)) => streams,
        error => {
            let _ = session.close().await;
            return Err(match error {
                Ok(Err(error)) => error,
                _ => "Shortcut setup timed out. Retry setup and approve your desktop's shortcut prompt.".into(),
            });
        }
    };
    let session_path: String =
        serde_json::from_value(serde_json::to_value(&session).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let ended = Arc::new(AtomicBool::new(false));
    let handler = Handler {
        does,
        gates: Default::default(),
    };

    let task = tauri::async_runtime::spawn({
        let ended = ended.clone();
        async move {
            let deliberate = match session.receive_closed().await {
                Err(_) => false,
                Ok(mut closed) => loop {
                    tokio::select! {
                        _ = &mut stopped => break true,
                        _ = closed.next() => break false,
                        event = pressed.next() => {
                            let Some(event) = event else { break false };
                            if event.session_handle().as_str() == session_path {
                                handler.on(&app, event.shortcut_id(), true);
                            }
                        }
                        event = released.next() => {
                            let Some(event) = event else { break false };
                            if event.session_handle().as_str() == session_path {
                                handler.on(&app, event.shortcut_id(), false);
                            }
                        }
                    }
                },
            };
            let _ = session.close().await;
            ended.store(true, Ordering::SeqCst);
            app.state::<annotate::AnnotateGesture>().release_key();
            if !deliberate {
                tracing::warn!("the desktop closed Librelog's global shortcuts session");
                press::cancel(&app);
            }
        }
    });
    Ok(Registration { stop, task, ended })
}

struct Handler {
    does: HashMap<String, Does>,
    gates: Arc<Mutex<HashMap<String, ReleaseGate>>>,
}

impl Handler {
    fn on(&self, app: &tauri::AppHandle, id: &str, pressed: bool) {
        let Some(does) = self.does.get(id).copied() else {
            return;
        };
        let action = match does {
            Does::Cancel => {
                if pressed {
                    let state = app.state::<super::GlobalState>();
                    if state.active.load(Ordering::SeqCst) {
                        press::cancel(app);
                    }
                    if state.annotate_escape.load(Ordering::SeqCst) {
                        let _ = AnnotateShortcutEvent::Escape.emit(app);
                    }
                }
                return;
            }
            Does::Act(action) => action,
        };
        match action {
            Action::Dictate(source) => {
                let event = if pressed {
                    ShortcutEvent::Pressed
                } else {
                    ShortcutEvent::Released
                };
                press::emit(app, source, event);
            }
            Action::Annotate if pressed => annotate::pressed(app),
            Action::Annotate => annotate::released(app),
            Action::PasteLast | Action::PasteLastAnnotation => {
                self.paste_once_released(app, id, pressed, action)
            }
            Action::Capture(action) if pressed => {
                let _ = CaptureShortcutEvent { action }.emit(app);
            }
            Action::Capture(_) | Action::Cancel => {}
        }
    }

    fn paste_once_released(&self, app: &tauri::AppHandle, id: &str, pressed: bool, action: Action) {
        let emit = move |app: &tauri::AppHandle| {
            let _ = if matches!(action, Action::PasteLast) {
                PasteLastEvent {}.emit(app)
            } else {
                PasteLastAnnotationEvent {}.emit(app)
            };
        };
        let mut gates = self.gates.lock().unwrap_or_else(|e| e.into_inner());
        let gate = gates.entry(id.to_owned()).or_default();
        if !pressed {
            if gate.release() {
                emit(app);
            }
            return;
        }
        let press = gate.press();
        let (app, gates, id) = (app.clone(), self.gates.clone(), id.to_owned());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(RELEASE_WAIT).await;
            let fire = gates
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&id)
                .is_some_and(|gate| gate.timed_out(press));
            if fire {
                emit(&app);
            }
        });
    }
}
