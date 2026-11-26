//! Keeps macOS from napping the app while an annotation records or is being
//! prepared. With its windows hidden and another app in front, App Nap lowers
//! the app's CPU and I/O priority and coalesces its timers, which would leave
//! the narration's transcription and the bundle to crawl until the user comes
//! back to paste them.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

/// A user-initiated activity, ended on drop.
pub struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

// SAFETY: NSProcessInfo's activity calls may come from any thread, and the
// token is only handed back to `endActivity`.
unsafe impl Send for Activity {}

impl Activity {
    pub fn begin(reason: &str) -> Self {
        let reason = NSString::from_str(reason);
        let token = NSProcessInfo::processInfo()
            .beginActivityWithOptions_reason(NSActivityOptions::UserInitiated, &reason);
        Self(token)
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the token came from `beginActivityWithOptions_reason` and is
        // ended once.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
