//! The paste keys through the desktop's RemoteDesktop portal (GNOME, KDE):
//! a keyboard-only session the user approves once. The portal's restore
//! token, kept in Librelog's data folder, restores that approval without a
//! dialog; KDE may instead remember it itself, and then a new session opens
//! at once without a token. GNOME and KDE show a remote-control indicator
//! while a session is open, so a session that reopens without a dialog
//! closes a while after the last paste. One that would need the dialog again
//! stays open on GNOME. On KDE, whose portal may give no token, it closes
//! after a long idle so the indicator does not stay for the whole run, and
//! the next dictation asks again while the user speaks. Keys go by keysym,
//! and by keycode when the active layout has no key for a keysym.
//!
//! The approval dialog can stay open as long as the user leaves it. It runs
//! on its own task: an insertion waits a moment for it and otherwise leaves
//! the text on the clipboard, while the approval stays pending for the next
//! paste.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, ResponseError, Session};

use super::route::Chord;

const TOKEN_FILE: &str = "remote-desktop-token";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the approval dialog may stay open.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long an insertion waits for a session that is still starting.
const INSERTION_WAIT: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_secs(15);
/// How long KDE keeps a session that needs the dialog again, and its
/// indicator, after the last paste.
const KDE_DIALOG_IDLE: Duration = Duration::from_secs(10 * 60);
/// A start that took longer showed the approval dialog.
const NO_DIALOG: Duration = Duration::from_millis(500);
/// A dialog takes focus; give it back to the app before the keys.
const DIALOG_FOCUS_SETTLE: Duration = Duration::from_millis(250);

const XK_CONTROL_L: i32 = 0xffe3;
const XK_SHIFT_L: i32 = 0xffe1;
const XK_V: i32 = 0x76;
const XK_RETURN: i32 = 0xff0d;
/// Linux input event codes, for a keyboard layout where the keysyms find no
/// key (Russian, Greek): the keys where V and Return are on a US layout.
const KEY_LEFTCTRL: i32 = 29;
const KEY_LEFTSHIFT: i32 = 42;
const KEY_V: i32 = 47;
const KEY_ENTER: i32 = 28;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static STATE: std::sync::Mutex<State> = std::sync::Mutex::new(State {
    open: None,
    refused: None,
    starting: false,
    prompted: false,
    silent: false,
    uses: 0,
});
static STARTED: tokio::sync::Notify = tokio::sync::Notify::const_new();

struct State {
    open: Option<Arc<Open>>,
    /// Why the desktop refused this run: the user declined, or there is no
    /// portal. Not asked again until Librelog restarts.
    refused: Option<String>,
    starting: bool,
    /// The session that just opened may have shown a dialog.
    prompted: bool,
    /// A session opened this run without a token and without a dialog: the
    /// desktop remembers the approval itself (KDE).
    silent: bool,
    uses: u64,
}

struct Open {
    portal: RemoteDesktop,
    session: Session<RemoteDesktop>,
    /// A later session opens without a dialog: a restore token came back,
    /// or the desktop remembers the approval.
    restorable: bool,
}

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Opens the session ahead of a paste, showing the approval dialog the
/// first time, so the paste itself does not wait for it.
pub fn prepare() {
    begin();
}

/// Starts opening a session unless one is open, opening or refused.
fn begin() {
    {
        let mut state = state();
        if state.open.is_some() || state.refused.is_some() || state.starting {
            return;
        }
        state.starting = true;
    }
    tauri::async_runtime::spawn(async {
        let started = Instant::now();
        let outcome = start().await;
        let mut state = state();
        state.starting = false;
        match outcome {
            Ok((mut open, restored)) => {
                let dialog = started.elapsed() > NO_DIALOG;
                state.prompted = !restored || dialog;
                if !open.restorable && !restored && !dialog {
                    open.restorable = true;
                    state.silent = true;
                }
                let idle = idle_close(open.restorable, crate::kwin::on_kde());
                state.open = Some(Arc::new(open));
                state.uses += 1;
                if let Some(idle) = idle {
                    close_when_idle(state.uses, idle);
                }
            }
            Err(Failure::Refused(reason)) => {
                tracing::info!(%reason, "the desktop refused remote control for pasting");
                state.refused = Some(reason);
            }
            Err(Failure::Failed(reason)) => {
                tracing::warn!(%reason, "the RemoteDesktop portal failed; it is asked again at the next paste");
            }
        }
        drop(state);
        STARTED.notify_waiters();
    });
}

/// The open session, waiting up to `wait` for one that is starting.
async fn session(wait: Duration) -> Result<(Arc<Open>, bool), String> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let started = STARTED.notified();
        {
            let mut state = state();
            if let Some(open) = &state.open {
                return Ok((open.clone(), std::mem::take(&mut state.prompted)));
            }
            if let Some(reason) = &state.refused {
                return Err(reason.clone());
            }
        }
        begin();
        if tokio::time::timeout_at(deadline, started).await.is_err() {
            return Err("the desktop's remote control approval is still pending".into());
        }
    }
}

/// Presses the paste chord, opening (or restoring) the session first.
pub async fn send(chord: Chord) -> Result<(), String> {
    let (open, prompted) = session(INSERTION_WAIT).await?;
    if prompted {
        tokio::time::sleep(DIALOG_FOCUS_SETTLE).await;
    }
    let mut result = press(open.as_ref(), chord).await;
    if result.is_err() {
        // The user may have stopped it from the desktop's indicator.
        forget(&open).await;
        let (open, _) = session(INSERTION_WAIT).await?;
        result = press(open.as_ref(), chord).await;
    }
    let mut state = state();
    state.uses += 1;
    let idle = state
        .open
        .as_ref()
        .and_then(|open| idle_close(open.restorable, crate::kwin::on_kde()));
    if let Some(idle) = idle {
        close_when_idle(state.uses, idle);
    }
    result
}

/// How long a session stays open after its last paste, while the desktop's
/// remote-control indicator shows. One that reopens without a dialog closes
/// soon. One that would need the dialog again stays for the app's lifetime
/// on GNOME, and on KDE, which may never hand out a token, a long while.
fn idle_close(restorable: bool, kde: bool) -> Option<Duration> {
    match (restorable, kde) {
        (true, _) => Some(IDLE),
        (false, true) => Some(KDE_DIALOG_IDLE),
        (false, false) => None,
    }
}

fn close_when_idle(uses: u64, idle: Duration) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(idle).await;
        let open = {
            let mut state = state();
            if state.uses != uses {
                return;
            }
            state.open.take()
        };
        if let Some(open) = open {
            let _ = tokio::time::timeout(CALL_TIMEOUT, open.session.close()).await;
        }
    });
}

async fn forget(open: &Arc<Open>) {
    let forgotten = {
        let mut state = state();
        match &state.open {
            Some(current) if Arc::ptr_eq(current, open) => state.open.take(),
            _ => None,
        }
    };
    if let Some(open) = forgotten {
        let _ = tokio::time::timeout(CALL_TIMEOUT, open.session.close()).await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Sym(i32),
    Code(i32),
}

/// Sends one key through the portal.
trait Injector {
    fn key(&self, key: Key, state: KeyState) -> impl Future<Output = Result<(), String>>;
}

impl Injector for Open {
    async fn key(&self, key: Key, state: KeyState) -> Result<(), String> {
        let sent = match key {
            Key::Sym(keysym) => {
                tokio::time::timeout(
                    CALL_TIMEOUT,
                    self.portal.notify_keyboard_keysym(
                        &self.session,
                        keysym,
                        state,
                        Default::default(),
                    ),
                )
                .await
            }
            Key::Code(keycode) => {
                tokio::time::timeout(
                    CALL_TIMEOUT,
                    self.portal.notify_keyboard_keycode(
                        &self.session,
                        keycode,
                        state,
                        Default::default(),
                    ),
                )
                .await
            }
        };
        match sent {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("the RemoteDesktop portal did not answer".into()),
        }
    }
}

/// Presses the chord by keysym, then by keycode when the desktop finds no
/// key for a keysym in the active layout.
async fn press(keys: &impl Injector, chord: Chord) -> Result<(), String> {
    let Err(keysym_error) = press_keys(keys, chord_keys(chord, false)).await else {
        return Ok(());
    };
    press_keys(keys, chord_keys(chord, true))
        .await
        .map_err(|keycode_error| format!("{keysym_error}; by keycode: {keycode_error}"))
}

async fn press_keys(keys: &impl Injector, chord: Vec<(Key, KeyState)>) -> Result<(), String> {
    let mut pressed = Vec::new();
    let mut result = Ok(());
    for (key, state) in chord {
        match keys.key(key, state).await {
            Ok(()) if state == KeyState::Pressed => pressed.push(key),
            Ok(()) => pressed.retain(|held| *held != key),
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }
    // A key left down would stay down for every app.
    for key in pressed.into_iter().rev() {
        let _ = keys.key(key, KeyState::Released).await;
    }
    result
}

#[derive(Debug, PartialEq)]
enum Failure {
    /// Remembered until Librelog restarts.
    Refused(String),
    /// Tried again at the next paste.
    Failed(String),
}

/// The user dismissing the dialog is remembered; a backend failure
/// (`ResponseError::Other`) or a broken call is not.
fn classify(error: &ashpd::Error) -> Failure {
    match error {
        ashpd::Error::Response(ResponseError::Cancelled) => {
            Failure::Refused(format!("the desktop did not allow remote control: {error}"))
        }
        ashpd::Error::PortalNotFound(_) => {
            Failure::Refused(format!("the desktop has no RemoteDesktop portal: {error}"))
        }
        error => Failure::Failed(error.to_string()),
    }
}

/// A started keyboard session, and whether a saved grant restored it.
async fn start() -> Result<(Open, bool), Failure> {
    let connection = anlg_desktop_portal::connection()
        .await
        .map_err(|e| Failure::Failed(e.to_string()))?;
    let portal = RemoteDesktop::with_connection(connection)
        .await
        .map_err(|error| classify(&error))?;
    let session = tokio::time::timeout(CALL_TIMEOUT, portal.create_session(Default::default()))
        .await
        .map_err(|_| Failure::Failed("the RemoteDesktop portal did not answer".into()))?
        .map_err(|error| classify(&error))?;
    let persists = portal.version() >= 2;
    let token = persists.then(load_token).flatten();
    let mut options = SelectDevicesOptions::default()
        .set_devices(ashpd::enumflags2::BitFlags::from_flag(DeviceType::Keyboard));
    if persists {
        options = options
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
    }
    let selected = tokio::time::timeout(APPROVAL_TIMEOUT, async {
        portal.select_devices(&session, options).await?.response()?;
        portal
            .start(&session, None, Default::default())
            .await?
            .response()
    })
    .await;
    let selected = match selected {
        Ok(Ok(selected)) => selected,
        Ok(Err(error)) => {
            let _ = tokio::time::timeout(CALL_TIMEOUT, session.close()).await;
            return Err(classify(&error));
        }
        Err(_) => {
            let _ = tokio::time::timeout(CALL_TIMEOUT, session.close()).await;
            return Err(Failure::Failed(
                "the remote control approval went unanswered".into(),
            ));
        }
    };
    if !selected.devices().contains(DeviceType::Keyboard) {
        let _ = tokio::time::timeout(CALL_TIMEOUT, session.close()).await;
        return Err(Failure::Refused(
            "the desktop did not share the keyboard".into(),
        ));
    }
    // Restore tokens are single-use: each start hands out the next one.
    let restorable = match selected.restore_token() {
        Some(next) => match token_path().map(|path| save_token(&path, next)) {
            Some(Ok(())) => true,
            Some(Err(error)) => {
                tracing::warn!(%error, "could not keep the remote control approval");
                false
            }
            None => false,
        },
        None => false,
    };
    Ok((
        Open {
            portal,
            session,
            restorable,
        },
        token.is_some(),
    ))
}

/// Whether keys can go through the portal without its approval dialog: a session is open, or a
/// saved grant restores one.
pub fn approved() -> bool {
    let state = state();
    state.open.is_some() || (state.refused.is_none() && (state.silent || load_token().is_some()))
}

fn chord_keys(chord: Chord, by_keycode: bool) -> Vec<(Key, KeyState)> {
    let [control, shift, v, enter] = if by_keycode {
        [KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V, KEY_ENTER].map(Key::Code)
    } else {
        [XK_CONTROL_L, XK_SHIFT_L, XK_V, XK_RETURN].map(Key::Sym)
    };
    if chord == Chord::Return {
        return vec![(enter, KeyState::Pressed), (enter, KeyState::Released)];
    }
    let with_shift = chord == Chord::TerminalPaste;
    let mut keys = vec![(control, KeyState::Pressed)];
    if with_shift {
        keys.push((shift, KeyState::Pressed));
    }
    keys.extend([(v, KeyState::Pressed), (v, KeyState::Released)]);
    if with_shift {
        keys.push((shift, KeyState::Released));
    }
    keys.push((control, KeyState::Released));
    keys
}

fn token_path() -> Option<PathBuf> {
    Some(DATA_DIR.get()?.join(TOKEN_FILE))
}

fn load_token() -> Option<String> {
    let token = std::fs::read_to_string(token_path()?).ok()?;
    let token = token.trim();
    (!token.is_empty() && token.len() <= 1024).then(|| token.to_owned())
}

fn save_token(path: &Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no data folder"))?;
    std::fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{TOKEN_FILE}.{}", std::process::id()));
    // A file left by an earlier crash would keep its own permissions.
    match std::fs::remove_file(&staged) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, path)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A grant that cannot be restored used to close after 15 seconds too,
    /// so every later paste showed the dialog again; on KDE one that can't
    /// kept the remote-control indicator on for the whole run.
    #[test]
    fn only_a_restorable_session_closes_soon_and_kde_closes_the_others_later() {
        assert_eq!(idle_close(true, false), Some(IDLE));
        assert_eq!(idle_close(true, true), Some(IDLE));
        assert_eq!(idle_close(false, false), None);
        assert_eq!(idle_close(false, true), Some(KDE_DIALOG_IDLE));
    }

    /// A layout without V's keysym, as Russian or Greek may be for KWin.
    struct NoV(std::sync::Mutex<Vec<(Key, KeyState)>>);

    impl Injector for NoV {
        async fn key(&self, key: Key, state: KeyState) -> Result<(), String> {
            if key == Key::Sym(XK_V) {
                return Err("no key for keysym".into());
            }
            self.0.lock().unwrap().push((key, state));
            Ok(())
        }
    }

    /// On a Russian or Greek layout the desktop may find no key for V's
    /// keysym, and nothing pasted.
    #[tokio::test]
    async fn a_layout_without_the_keysym_gets_the_chord_by_keycode() {
        let keys = NoV(Default::default());
        press(&keys, Chord::TerminalPaste).await.unwrap();
        let sent = keys.0.lock().unwrap().clone();
        let (by_keysym, by_keycode) = sent.split_at(4);
        // Control and Shift went down by keysym and must come up again.
        assert_eq!(
            by_keysym,
            [
                (Key::Sym(XK_CONTROL_L), KeyState::Pressed),
                (Key::Sym(XK_SHIFT_L), KeyState::Pressed),
                (Key::Sym(XK_SHIFT_L), KeyState::Released),
                (Key::Sym(XK_CONTROL_L), KeyState::Released),
            ]
        );
        assert_eq!(by_keycode, chord_keys(Chord::TerminalPaste, true));
    }

    #[test]
    fn a_dismissed_dialog_is_remembered_and_a_backend_failure_is_not() {
        assert!(matches!(
            classify(&ashpd::Error::Response(ResponseError::Cancelled)),
            Failure::Refused(_)
        ));
        assert!(matches!(
            classify(&ashpd::Error::Response(ResponseError::Other)),
            Failure::Failed(_)
        ));
    }

    #[test]
    fn the_saved_token_is_owner_only_even_over_a_leftover_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TOKEN_FILE);
        let staged = dir
            .path()
            .join(format!(".{TOKEN_FILE}.{}", std::process::id()));
        std::fs::write(&staged, "old").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).unwrap();

        save_token(&path, "token").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "token");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
