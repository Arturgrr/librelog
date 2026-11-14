//! The real clipboard and the ways to send the paste keys.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arboard::SetExtLinux;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::wrapper::ConnectionExt as _;

use super::paste::{Clipboard, Keyboard, Snapshot};
use super::route::{App, Chord, KeyPath, Session};

/// Kept for the whole run: on X11 (and XWayland) the clipboard holds what
/// Librelog put there only while Librelog serves it.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

pub struct SystemClipboard<'a>(std::sync::MutexGuard<'a, Option<arboard::Clipboard>>);

impl SystemClipboard<'_> {
    pub fn open() -> Result<Self, String> {
        let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        Ok(Self(guard))
    }

    fn clipboard(&mut self) -> &mut arboard::Clipboard {
        self.0.as_mut().expect("opened")
    }
}

impl Clipboard for SystemClipboard<'_> {
    fn snapshot(&mut self) -> Snapshot {
        let clipboard = self.clipboard();
        if let Ok(text) = clipboard.get_text() {
            return Snapshot::Text(text);
        }
        match clipboard.get_image() {
            Ok(image) => Snapshot::Image {
                width: image.width,
                height: image.height,
                rgba: image.bytes.into_owned(),
            },
            // Empty, or a format arboard cannot read: either way nothing
            // can be put back.
            Err(_) => Snapshot::Unreadable,
        }
    }

    fn set_text(&mut self, text: &str) -> Result<(), String> {
        // Clipboard managers that honour the hint skip the transcript.
        self.clipboard()
            .set()
            .exclude_from_history()
            .text(text)
            .map_err(|e| e.to_string())
    }

    fn holds(&mut self, text: &str) -> bool {
        self.clipboard().get_text().is_ok_and(|now| now == text)
    }

    fn put_back(&mut self, snapshot: Snapshot) -> Result<(), String> {
        let clipboard = self.clipboard();
        match snapshot {
            Snapshot::Text(text) => clipboard.set_text(text),
            Snapshot::Image {
                width,
                height,
                rgba,
            } => clipboard.set_image(arboard::ImageData {
                width,
                height,
                bytes: rgba.into(),
            }),
            Snapshot::Unreadable => return Ok(()),
        }
        .map_err(|e| e.to_string())
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn key_paths() -> Vec<KeyPath> {
    let (wayland, x11, desktop) = (
        env("WAYLAND_DISPLAY"),
        env("DISPLAY"),
        env("XDG_CURRENT_DESKTOP"),
    );
    super::route::key_paths(
        &Session {
            wayland_display: wayland.as_deref(),
            x11_display: x11.as_deref(),
            current_desktop: desktop.as_deref(),
        },
        find_program("wtype").is_some(),
    )
}

/// Sends the chord through each path in turn until one works.
pub struct Keys(pub Vec<KeyPath>);

impl Keyboard for Keys {
    fn send(&mut self, chord: Chord) -> Result<(), String> {
        let mut errors = Vec::new();
        for path in &self.0 {
            let sent = match path {
                KeyPath::Portal => tauri::async_runtime::block_on(super::remote::send(chord)),
                KeyPath::Wtype => wtype(chord),
                KeyPath::XTest => xtest(chord),
            };
            match sent {
                Ok(()) => return Ok(()),
                Err(error) => errors.push(format!("{path:?}: {error}")),
            }
        }
        Err(if errors.is_empty() {
            "no way to send keys in this session".into()
        } else {
            errors.join("; ")
        })
    }
}

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn wtype(chord: Chord) -> Result<(), String> {
    let program = find_program("wtype").ok_or("wtype is not installed")?;
    let mut args = vec!["-M", "ctrl"];
    if chord == Chord::TerminalPaste {
        args.extend(["-M", "shift"]);
    }
    args.extend(["-k", "v"]);
    if chord == Chord::TerminalPaste {
        args.extend(["-m", "shift"]);
    }
    args.extend(["-m", "ctrl"]);
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            // The compositor lacks the virtual keyboard protocol.
            Some(status) => return Err(format!("wtype exited with {status}")),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("wtype did not finish".into());
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

const XK_CONTROL_L: u32 = 0xffe3;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_V: u32 = 0x76;
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
/// How long a paste waits for the shortcut's modifiers to come up.
const MODIFIER_WAIT: Duration = Duration::from_secs(2);

fn xtest(chord: Chord) -> Result<(), String> {
    let (connection, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = connection.setup().roots[screen].root;
    let keycode = |keysym| keycode(&connection, keysym);
    let control = keycode(XK_CONTROL_L)?;
    let shift = keycode(XK_SHIFT_L)?;
    let v = keycode(XK_V)?;
    wait_for_modifiers(&connection)?;
    let mut keys = vec![control];
    if chord == Chord::TerminalPaste {
        keys.push(shift);
    }
    keys.push(v);
    let fake = |kind, code| {
        connection
            .xtest_fake_input(kind, code, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .map(drop)
            .map_err(|e| e.to_string())
    };
    let mut result = Ok(());
    for &code in &keys {
        result = fake(KEY_PRESS, code);
        if result.is_err() {
            break;
        }
    }
    for &code in keys.iter().rev() {
        let released = fake(KEY_RELEASE, code);
        result = result.and(released);
    }
    connection.sync().map_err(|e| e.to_string())?;
    result
}

fn keycode(connection: &impl Connection, keysym: u32) -> Result<u8, String> {
    let setup = connection.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = connection
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let per = mapping.keysyms_per_keycode.max(1) as usize;
    mapping
        .keysyms
        .chunks(per)
        .position(|syms| syms.contains(&keysym))
        .map(|index| min + index as u8)
        .ok_or_else(|| format!("no key produces keysym {keysym:#x}"))
}

/// A Control+V sent while the user still holds Alt or Shift reaches the app
/// as another chord.
fn wait_for_modifiers(connection: &impl Connection) -> Result<(), String> {
    let modifiers = connection
        .get_modifier_mapping()
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .keycodes;
    let deadline = Instant::now() + MODIFIER_WAIT;
    while Instant::now() < deadline {
        let down = connection
            .query_keymap()
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .keys;
        let held = modifiers
            .iter()
            .filter(|&&code| code != 0)
            .any(|&code| down[code as usize / 8] & (1 << (code % 8)) != 0);
        if !held {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// The active window's app in an X11 session, which AT-SPI may not know.
pub fn x11_active_app() -> Option<App> {
    if env("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()) {
        // XWayland knows only its own windows.
        return None;
    }
    let (connection, screen) = x11rb::connect(None).ok()?;
    let root = connection.setup().roots[screen].root;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|reply| reply.atom)
    };
    let window = connection
        .get_property(
            false,
            root,
            atom(b"_NET_ACTIVE_WINDOW")?,
            AtomEnum::WINDOW,
            0,
            1,
        )
        .ok()?
        .reply()
        .ok()?
        .value32()?
        .next()
        .filter(|window| *window != 0)?;
    let pid = connection
        .get_property(
            false,
            window,
            atom(b"_NET_WM_PID")?,
            AtomEnum::CARDINAL,
            0,
            1,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32()?.next());
    let class = connection
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| {
            let value = reply.value;
            let instance = value.split(|byte| *byte == 0).next().unwrap_or_default();
            String::from_utf8_lossy(instance).into_owned()
        })
        .unwrap_or_default();
    Some(App {
        name: class,
        pid,
        executable: pid.and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok()),
    })
}

pub fn exists(path: &Path) -> bool {
    path.exists()
}
