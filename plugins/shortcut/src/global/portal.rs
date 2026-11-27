use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use ashpd::desktop::ResponseError;
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tauri::Manager;
use tauri_specta::Event;

use super::Action;
use super::bindings::{Binding, Does, RELEASE_WAIT, ReleaseGate, stale};

use crate::{
    AnnotateShortcutEvent, CaptureShortcutEvent, DictationPresetEvent, PasteLastAnnotationEvent,
    PasteLastEvent, ShortcutEvent, annotate,
    press::{self, TapLock},
};

const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the desktop's approval dialog may stay open. Giving up closes
/// the session, and KDE then keeps every shortcut with no key and never asks
/// again.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub enum Failure {
    /// The user dismissed the desktop's dialog.
    Dismissed,
    Failed(String),
}

pub struct Registration {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tauri::async_runtime::JoinHandle<()>,
    ended: Arc<AtomicBool>,
}

impl Registration {
    /// Closes the session. Every shortcut still down is released first, so
    /// a hold in progress (dictation, Annotate's ink) ends as if let go.
    pub async fn close(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }

    /// The desktop closed the session, or it could not be watched.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }
}

/// The trigger the desktop reports for each id, when it says.
pub type Triggers = HashMap<String, String>;

/// Binds `bindings` in a new portal session and acts on each activation and
/// deactivation until the registration closes or the desktop ends the
/// session, which calls `on_end`. `on_triggers` hears the keys the user
/// later changes in the desktop's settings.
pub async fn bind(
    app: tauri::AppHandle,
    bindings: Vec<Binding>,
    on_triggers: impl Fn(Triggers) + Send + 'static,
    on_end: impl FnOnce() + Send + 'static,
) -> Result<(Registration, Triggers), Failure> {
    let failed =
        |what: &str, error: &dyn std::fmt::Display| Failure::Failed(format!("{what}: {error}"));
    let shortcuts: Vec<NewShortcut> = bindings
        .iter()
        .map(|binding| {
            NewShortcut::new(binding.id.clone(), binding.description.clone())
                .preferred_trigger(binding.trigger.as_str())
        })
        .collect();
    let ids: Vec<String> = bindings.iter().map(|binding| binding.id.clone()).collect();
    let does: HashMap<String, Does> = bindings
        .into_iter()
        .map(|binding| (binding.id, binding.does))
        .collect();

    let connection = anlg_desktop_portal::connection().await.map_err(|e| {
        failed(
            "Librelog could not reach the desktop portals for global shortcuts",
            &e,
        )
    })?;
    let portal = GlobalShortcuts::with_connection(connection)
        .await
        .map_err(|e| {
            failed(
                "This Wayland desktop does not provide the Global Shortcuts portal",
                &e,
            )
        })?;
    let session = tokio::time::timeout(CALL_TIMEOUT, portal.create_session(Default::default()))
        .await
        .map_err(|_| {
            Failure::Failed("The desktop's Global Shortcuts portal did not answer.".into())
        })?
        .map_err(|e| failed("The desktop refused a shortcuts session", &e))?;
    let streams = tokio::time::timeout(CALL_TIMEOUT, async {
        Ok::<_, ashpd::Error>((
            portal.receive_activated().await?,
            portal.receive_deactivated().await?,
            portal.receive_shortcuts_changed().await?,
        ))
    })
    .await;
    let (mut pressed, mut released, mut changed) = match streams {
        Ok(Ok(streams)) => streams,
        Ok(Err(error)) => {
            let _ = session.close().await;
            return Err(failed(
                "The desktop's Global Shortcuts portal failed",
                &error,
            ));
        }
        Err(_) => {
            let _ = session.close().await;
            return Err(Failure::Failed(
                "The desktop's Global Shortcuts portal did not answer.".into(),
            ));
        }
    };
    // KWin raises the approval dialog above Librelog's window, and gives
    // it the focus, only when it knows whose it is.
    let parent = main_window(&app);
    let bound = tokio::time::timeout(APPROVAL_TIMEOUT, async {
        portal
            .bind_shortcuts(&session, &shortcuts, parent.as_ref(), Default::default())
            .await?
            .response()
    })
    .await;
    let bound = match bound {
        Ok(Ok(bound)) => bound,
        Ok(Err(error)) => {
            let _ = session.close().await;
            return Err(match error {
                ashpd::Error::Response(ResponseError::Cancelled) => Failure::Dismissed,
                error => failed("The desktop did not bind Librelog's shortcuts", &error),
            });
        }
        Err(_) => {
            let _ = session.close().await;
            return Err(Failure::Failed(
                "Your desktop's shortcut prompt stayed unanswered. Ask your desktop again.".into(),
            ));
        }
    };
    let bound_triggers = triggers(bound.shortcuts());
    tauri::async_runtime::spawn(forget_stale(ids));
    let session_path: String =
        serde_json::from_value(serde_json::to_value(&session).map_err(|e| failed("session", &e))?)
            .map_err(|e| failed("session", &e))?;
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let ended = Arc::new(AtomicBool::new(false));
    let handler = Handler {
        does,
        gates: Default::default(),
        down: Default::default(),
        locks: Default::default(),
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
                                if let Some(token) = activation_token(event.options()) {
                                    anlg_desktop_portal::set_activation_token(token);
                                }
                                handler.on(&app, event.shortcut_id(), true);
                            }
                        }
                        event = released.next() => {
                            let Some(event) = event else { break false };
                            if event.session_handle().as_str() == session_path {
                                anlg_desktop_portal::note_shortcut_release();
                                handler.on(&app, event.shortcut_id(), false);
                            }
                        }
                        event = changed.next() => {
                            let Some(event) = event else { break false };
                            if event.session_handle().as_str() == session_path {
                                on_triggers(triggers(event.shortcuts()));
                            }
                        }
                    }
                },
            };
            handler.release_all(&app);
            let _ = tokio::time::timeout(CALL_TIMEOUT, session.close()).await;
            ended.store(true, Ordering::SeqCst);
            app.state::<annotate::AnnotateGesture>().release_key();
            if !deliberate {
                tracing::warn!("the desktop closed Librelog's global shortcuts session");
                press::cancel(&app);
                on_end();
            }
        }
    });
    Ok((Registration { stop, task, ended }, bound_triggers))
}

/// Librelog's main window, when it shows: a dialog parented to a hidden or
/// minimized window would be hidden with it.
fn main_window(app: &tauri::AppHandle) -> Option<ashpd::WindowIdentifier> {
    let window = app.get_webview_window("main")?;
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(true) {
        return None;
    }
    anlg_desktop_portal::parent(&window)
}

/// The token an `Activated` signal carries (portal version 2), for the
/// window the shortcut opens.
fn activation_token(
    options: &HashMap<String, ashpd::zbus::zvariant::OwnedValue>,
) -> Option<String> {
    let token = String::try_from(options.get("activation_token")?.try_clone().ok()?).ok()?;
    (!token.is_empty()).then_some(token)
}

/// KDE keeps every shortcut id a session ever bound, listed under Librelog
/// in System Settings, and a shortcut changed in Librelog's Settings is a
/// new id. Once a session is bound, the ids it no longer has are removed
/// from kglobalaccel, under the component the portal named after Librelog's
/// app id.
async fn forget_stale(bound: Vec<String>) {
    if !super::trigger::kde() {
        return;
    }
    let Some(component) = anlg_desktop_portal::app_id() else {
        return;
    };
    match anlg_desktop_portal::kde_shortcut_ids(&component).await {
        Ok(known) => {
            for id in stale(&known, &bound) {
                match anlg_desktop_portal::forget_kde_shortcut(&component, &id).await {
                    Ok(()) => tracing::info!(id, "removed a shortcut Librelog no longer binds"),
                    Err(error) => tracing::debug!(%error, id, "could not remove a stale shortcut"),
                }
            }
        }
        Err(error) => tracing::debug!(%error, "kglobalaccel did not list Librelog's shortcuts"),
    }
}

fn triggers(shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut]) -> Triggers {
    shortcuts
        .iter()
        .map(|shortcut| {
            tracing::info!(
                id = shortcut.id(),
                trigger = shortcut.trigger_description(),
                "the desktop bound a Librelog shortcut"
            );
            (
                shortcut.id().to_owned(),
                shortcut.trigger_description().to_owned(),
            )
        })
        .collect()
}

struct Handler {
    does: HashMap<String, Does>,
    gates: Arc<Mutex<HashMap<String, ReleaseGate>>>,
    /// Shortcuts activated and not yet deactivated.
    down: Mutex<HashSet<String>>,
    /// Each dictation shortcut's double-tap lock.
    locks: Mutex<HashMap<String, TapLock>>,
}

impl Handler {
    fn on(&self, app: &tauri::AppHandle, id: &str, pressed: bool) {
        let Some(does) = self.does.get(id).copied() else {
            return;
        };
        {
            let mut down = self.down.lock().unwrap_or_else(|e| e.into_inner());
            if pressed {
                down.insert(id.to_owned());
            } else {
                down.remove(id);
            }
            anlg_desktop_portal::set_shortcuts_held(down.len());
        }
        let action = match does {
            Does::Cancel => {
                if pressed {
                    self.locks.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
                let passes = {
                    let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
                    let lock = locks.entry(id.to_owned()).or_default();
                    if pressed {
                        lock.press();
                        true
                    } else {
                        lock.release(std::time::Instant::now())
                    }
                };
                if !passes {
                    // Locked: the dictation goes on, and the next press and
                    // release of a dictation shortcut end it.
                    press::forget(app, source);
                    return;
                }
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
            Action::SwitchPreset if pressed => {
                let _ = DictationPresetEvent { preset: None }.emit(app);
            }
            Action::Capture(_) | Action::SwitchPreset | Action::Cancel | Action::AnnotatePen => {}
        }
    }

    /// The session is going: what is still down is let go, as the desktop
    /// will not say so any more.
    fn release_all(&self, app: &tauri::AppHandle) {
        // Nothing still down may lock a dictation now.
        self.locks.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let down: Vec<String> = self
            .down
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect();
        for id in down {
            self.on(app, &id, false);
        }
        anlg_desktop_portal::set_shortcuts_held(0);
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
