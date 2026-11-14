//! Librelog's identity with the desktop portals (`org.freedesktop.portal.*`).
//!
//! The portals keep each app's grants (its global shortcuts, the approval to
//! send keys) under an app id. A sandboxed app has one; a host app such as
//! Librelog has to claim it with `org.freedesktop.host.portal.Registry`
//! before its first request on a bus connection, and the claim holds for that
//! connection only. GNOME 48 and later need it to attribute a session to an
//! app. So every portal request goes through [`connection`], which registers
//! once and is shared. A portal without the registry (older ones derive the id
//! from the systemd unit instead), a refused registration or a build with no
//! desktop entry never fails a request: it just goes unregistered.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{connection, set_app_id};

use std::path::{Path, PathBuf};

/// The portal names a host app by its desktop entry: `<app id>.desktop` in
/// an `applications` folder of the XDG data dirs. The registry refuses an id
/// without one, so this picks the first candidate that has an entry: the
/// reverse-DNS `identifier` (GNOME before 51 refuses portal ids that are not
/// reverse-DNS), then `product_name`, the name the packages give the entry
/// (`Librelog.desktop`). `None` when neither is installed, as in development.
pub fn app_id(product_name: &str, identifier: &str, data_dirs: &[PathBuf]) -> Option<String> {
    [identifier, product_name]
        .into_iter()
        .filter(|id| valid_entry_name(id))
        .find(|id| {
            data_dirs.iter().any(|dir| {
                dir.join("applications")
                    .join(format!("{id}.desktop"))
                    .is_file()
            })
        })
        .map(str::to_owned)
}

/// `$XDG_DATA_HOME` (or `~/.local/share`), then `$XDG_DATA_DIRS` (or
/// `/usr/local/share:/usr/share`).
pub fn xdg_data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    data_dirs(
        std::env::var_os("XDG_DATA_HOME").as_deref().map(Path::new),
        std::env::var_os("XDG_DATA_DIRS").as_deref(),
        home.as_deref(),
    )
}

fn data_dirs(
    data_home: Option<&Path>,
    data_dirs: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match data_home.filter(|dir| dir.is_absolute()) {
        Some(dir) => dirs.push(dir.to_owned()),
        None => dirs.extend(home.map(|home| home.join(".local/share"))),
    }
    let system: Vec<PathBuf> = data_dirs
        .map(|dirs| {
            std::env::split_paths(dirs)
                .filter(|dir| dir.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    if system.is_empty() {
        dirs.extend(["/usr/local/share", "/usr/share"].map(PathBuf::from));
    } else {
        dirs.extend(system);
    }
    dirs
}

/// Desktop entry names are file names the portal can look up: no path
/// separators or spaces ("Librelog Dev" is not one).
fn valid_entry_name(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_app_id_is_the_installed_desktop_entry_preferring_reverse_dns() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let dirs = [user.path().to_owned(), system.path().to_owned()];
        let install = |dir: &Path, name: &str| {
            std::fs::create_dir_all(dir.join("applications")).unwrap();
            std::fs::write(dir.join("applications").join(name), "[Desktop Entry]\n").unwrap();
        };

        // A development build: no entry, so no registration.
        assert_eq!(
            app_id("Librelog Dev", "com.arturgrr.librelog.dev", &dirs),
            None
        );
        install(system.path(), "Librelog Dev.desktop");
        assert_eq!(
            app_id("Librelog Dev", "com.arturgrr.librelog.dev", &dirs),
            None
        );

        // The packages install Librelog.desktop.
        install(system.path(), "Librelog.desktop");
        assert_eq!(
            app_id("Librelog", "com.arturgrr.librelog", &dirs).as_deref(),
            Some("Librelog")
        );
        install(user.path(), "com.arturgrr.librelog.desktop");
        assert_eq!(
            app_id("Librelog", "com.arturgrr.librelog", &dirs).as_deref(),
            Some("com.arturgrr.librelog")
        );
    }

    #[test]
    fn data_dirs_follow_the_xdg_defaults() {
        let home = Path::new("/home/a");
        assert_eq!(
            data_dirs(None, None, Some(home)),
            ["/home/a/.local/share", "/usr/local/share", "/usr/share"].map(PathBuf::from)
        );
        assert_eq!(
            data_dirs(
                Some(Path::new("/data")),
                Some(std::ffi::OsStr::new("/opt/share:relative:/usr/share")),
                Some(home)
            ),
            ["/data", "/opt/share", "/usr/share"].map(PathBuf::from)
        );
    }
}
