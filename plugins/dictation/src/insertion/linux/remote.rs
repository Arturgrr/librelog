//! The paste keys through the desktop's RemoteDesktop portal (GNOME, KDE):
//! a keyboard-only session the user approves once. The portal's restore
//! token, kept in Librelog's data folder, restores that approval without a
//! dialog. GNOME shows its remote-control indicator while a session is
//! open, so one closes a while after the last paste and the next restores.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::{PersistMode, Session};

use super::route::Chord;

const TOKEN_FILE: &str = "remote-desktop-token";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Starting can wait for the user to answer the approval dialog.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE: Duration = Duration::from_secs(15);
/// A dialog takes focus; give it back to the app before the keys.
const DIALOG_FOCUS_SETTLE: Duration = Duration::from_millis(250);

const XK_CONTROL_L: i32 = 0xffe3;
const XK_SHIFT_L: i32 = 0xffe1;
const XK_V: i32 = 0x76;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static STATE: tokio::sync::Mutex<State> = tokio::sync::Mutex::const_new(State {
    open: None,
    refused: None,
    uses: 0,
});

struct State {
    open: Option<Open>,
    /// Why the desktop refused this run: the user declined, or there is no
    /// portal. Not asked again until Librelog restarts.
    refused: Option<String>,
    uses: u64,
}

struct Open {
    portal: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

/// Opens the session ahead of a paste, showing the approval dialog the
/// first time, so the paste itself does not wait for it.
pub fn prepare() {
    tauri::async_runtime::spawn(async {
        let mut state = STATE.lock().await;
        if let Err(error) = state.ensure().await {
            tracing::info!(%error, "the RemoteDesktop portal is not available for pasting");
            return;
        }
        state.uses += 1;
        close_when_idle(state.uses);
    });
}

/// Presses the paste chord, opening (or restoring) the session first.
pub async fn send(chord: Chord) -> Result<(), String> {
    let mut state = STATE.lock().await;
    let prompted = state.ensure().await?;
    if prompted {
        tokio::time::sleep(DIALOG_FOCUS_SETTLE).await;
    }
    let mut result = state.press(chord).await;
    if result.is_err() {
        // The user may have stopped it from the desktop's indicator.
        state.close().await;
        state.ensure().await?;
        result = state.press(chord).await;
    }
    state.uses += 1;
    close_when_idle(state.uses);
    result
}

fn close_when_idle(uses: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE).await;
        let mut state = STATE.lock().await;
        if state.uses == uses {
            state.close().await;
        }
    });
}

impl State {
    /// Whether this opened a session that may have shown a dialog.
    async fn ensure(&mut self) -> Result<bool, String> {
        if self.open.is_some() {
            return Ok(false);
        }
        if let Some(reason) = &self.refused {
            return Err(reason.clone());
        }
        let started = Instant::now();
        match tokio::time::timeout(APPROVAL_TIMEOUT, start()).await {
            Ok(Ok((open, restored))) => {
                self.open = Some(open);
                Ok(!restored || started.elapsed() > Duration::from_millis(500))
            }
            Ok(Err(Failure::Refused(reason))) => {
                self.refused = Some(reason.clone());
                Err(reason)
            }
            Ok(Err(Failure::Failed(reason))) => Err(reason),
            Err(_) => Err("the desktop's remote control approval timed out".into()),
        }
    }

    async fn press(&self, chord: Chord) -> Result<(), String> {
        let open = self.open.as_ref().ok_or("no RemoteDesktop session")?;
        let mut pressed = Vec::new();
        let mut result = Ok(());
        for (keysym, state) in chord_keys(chord) {
            if state == KeyState::Pressed {
                pressed.push(keysym);
            }
            let sent = tokio::time::timeout(
                CALL_TIMEOUT,
                open.portal.notify_keyboard_keysym(
                    &open.session,
                    keysym,
                    state,
                    Default::default(),
                ),
            )
            .await;
            match sent {
                Ok(Ok(())) => {
                    pressed.retain(|held| !(state == KeyState::Released && *held == keysym))
                }
                Ok(Err(error)) => {
                    result = Err(error.to_string());
                    break;
                }
                Err(_) => {
                    result = Err("the RemoteDesktop portal did not answer".into());
                    break;
                }
            }
        }
        // A key left down would stay down for every app.
        for keysym in pressed.into_iter().rev() {
            let _ = tokio::time::timeout(
                CALL_TIMEOUT,
                open.portal.notify_keyboard_keysym(
                    &open.session,
                    keysym,
                    KeyState::Released,
                    Default::default(),
                ),
            )
            .await;
        }
        result
    }

    async fn close(&mut self) {
        if let Some(open) = self.open.take() {
            let _ = tokio::time::timeout(CALL_TIMEOUT, open.session.close()).await;
        }
    }
}

enum Failure {
    Refused(String),
    Failed(String),
}

/// A started keyboard session, and whether a saved grant restored it.
async fn start() -> Result<(Open, bool), Failure> {
    let failed = |error: &dyn std::fmt::Display| Failure::Failed(error.to_string());
    let connection = anlg_desktop_portal::connection()
        .await
        .map_err(|e| failed(&e))?;
    let portal = RemoteDesktop::with_connection(connection)
        .await
        .map_err(|error| {
            Failure::Refused(format!("the desktop has no RemoteDesktop portal: {error}"))
        })?;
    let session = portal
        .create_session(Default::default())
        .await
        .map_err(|e| failed(&e))?;
    let persists = portal.version() >= 2;
    let token = persists.then(load_token).flatten();
    let mut options = SelectDevicesOptions::default()
        .set_devices(ashpd::enumflags2::BitFlags::from_flag(DeviceType::Keyboard));
    if persists {
        options = options
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
    }
    let selected = async {
        portal.select_devices(&session, options).await?.response()?;
        portal
            .start(&session, None, Default::default())
            .await?
            .response()
    }
    .await;
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => {
            let _ = session.close().await;
            return Err(match error {
                ashpd::Error::Response(_) => {
                    Failure::Refused(format!("the desktop did not allow remote control: {error}"))
                }
                error => failed(&error),
            });
        }
    };
    if !selected.devices().contains(DeviceType::Keyboard) {
        let _ = session.close().await;
        return Err(Failure::Refused(
            "the desktop did not share the keyboard".into(),
        ));
    }
    // Restore tokens are single-use: each start hands out the next one.
    if let Some(next) = selected.restore_token()
        && let Err(error) = save_token(next)
    {
        tracing::warn!(%error, "could not keep the remote control approval");
    }
    Ok((Open { portal, session }, token.is_some()))
}

fn chord_keys(chord: Chord) -> Vec<(i32, KeyState)> {
    let shift = chord == Chord::TerminalPaste;
    let mut keys = vec![(XK_CONTROL_L, KeyState::Pressed)];
    if shift {
        keys.push((XK_SHIFT_L, KeyState::Pressed));
    }
    keys.extend([(XK_V, KeyState::Pressed), (XK_V, KeyState::Released)]);
    if shift {
        keys.push((XK_SHIFT_L, KeyState::Released));
    }
    keys.push((XK_CONTROL_L, KeyState::Released));
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

fn save_token(token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let path = token_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no data folder"))?;
    std::fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{TOKEN_FILE}.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, &path)
}
