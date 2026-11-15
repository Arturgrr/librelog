//! Whether a Librelog global shortcut is down, which the shortcut plugin
//! knows from the portal and dictation needs: a paste chord sent through the
//! portal or `wtype` while the shortcut's Alt or Shift is still held reaches
//! the app as another chord.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static HELD: AtomicUsize = AtomicUsize::new(0);

pub fn set_shortcuts_held(count: usize) {
    HELD.store(count, Ordering::SeqCst);
}

/// Waits up to `limit` for every shortcut to be released; true when none is
/// held.
pub fn wait_until_no_shortcut_is_held(limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while HELD.load(Ordering::SeqCst) > 0 {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}
