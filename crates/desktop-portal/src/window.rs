//! What a portal dialog and a window opened from a shortcut need from the
//! compositor: a parent window, and the activation token the shortcut came
//! with.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The window a portal dialog belongs to, so KWin and Mutter keep it above
/// Librelog's window and give it focus: `x11:<xid>` under XWayland, where
/// Librelog's windows run by default. A native Wayland window would need an
/// xdg-foreign handle, which GTK 3 does not give out here; its dialogs go
/// without a parent, as before.
pub fn parent(window: &impl HasWindowHandle) -> Option<ashpd::WindowIdentifier> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => Some(ashpd::WindowIdentifier::from_xid(handle.window)),
        RawWindowHandle::Xcb(handle) => Some(ashpd::WindowIdentifier::from_xid(
            handle.window.get().into(),
        )),
        _ => None,
    }
}

/// How long a shortcut's activation token is worth using: the window it is
/// for opens right after the press, and compositors expire tokens soon.
const TOKEN_LIFETIME: Duration = Duration::from_secs(10);

static TOKEN: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// The activation token the desktop sent with a shortcut's press (the
/// Global Shortcuts portal's `activation_token`).
pub fn set_activation_token(token: String) {
    *TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some((token, Instant::now()));
}

/// The last shortcut's activation token, once, while it is fresh. A window
/// opened in answer to the shortcut presents it so KWin's focus-stealing
/// prevention lets it take the focus.
pub fn take_activation_token() -> Option<String> {
    TOKEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .filter(|(_, at)| at.elapsed() <= TOKEN_LIFETIME)
        .map(|(token, _)| token)
}
