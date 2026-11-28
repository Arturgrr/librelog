use std::sync::Mutex;
#[cfg(any(target_os = "linux", test))]
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

use crate::ShortcutEvent;

/// Several shortcuts can start the same dictation. The first one pressed owns
/// it until that shortcut is released, so pressing another one meanwhile does
/// nothing. `source` is the shortcut's position in the configured list.
#[derive(Default)]
pub struct Presses(Mutex<State>);

#[derive(Default)]
struct State {
    /// The shortcut holding the dictation, and which press that was.
    owner: Option<(usize, u64)>,
    presses: u64,
    /// The press that held the shortcut when a capture became active.
    active: Option<u64>,
}

impl Presses {
    fn filter(&self, source: usize, event: ShortcutEvent) -> Option<ShortcutEvent> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pass = match event {
            ShortcutEvent::Pressed => state.owner.is_none(),
            ShortcutEvent::Released | ShortcutEvent::Discarded => {
                state.owner.is_some_and(|(owner, _)| owner == source)
            }
            ShortcutEvent::Cancelled => true,
        };
        if pass {
            state.owner = if matches!(event, ShortcutEvent::Pressed) {
                state.presses += 1;
                Some((source, state.presses))
            } else {
                None
            };
        }
        pass.then_some(event)
    }

    pub fn reset(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).owner = None;
    }

    /// A capture began, held by the current press if any.
    pub fn activate(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.active = state.owner.map(|(_, press)| press);
    }

    /// A capture ended. Forgets the press that held it, whose release may
    /// never come, but not a newer press: when a shortcut is released and
    /// pressed again before this arrives, that press starts the next capture
    /// and its release must still stop it.
    pub fn deactivate(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.owner.map(|(_, press)| press) == state.active.take() {
            state.owner = None;
        }
    }
}

/// The second release within this of the first locks the dictation, as on
/// macOS (`anlg_shortcut_macos`'s `DOUBLE_TAP_WINDOW`).
#[cfg(any(target_os = "linux", test))]
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(300);

/// macOS's double-tap lock for a desktop that reports a shortcut's press and
/// release (the Wayland portal on KDE and GNOME): tap a dictation shortcut,
/// then press and release it again within [`DOUBLE_TAP_WINDOW`] of the first
/// release, and that second release is held back, so the dictation goes on
/// hands-free. The next press and release end it as a hold would. A desktop
/// that never reports releases never locks.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub struct TapLock {
    last_release: Option<Instant>,
    /// The press after a lock: its release ends the dictation and starts
    /// no new tap.
    unlocking: bool,
    locked: bool,
}

#[cfg(any(target_os = "linux", test))]
impl TapLock {
    pub fn press(&mut self) {
        self.unlocking = std::mem::take(&mut self.locked);
    }

    /// Whether this release passes on; false locks the dictation.
    pub fn release(&mut self, now: Instant) -> bool {
        if std::mem::take(&mut self.unlocking) {
            self.last_release = None;
            return true;
        }
        if self
            .last_release
            .take()
            .is_some_and(|at| now.saturating_duration_since(at) < DOUBLE_TAP_WINDOW)
        {
            self.locked = true;
            return false;
        }
        self.last_release = Some(now);
        true
    }
}

/// The dictation's owner lets go without a release reaching the dictation,
/// so the next press of any dictation shortcut is heard.
#[cfg(target_os = "linux")]
pub fn forget<R: Runtime>(app: &AppHandle<R>, source: usize) {
    let _ = app
        .state::<Presses>()
        .filter(source, ShortcutEvent::Released);
}

pub fn emit<R: Runtime>(app: &AppHandle<R>, source: usize, event: ShortcutEvent) {
    if let Some(event) = app.state::<Presses>().filter(source, event) {
        let _ = event.emit(app);
    }
}

pub fn cancel<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Presses>().reset();
    let _ = ShortcutEvent::Cancelled.emit(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_shortcut_pressed_owns_the_dictation_until_it_is_released() {
        let presses = Presses::default();
        let pass = |source, event| presses.filter(source, event).is_some();

        assert!(pass(0, ShortcutEvent::Pressed));
        assert!(!pass(1, ShortcutEvent::Pressed));
        assert!(!pass(1, ShortcutEvent::Released));
        assert!(!pass(0, ShortcutEvent::Pressed));
        assert!(pass(0, ShortcutEvent::Released));
        assert!(!pass(0, ShortcutEvent::Released));

        assert!(pass(1, ShortcutEvent::Pressed));
        assert!(!pass(0, ShortcutEvent::Discarded));
        assert!(pass(0, ShortcutEvent::Cancelled));
        assert!(pass(0, ShortcutEvent::Pressed));
    }

    /// The double-tap lock was macOS-only: on KDE a dictation could only be
    /// held or toggled for good in Settings.
    #[test]
    fn a_double_tap_holds_back_its_release_and_the_next_tap_ends_the_dictation() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut lock = TapLock::default();

        // A tap, and a second one ending 250 ms after the first: locked.
        lock.press();
        assert!(lock.release(at(100)));
        lock.press();
        assert!(!lock.release(at(350)));
        // The next press and release end it, and start no new tap.
        lock.press();
        assert!(lock.release(at(2_000)));
        lock.press();
        assert!(lock.release(at(2_200)));

        // Releases further apart are two holds.
        lock.press();
        assert!(lock.release(at(3_000)));
        lock.press();
        assert!(lock.release(at(3_400)));
    }

    #[test]
    fn a_quick_second_press_still_stops_its_capture_when_the_first_ends_late() {
        let presses = Presses::default();
        let pass = |source, event| presses.filter(source, event).is_some();

        assert!(pass(0, ShortcutEvent::Pressed));
        presses.activate();
        assert!(pass(0, ShortcutEvent::Released));
        assert!(pass(0, ShortcutEvent::Pressed));
        presses.deactivate();
        assert!(pass(0, ShortcutEvent::Released));

        // A release that never came no longer blocks the next dictation.
        assert!(pass(0, ShortcutEvent::Pressed));
        presses.activate();
        presses.deactivate();
        assert!(pass(1, ShortcutEvent::Pressed));
    }
}
