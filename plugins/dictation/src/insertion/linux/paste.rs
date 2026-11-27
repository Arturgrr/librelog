//! Pasting a dictation as macOS does when Accessibility cannot insert it:
//! the text goes on the clipboard, the paste keys are sent, and the
//! clipboard it replaced comes back a moment later. Without a way to send
//! the keys the text stays on the clipboard for the user to paste.

use std::time::Duration;

use super::route::Chord;

pub const COPIED: &str = "Your dictation is on the clipboard. Press Control+V to paste it.";
pub const COPIED_TERMINAL: &str =
    "Your dictation is on the clipboard. Press Control+Shift+V to paste it.";
pub const NO_CLIPBOARD: &str = "Could not put your dictation on the clipboard. Copy your last dictation from Settings > Dictation.";

/// How long the target gets to see the new clipboard before the keys.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(150);
/// How long after the paste keys the previous clipboard comes back. Nothing
/// acknowledges that the app read the paste.
pub const RESTORE_DELAY: Duration = Duration::from_millis(500);

/// What was on the clipboard before a paste.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Snapshot {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    /// Empty, or something only its owner can serve again (files, rich text
    /// without a plain one): putting it back would lose it, so the dictation stays.
    #[default]
    Unreadable,
}

pub trait Clipboard {
    fn snapshot(&mut self) -> Snapshot;
    fn set_text(&mut self, text: &str) -> Result<(), String>;
    /// Whether the clipboard still holds `text`: nothing replaced it since.
    fn holds(&mut self, text: &str) -> bool;
    fn put_back(&mut self, snapshot: Snapshot) -> Result<(), String>;
}

pub trait Keyboard {
    fn send(&mut self, chord: Chord) -> Result<(), String>;
}

pub fn copied(chord: Chord) -> String {
    match chord {
        Chord::Paste | Chord::Return => COPIED.into(),
        Chord::TerminalPaste => COPIED_TERMINAL.into(),
    }
}

/// `keyboard` is `Err` with the reason when no way to send keys is
/// available or allowed.
pub fn paste(
    clipboard: &mut impl Clipboard,
    keyboard: Result<&mut impl Keyboard, String>,
    text: &str,
    chord: Chord,
    wait: impl Fn(Duration),
) -> Result<(), String> {
    let original = clipboard.snapshot();
    if let Err(error) = clipboard.set_text(text) {
        tracing::warn!(%error, "could not put a dictation on the clipboard");
        return Err(NO_CLIPBOARD.into());
    }
    let keyboard = match keyboard {
        Ok(keyboard) => keyboard,
        Err(reason) => {
            tracing::warn!(%reason, "no way to send the paste keys; the dictation stays on the clipboard");
            return Err(copied(chord));
        }
    };
    wait(CLIPBOARD_SETTLE);
    if let Err(error) = keyboard.send(chord) {
        tracing::warn!(%error, "could not send the paste keys; the dictation stays on the clipboard");
        return Err(copied(chord));
    }
    wait(RESTORE_DELAY);
    if original != Snapshot::Unreadable
        && clipboard.holds(text)
        && let Err(error) = clipboard.put_back(original)
    {
        tracing::warn!(%error, "could not restore the clipboard after a paste");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct Fake {
        now: Snapshot,
        fail_set: bool,
        /// Another app copies this while the paste waits.
        copied_meanwhile: Option<String>,
    }

    impl Clipboard for Fake {
        fn snapshot(&mut self) -> Snapshot {
            self.now.clone()
        }
        fn set_text(&mut self, text: &str) -> Result<(), String> {
            if self.fail_set {
                return Err("no clipboard".into());
            }
            self.now = Snapshot::Text(text.into());
            Ok(())
        }
        fn holds(&mut self, text: &str) -> bool {
            if let Some(other) = self.copied_meanwhile.take() {
                self.now = Snapshot::Text(other);
            }
            self.now == Snapshot::Text(text.into())
        }
        fn put_back(&mut self, snapshot: Snapshot) -> Result<(), String> {
            self.now = snapshot;
            Ok(())
        }
    }

    struct Keys {
        sent: RefCell<Vec<Chord>>,
        fail: bool,
    }

    impl Keyboard for Keys {
        fn send(&mut self, chord: Chord) -> Result<(), String> {
            self.sent.borrow_mut().push(chord);
            if self.fail {
                Err("portal closed".into())
            } else {
                Ok(())
            }
        }
    }

    fn keys(fail: bool) -> Keys {
        Keys {
            sent: RefCell::default(),
            fail,
        }
    }

    #[test]
    fn a_paste_restores_the_clipboard_and_a_failed_one_leaves_the_dictation() {
        let before = Snapshot::Text("copied earlier".into());
        let mut clipboard = Fake {
            now: before.clone(),
            ..Fake::default()
        };
        let mut ok = keys(false);
        paste(
            &mut clipboard,
            Ok(&mut ok),
            "Hello.",
            Chord::TerminalPaste,
            |_| {},
        )
        .unwrap();
        assert_eq!(ok.sent.into_inner(), [Chord::TerminalPaste]);
        assert_eq!(clipboard.now, before);

        let mut failing = keys(true);
        assert_eq!(
            paste(
                &mut clipboard,
                Ok(&mut failing),
                "Hello.",
                Chord::Paste,
                |_| {}
            ),
            Err(COPIED.into())
        );
        assert_eq!(clipboard.now, Snapshot::Text("Hello.".into()));

        // No way to send keys: copied, never restored.
        let mut clipboard = Fake {
            now: before,
            ..Fake::default()
        };
        assert_eq!(
            paste(
                &mut clipboard,
                Err::<&mut Keys, _>("denied".into()),
                "Hi.",
                Chord::TerminalPaste,
                |_| {}
            ),
            Err(COPIED_TERMINAL.into())
        );
        assert_eq!(clipboard.now, Snapshot::Text("Hi.".into()));
    }

    #[test]
    fn a_restore_never_overwrites_a_newer_copy_or_drops_what_it_could_not_read() {
        let mut clipboard = Fake {
            now: Snapshot::Text("old".into()),
            copied_meanwhile: Some("newer".into()),
            ..Fake::default()
        };
        paste(
            &mut clipboard,
            Ok(&mut keys(false)),
            "Hello.",
            Chord::Paste,
            |_| {},
        )
        .unwrap();
        assert_eq!(clipboard.now, Snapshot::Text("newer".into()));

        let mut clipboard = Fake {
            now: Snapshot::Unreadable,
            ..Fake::default()
        };
        paste(
            &mut clipboard,
            Ok(&mut keys(false)),
            "Hello.",
            Chord::Paste,
            |_| {},
        )
        .unwrap();
        assert_eq!(clipboard.now, Snapshot::Text("Hello.".into()));

        let mut clipboard = Fake {
            fail_set: true,
            ..Fake::default()
        };
        let mut unused = keys(false);
        assert_eq!(
            paste(
                &mut clipboard,
                Ok(&mut unused),
                "Hello.",
                Chord::Paste,
                |_| {}
            ),
            Err(NO_CLIPBOARD.into())
        );
        assert!(unused.sent.into_inner().is_empty());
    }
}
