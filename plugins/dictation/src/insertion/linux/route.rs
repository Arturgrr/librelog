//! How dictated text reaches the focused app on Linux, decided from what
//! AT-SPI saw and the session, so it can be tested without a desktop.

use std::path::{Path, PathBuf};

use super::accessible::Focus;

pub const PASSWORD: &str = "Dictation is unavailable in password fields.";

/// An app AT-SPI names, with its process when the bus knows it.
#[derive(Clone, Debug, Default)]
pub struct App {
    pub name: String,
    pub pid: Option<u32>,
    pub executable: Option<PathBuf>,
}

/// Where a dictation goes, read when it starts and again right before it is
/// inserted.
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "route", rename_all = "camelCase")]
pub enum Target {
    /// Through AT-SPI's `EditableText` into this field, or by pasting if it
    /// does not take the text.
    Accessible { object: String },
    /// Clipboard and the paste keys, `terminal` choosing Control+Shift+V.
    Paste { terminal: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    /// Control+V.
    Paste,
    /// Control+Shift+V: Control+V in a terminal is a literal next character.
    TerminalPaste,
}

impl Chord {
    pub fn for_terminal(terminal: bool) -> Self {
        if terminal {
            Self::TerminalPaste
        } else {
            Self::Paste
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Terminal,
    /// Chromium, Electron and Gecko accept AT-SPI edits they never apply,
    /// and web content may apply them without telling the page's editor:
    /// they are never asked.
    Web,
    Native,
}

/// Terminal emulators by executable, or by the name they give AT-SPI
/// when a script interpreter runs them (Terminator, Guake).
const TERMINALS: &[&str] = &[
    "alacritty",
    "blackbox",
    "contour",
    "cool-retro-term",
    "deepin-terminal",
    "foot",
    "footclient",
    "ghostty",
    "gnome-terminal",
    "gnome-terminal-server",
    "guake",
    "kgx",
    "kitty",
    "konsole",
    "lxterminal",
    "mate-terminal",
    "ptyxis",
    "ptyxis-agent",
    "qterminal",
    "rio",
    "sakura",
    "st",
    "terminator",
    "terminology",
    "tilda",
    "tilix",
    "urxvt",
    "warp",
    "warp-terminal",
    "wezterm",
    "wezterm-gui",
    "xfce4-terminal",
    "xterm",
    "yakuake",
];

/// Chromium and Electron ship these next to their executable, Gecko
/// `libxul.so`.
const WEB_ENGINE_FILES: &[&str] = &[
    "chrome_100_percent.pak",
    "v8_context_snapshot.bin",
    "libxul.so",
];

fn kind(app: &App, own_pid: u32, exists: &impl Fn(&Path) -> bool) -> Kind {
    let executable = app
        .executable
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_lowercase());
    let name = app.name.to_lowercase();
    if executable
        .iter()
        .chain([&name])
        .any(|name| TERMINALS.contains(&name.as_str()))
    {
        return Kind::Terminal;
    }
    // Librelog's own editor is web content too.
    if app.pid == Some(own_pid) {
        return Kind::Web;
    }
    let folder = app.executable.as_deref().and_then(Path::parent);
    if folder.is_some_and(|folder| {
        WEB_ENGINE_FILES
            .iter()
            .any(|file| exists(&folder.join(file)))
    }) {
        return Kind::Web;
    }
    Kind::Native
}

/// The route for what AT-SPI saw; `active` is the active window's app by
/// other means (X11), for when AT-SPI knows of none. Only a password field
/// refuses: everything else that is not an editable native field pastes.
pub fn target(
    focus: Focus,
    active: impl FnOnce() -> Option<App>,
    own_pid: u32,
    exists: impl Fn(&Path) -> bool,
) -> Result<Target, String> {
    let terminal = |app: &App| kind(app, own_pid, &exists) == Kind::Terminal;
    Ok(match focus {
        Focus::Password => return Err(PASSWORD.into()),
        Focus::Editable { object, app } => match kind(&app, own_pid, &exists) {
            Kind::Native => Target::Accessible {
                object: serde_json::to_string(&object).map_err(|e| e.to_string())?,
            },
            kind => Target::Paste {
                terminal: kind == Kind::Terminal,
            },
        },
        Focus::Other { app } => Target::Paste {
            terminal: terminal(&app),
        },
        Focus::Unknown { app } => Target::Paste {
            terminal: app.or_else(active).is_some_and(|app| terminal(&app)),
        },
    })
}

/// How the paste keys are sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPath {
    /// The desktop's RemoteDesktop portal (GNOME, KDE): approved once.
    Portal,
    /// `wtype`, on compositors with the virtual keyboard protocol (wlroots).
    Wtype,
    /// XTest, in an X11 session.
    XTest,
}

/// The session as the environment describes it. Librelog may be an X11
/// client through XWayland in a Wayland session: it is the session that
/// decides, since XTest from XWayland reaches only other X11 apps.
pub struct Session<'a> {
    pub wayland_display: Option<&'a str>,
    pub x11_display: Option<&'a str>,
    pub current_desktop: Option<&'a str>,
}

/// The paths to try, in order. GNOME and KDE lack the virtual keyboard
/// protocol `wtype` needs.
pub fn key_paths(session: &Session<'_>, wtype_installed: bool) -> Vec<KeyPath> {
    let set = |value: Option<&str>| value.is_some_and(|value| !value.is_empty());
    if set(session.wayland_display) {
        let desktops = session.current_desktop.unwrap_or_default().to_lowercase();
        let portal_only = desktops
            .split(':')
            .any(|desktop| matches!(desktop, "gnome" | "kde" | "unity" | "budgie" | "cinnamon"));
        return if wtype_installed && !portal_only {
            vec![KeyPath::Wtype, KeyPath::Portal]
        } else {
            vec![KeyPath::Portal]
        };
    }
    if set(session.x11_display) {
        return vec![KeyPath::XTest];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, executable: &str) -> App {
        App {
            name: name.into(),
            pid: Some(10),
            executable: Some(executable.into()),
        }
    }

    fn object() -> atspi::ObjectRef {
        serde_json::from_value(serde_json::json!([":1.5", "/org/a11y/atspi/accessible/7"])).unwrap()
    }

    fn route(focus: Focus) -> Result<Target, String> {
        target(
            focus,
            || None,
            1,
            |path| path == Path::new("/usr/share/code/chrome_100_percent.pak"),
        )
    }

    /// AT-SPI used to be the only path: a terminal, an Electron app or an app
    /// without an editable accessible field refused the dictation.
    #[test]
    fn only_editable_native_fields_take_at_spi_and_only_passwords_refuse() {
        assert!(matches!(
            route(Focus::Editable {
                object: object(),
                app: app("gedit", "/usr/bin/gnome-text-editor")
            }),
            Ok(Target::Accessible { .. })
        ));
        for (focus, terminal) in [
            (
                Focus::Editable {
                    object: object(),
                    app: app("Code", "/usr/share/code/code"),
                },
                false,
            ),
            (
                Focus::Editable {
                    object: object(),
                    app: App {
                        pid: Some(1),
                        ..app("Librelog", "/usr/bin/librelog")
                    },
                },
                false,
            ),
            (
                Focus::Other {
                    app: app("Terminal", "/usr/libexec/gnome-terminal-server"),
                },
                true,
            ),
            // Terminator runs in Python and names itself to AT-SPI.
            (
                Focus::Other {
                    app: app("terminator", "/usr/bin/python3.13"),
                },
                true,
            ),
            (
                Focus::Other {
                    app: app("Files", "/usr/bin/nautilus"),
                },
                false,
            ),
            (
                Focus::Unknown {
                    app: Some(app("kitty", "/usr/bin/kitty")),
                },
                true,
            ),
            (Focus::Unknown { app: None }, false),
        ] {
            assert_eq!(route(focus), Ok(Target::Paste { terminal }));
        }
        assert_eq!(route(Focus::Password), Err(PASSWORD.into()));

        // X11 names the active window's app when AT-SPI cannot.
        let target = target(
            Focus::Unknown { app: None },
            || Some(app("", "/usr/bin/alacritty")),
            1,
            |_| false,
        );
        assert_eq!(target, Ok(Target::Paste { terminal: true }));
    }

    #[test]
    fn the_session_not_librelogs_own_backend_picks_how_keys_are_sent() {
        let session = |wayland, x11, desktop| Session {
            wayland_display: wayland,
            x11_display: x11,
            current_desktop: desktop,
        };
        let gnome = session(Some("wayland-0"), Some(":0"), Some("GNOME"));
        assert_eq!(key_paths(&gnome, true), [KeyPath::Portal]);
        let kde = session(Some("wayland-0"), None, Some("KDE"));
        assert_eq!(key_paths(&kde, false), [KeyPath::Portal]);
        let sway = session(Some("wayland-1"), Some(":1"), Some("sway"));
        assert_eq!(key_paths(&sway, true), [KeyPath::Wtype, KeyPath::Portal]);
        assert_eq!(key_paths(&sway, false), [KeyPath::Portal]);
        let x11 = session(None, Some(":0"), Some("GNOME"));
        assert_eq!(key_paths(&x11, true), [KeyPath::XTest]);
        assert!(key_paths(&session(None, None, None), true).is_empty());
    }
}
