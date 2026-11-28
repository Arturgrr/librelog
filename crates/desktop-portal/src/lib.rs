//! Librelog's identity with the desktop portals (`org.freedesktop.portal.*`),
//! and the session bus facts the desktop integration needs.
//!
//! The portals keep each app's grants (its global shortcuts, the approval to
//! send keys) under an app id. A sandboxed app has one; a host app such as
//! Librelog has to claim it with `org.freedesktop.host.portal.Registry`
//! before its first request on a bus connection, and the claim holds for that
//! connection only. GNOME 48 and later need it to attribute a session to an
//! app. So every portal request goes through [`connection`], which registers
//! once and is shared. The registry finds the app through its desktop entry,
//! `<app id>.desktop`: the packages install it, and [`prepare_app_id`] writes
//! one for an AppImage or a development build. A portal without the registry
//! derives the id from the systemd unit instead, so Librelog gives itself a
//! unit named after its id when it has none (see [`scope`]). A refused
//! registration never fails a request: it just goes unregistered.

#[cfg(any(target_os = "linux", test))]
pub mod autostart;
#[cfg(any(target_os = "linux", test))]
mod entry;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod scope;
#[cfg(target_os = "linux")]
mod shortcuts_held;
#[cfg(target_os = "linux")]
mod window;

#[cfg(target_os = "linux")]
pub use linux::{
    app_id, connection, forget_kde_shortcut, kde_shortcut_ids, prepare_app_id, set_app_id,
    status_notifier_host, watch_status_notifier_host,
};
#[cfg(target_os = "linux")]
pub use shortcuts_held::{
    note_shortcut_release, set_shortcuts_held, shortcut_releases_reported,
    wait_until_no_shortcut_is_held,
};
#[cfg(target_os = "linux")]
pub use window::{parent, set_activation_token, take_activation_token};

/// GNOME before 51 refuses portal app ids that are not reverse-DNS (two or
/// more dot-separated elements of letters, digits, `_` and `-`, none
/// starting with a digit, `-` only in the last).
pub fn reverse_dns(id: &str) -> bool {
    let elements: Vec<&str> = id.split('.').collect();
    !id.is_empty()
        && id.len() <= 255
        && elements.len() >= 2
        && elements.iter().enumerate().all(|(index, element)| {
            !element.is_empty()
                && !element.starts_with(|c: char| c.is_ascii_digit())
                && element.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || c == '_'
                        || (c == '-' && index == elements.len() - 1)
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reverse_dns_ids_go_to_the_portal() {
        for id in [
            "com.arturgrr.librelog",
            "com.arturgrr.librelog.nightly",
            "com.arturgrr.librelog.dev",
        ] {
            assert!(reverse_dns(id), "{id}");
        }
        for id in [
            "Librelog",
            "Librelog Dev",
            "",
            "com..librelog",
            "com.1librelog",
            "a-b.c",
        ] {
            assert!(!reverse_dns(id), "{id}");
        }
    }
}
