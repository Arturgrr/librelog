//! What the Global Shortcuts portal binds. On Wayland every Librelog
//! shortcut lives in one portal session: desktops keep one shortcut set per
//! app, so a second session's set would replace the first's. [`Held`] stands
//! in for the OS in [`super::Keys`], so each family keeps the same rules as
//! on X11 (who yields to whom, partial Capture registration), and the session
//! binds whatever it holds.
//!
//! The portal only suggests each trigger: the desktop asks the user to
//! approve or change it, and keeps what it stored for an id over a later
//! suggestion (GNOME, KDE). So each id carries the trigger it asks for: a
//! shortcut changed in Settings is a new id, which the desktop asks about,
//! while an unchanged one keeps its id and any key the user gave it there.

use std::cell::RefCell;

use tauri_plugin_global_shortcut::Shortcut;

use super::{Action, Backend, trigger::portal_trigger};
use crate::CaptureAction;

/// Control+Alt+Escape is KWin's Kill Window.
const CANCEL_TRIGGER: &str = "CTRL+ALT+period";

/// The shortcuts held for the portal session, as the OS would hold them.
#[derive(Default)]
pub(super) struct Held(RefCell<Vec<(Shortcut, Action)>>);

impl Backend for Held {
    fn register(&self, key: Shortcut, action: Action) -> Result<(), String> {
        let mut held = self.0.borrow_mut();
        if held.iter().any(|(other, _)| *other == key) {
            return Err("already registered".into());
        }
        held.push((key, action));
        Ok(())
    }

    fn unregister(&self, key: Shortcut) -> Result<(), String> {
        self.0.borrow_mut().retain(|(other, _)| *other != key);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Binding {
    pub id: String,
    pub description: String,
    pub trigger: String,
    pub does: Does,
    /// The Librelog shortcut it stands for; none for the cancel one.
    pub key: Option<Shortcut>,
}

/// `{base}-{trigger}`, with the trigger's `+` as `-`.
fn shortcut_id(base: &str, trigger: &str) -> String {
    format!("{base}-{}", trigger.replace('+', "-"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Does {
    Act(Action),
    /// Cancels dictation, and turns Annotate's ink off: Escape on X11.
    Cancel,
}

impl Held {
    /// The session's shortcuts in a stable order. Escape is never bound
    /// (the portal cannot take it only while dictating or inking, and would
    /// keep it from every app): a cancel shortcut, Control+Alt+period unless
    /// the user picks another, stands in for it while dictation or Annotate
    /// has a shortcut.
    pub fn bindings(&self) -> Result<Vec<Binding>, String> {
        let mut held: Vec<(Shortcut, Action)> = self
            .0
            .borrow()
            .iter()
            .copied()
            .filter(|(_, action)| !matches!(action, Action::Cancel))
            .collect();
        held.sort_by_key(|(_, action)| order(*action));
        let mut bindings = Vec::new();
        let mut dictations = 0;
        for (key, action) in &held {
            let (base, description) = match action {
                Action::Dictate(_) => {
                    dictations += 1;
                    let description = if dictations == 1 {
                        "Dictate with Librelog".to_string()
                    } else {
                        format!("Dictate with Librelog ({dictations})")
                    };
                    ("dictate".to_string(), description)
                }
                Action::PasteLast => (
                    "paste-last-dictation".into(),
                    "Paste the last Librelog dictation".into(),
                ),
                Action::Annotate => (
                    "annotate".into(),
                    "Start, stop or draw with Librelog Annotate".into(),
                ),
                Action::PasteLastAnnotation => (
                    "paste-last-annotation".into(),
                    "Paste the last Librelog annotation".into(),
                ),
                Action::Capture(capture) => capture_binding(*capture),
                Action::Cancel => continue,
            };
            let trigger = portal_trigger(*key)?;
            bindings.push(Binding {
                id: shortcut_id(&base, &trigger),
                description,
                trigger,
                does: Does::Act(*action),
                key: Some(*key),
            });
        }
        if held
            .iter()
            .any(|(_, action)| matches!(action, Action::Dictate(_) | Action::Annotate))
        {
            let at = bindings
                .iter()
                .position(|binding| !matches!(binding.does, Does::Act(Action::Dictate(_))))
                .unwrap_or(bindings.len());
            bindings.insert(
                at,
                Binding {
                    id: shortcut_id("cancel-dictation", CANCEL_TRIGGER),
                    description: "Cancel Librelog dictation".into(),
                    trigger: CANCEL_TRIGGER.into(),
                    does: Does::Cancel,
                    key: None,
                },
            );
        }
        Ok(bindings)
    }
}

fn order(action: Action) -> (u8, usize) {
    match action {
        Action::Dictate(source) => (0, source),
        Action::Cancel => (1, 0),
        Action::PasteLast => (2, 0),
        Action::Annotate => (3, 0),
        Action::PasteLastAnnotation => (4, 0),
        Action::Capture(capture) => (5, capture as usize),
    }
}

fn capture_binding(action: CaptureAction) -> (String, String) {
    let (id, description) = match action {
        CaptureAction::Area => ("capture-area", "Capture an area with Librelog"),
        CaptureAction::Fullscreen => ("capture-fullscreen", "Capture the screen with Librelog"),
        CaptureAction::Window => ("capture-window", "Capture a window with Librelog"),
        CaptureAction::Record => ("capture-record", "Record the screen with Librelog"),
        CaptureAction::Gif => ("capture-gif", "Record a GIF with Librelog"),
    };
    (id.into(), description.into())
}

/// Whether the desktop's description of a trigger (`Ctrl+Alt+Space`,
/// `Press <ctrl><alt>space`) names the keys Librelog asked for
/// (`CTRL+ALT+space`).
pub(super) fn same_trigger(asked: &str, described: &str) -> bool {
    fn keys(trigger: &str) -> std::collections::BTreeSet<String> {
        let trigger = trigger.trim();
        let trigger = trigger.strip_prefix("Press ").unwrap_or(trigger);
        trigger
            .split(['+', '<', '>', ' '])
            .filter(|key| !key.is_empty())
            .map(|key| match key.to_lowercase().as_str() {
                "control" | "primary" => "ctrl".to_string(),
                "logo" | "meta" | "win" | "mod4" => "super".to_string(),
                "." => "period".to_string(),
                "pgup" | "prior" => "page_up".to_string(),
                "pgdown" | "next" => "page_down".to_string(),
                key => key.to_string(),
            })
            .collect()
    }
    keys(asked) == keys(described)
}

/// Paste shortcuts fire once the shortcut's keys are up: a Control+V sent
/// while its Alt or Shift is still down reaches the app as another chord.
/// A desktop that never reports the release pastes after [`RELEASE_WAIT`].
#[derive(Default)]
pub(super) struct ReleaseGate {
    presses: u64,
    pending: Option<u64>,
}

pub(super) const RELEASE_WAIT: std::time::Duration = std::time::Duration::from_millis(1_500);

impl ReleaseGate {
    /// A press: call [`Self::timed_out`] with it after [`RELEASE_WAIT`].
    pub fn press(&mut self) -> u64 {
        self.presses += 1;
        self.pending = Some(self.presses);
        self.presses
    }

    /// The release: whether it fires the paste.
    pub fn release(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Whether `press`, still unreleased, fires now.
    pub fn timed_out(&mut self, press: u64) -> bool {
        if self.pending == Some(press) {
            self.pending = None;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Keys, parse_shortcut};
    use super::*;

    fn key(shortcut: &str) -> Shortcut {
        parse_shortcut(shortcut).unwrap()
    }

    fn ids(held: &Held) -> Vec<(String, String)> {
        held.bindings()
            .unwrap()
            .into_iter()
            .map(|binding| (binding.id, binding.trigger))
            .collect()
    }

    /// Wayland used to bind dictation and paste-last only: Annotate and
    /// Capture shortcuts answered "unavailable".
    #[test]
    fn one_portal_session_binds_every_family_with_the_x11_rules() {
        let held = Held::default();
        let mut keys = Keys::default();
        keys.set_capture(
            &held,
            vec![
                (CaptureAction::Area, Ok(key("Alt+Shift+Digit4"))),
                (CaptureAction::Gif, Ok(key("Alt+Shift+Digit6"))),
            ],
        )
        .unwrap();
        keys.add_dictation(
            &held,
            vec![(0, key("Control+Alt+Space")), (2, key("Alt+Shift+Digit6"))],
        )
        .unwrap();
        keys.set_paste_last(&held, Some(key("Alt+Shift+KeyV")))
            .unwrap();
        assert_eq!(
            keys.set_annotate(&held, Some(key("Alt+Shift+KeyV")), None),
            Err(super::super::TAKEN_BY_PASTE_LAST.into())
        );
        keys.set_annotate(
            &held,
            Some(key("Alt+Shift+KeyA")),
            Some(key("Alt+Shift+KeyP")),
        )
        .unwrap();

        // Capture's GIF gave way to the second dictation shortcut.
        assert_eq!(
            ids(&held),
            [
                ("dictate-CTRL-ALT-space", "CTRL+ALT+space"),
                ("dictate-ALT-SHIFT-6", "ALT+SHIFT+6"),
                ("cancel-dictation-CTRL-ALT-period", "CTRL+ALT+period"),
                ("paste-last-dictation-ALT-SHIFT-v", "ALT+SHIFT+v"),
                ("annotate-ALT-SHIFT-a", "ALT+SHIFT+a"),
                ("paste-last-annotation-ALT-SHIFT-p", "ALT+SHIFT+p"),
                ("capture-area-ALT-SHIFT-4", "ALT+SHIFT+4"),
            ]
            .map(|(id, trigger)| (id.to_string(), trigger.to_string()))
        );

        // Annotate alone still gets the cancel shortcut, to turn ink off.
        keys.remove_dictation(&held).unwrap();
        keys.set_paste_last(&held, None).unwrap();
        keys.set_capture(&held, vec![]).unwrap();
        assert_eq!(
            ids(&held).into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
            [
                "cancel-dictation-CTRL-ALT-period",
                "annotate-ALT-SHIFT-a",
                "paste-last-annotation-ALT-SHIFT-p"
            ]
        );
        keys.set_annotate(&held, None, None).unwrap();
        assert!(held.bindings().unwrap().is_empty());
    }

    /// Ids used to be positions: a desktop kept the key it stored for
    /// `dictate-0`, so a shortcut changed in Settings never changed, and
    /// removing the first of two shortcuts moved the second onto its id.
    #[test]
    fn a_changed_shortcut_is_a_new_id_and_an_unchanged_one_keeps_its_own() {
        let held = Held::default();
        let mut keys = Keys::default();
        let dictation_ids = |held: &Held| -> Vec<String> {
            ids(held)
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| id.starts_with("dictate-"))
                .collect()
        };
        keys.add_dictation(
            &held,
            vec![(0, key("Control+Alt+Space")), (1, key("Alt+KeyD"))],
        )
        .unwrap();
        let second = dictation_ids(&held)[1].clone();
        keys.remove_dictation(&held).unwrap();
        keys.add_dictation(&held, vec![(0, key("Alt+KeyD"))])
            .unwrap();
        assert_eq!(dictation_ids(&held), [second]);
        keys.remove_dictation(&held).unwrap();
        keys.add_dictation(&held, vec![(0, key("Alt+KeyJ"))])
            .unwrap();
        assert_eq!(dictation_ids(&held), ["dictate-ALT-j"]);
    }

    #[test]
    fn the_desktops_own_wording_of_a_trigger_matches_what_was_asked() {
        assert!(same_trigger("CTRL+ALT+space", "Ctrl+Alt+Space"));
        assert!(same_trigger("CTRL+ALT+space", "Press <ctrl><alt>space"));
        assert!(same_trigger("LOGO+SHIFT+Page_Up", "Super+Shift+Page_Up"));
        assert!(same_trigger("CTRL+ALT+period", "Ctrl+Alt+."));
        assert!(!same_trigger("CTRL+ALT+space", "Ctrl+Alt+D"));
        assert!(!same_trigger("CTRL+ALT+space", "Alt+Space"));
    }

    #[test]
    fn a_paste_shortcut_fires_once_on_release_or_after_the_wait() {
        let mut gate = ReleaseGate::default();
        let first = gate.press();
        assert!(gate.release());
        assert!(!gate.timed_out(first));
        assert!(!gate.release());

        // No release came: the wait fires it, and a late release does not.
        let second = gate.press();
        assert!(gate.timed_out(second));
        assert!(!gate.release());

        // A newer press supersedes an older one's wait.
        let third = gate.press();
        let fourth = gate.press();
        assert!(!gate.timed_out(third));
        assert!(gate.timed_out(fourth));
    }
}
