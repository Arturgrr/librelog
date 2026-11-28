//! The desktop entry the portal registry names Librelog by. The rpm and deb
//! install `/usr/share/applications/<identifier>.desktop`; an AppImage or a
//! development build has none, so Librelog writes its own to
//! `~/.local/share/applications`, marked as managed, refreshes it when the
//! program moves, and removes it once a package provides one (a user entry
//! would hide the package's). An entry Librelog did not write is never
//! touched.

use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) const MANAGED: &str = "X-Librelog-Managed=true";

pub struct Launcher<'a> {
    pub identifier: &'a str,
    pub name: &'a str,
    /// What the entry starts: the AppImage itself, or the executable.
    pub executable: &'a Path,
    /// An AppImage gets a visible launcher, since nothing else installs one;
    /// a development build's entry only names it to the portal.
    pub visible: bool,
    pub icon: Option<&'a Path>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Entry {
    /// A package installed it.
    Installed(PathBuf),
    /// The user's own, left alone.
    Unmanaged(PathBuf),
    Written(PathBuf),
    Current(PathBuf),
}

pub fn ensure(
    launcher: &Launcher<'_>,
    data_home: &Path,
    data_dirs: &[PathBuf],
) -> std::io::Result<Entry> {
    let name = format!("{}.desktop", launcher.identifier);
    let own = data_home.join("applications").join(&name);
    let existing = std::fs::read_to_string(&own).ok();
    let managed = existing
        .as_deref()
        .is_some_and(|text| text.lines().any(|line| line.trim() == MANAGED));
    if let Some(installed) = data_dirs
        .iter()
        .filter(|dir| dir.as_path() != data_home)
        .map(|dir| dir.join("applications").join(&name))
        .find(|path| path.is_file())
    {
        if managed {
            std::fs::remove_file(&own)?;
        }
        return Ok(Entry::Installed(installed));
    }
    if existing.is_some() && !managed {
        return Ok(Entry::Unmanaged(own));
    }
    let wanted = desktop_entry(launcher);
    if existing.as_deref() == Some(wanted.as_str()) {
        return Ok(Entry::Current(own));
    }
    let dir = own.parent().expect("applications folder");
    std::fs::create_dir_all(dir)?;
    let staged = dir.join(format!(".{name}.{}", std::process::id()));
    let _ = std::fs::remove_file(&staged);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(wanted.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, &own)?;
    Ok(Entry::Written(own))
}

/// `data_home`, then `$XDG_DATA_DIRS` (or `/usr/local/share:/usr/share`).
pub fn data_dirs(data_home: &Path, system: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut dirs = vec![data_home.to_owned()];
    let system: Vec<PathBuf> = system
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

pub fn desktop_entry(launcher: &Launcher<'_>) -> String {
    let mut entry = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={} %u\nTerminal=false\nCategories=Office;\nStartupWMClass={}\n",
        launcher.name,
        exec_value(launcher.executable),
        // The window's WM_CLASS instance and Wayland app id: the app names
        // GTK's program after the identifier.
        launcher.identifier,
    );
    if let Some(icon) = launcher.icon {
        entry.push_str(&format!("Icon={}\n", icon.display()));
    }
    if !launcher.visible {
        entry.push_str("NoDisplay=true\n");
    }
    entry.push_str(MANAGED);
    entry.push('\n');
    entry
}

/// An `Exec` program quoted as the Desktop Entry spec asks: double quotes
/// around anything with reserved characters, `"`, `` ` ``, `$` and `\`
/// escaped inside them, and then every `\` doubled again because `Exec` is a
/// string value; `%` is written `%%`.
pub(crate) fn exec_value(path: &Path) -> String {
    let path = path.to_string_lossy().replace('%', "%%");
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let argument = if path.contains(reserved) {
        let mut quoted = String::from('"');
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        quoted
    } else {
        path
    };
    argument.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher<'a>(executable: &'a Path, visible: bool) -> Launcher<'a> {
        Launcher {
            identifier: "com.arturgrr.librelog",
            name: "Librelog",
            executable,
            visible,
            icon: None,
        }
    }

    #[test]
    fn data_dirs_follow_the_xdg_defaults() {
        let home = Path::new("/home/a/.local/share");
        assert_eq!(
            data_dirs(home, None),
            ["/home/a/.local/share", "/usr/local/share", "/usr/share"].map(PathBuf::from)
        );
        assert_eq!(
            data_dirs(
                home,
                Some(std::ffi::OsStr::new("/opt/share:relative:/usr/share"))
            ),
            ["/home/a/.local/share", "/opt/share", "/usr/share"].map(PathBuf::from)
        );
    }

    #[test]
    fn exec_paths_are_quoted_for_the_desktop_entry_spec() {
        assert_eq!(exec_value(Path::new("/opt/librelog")), "/opt/librelog");
        assert_eq!(
            exec_value(Path::new("/home/a/My Apps/Librelog.AppImage")),
            "\"/home/a/My Apps/Librelog.AppImage\""
        );
        assert_eq!(
            exec_value(Path::new("/home/a/$x \"q\"/100%")),
            "\"/home/a/\\\\$x \\\\\"q\\\\\"/100%%\""
        );
    }

    /// The portal registry refuses an id without an entry: an AppImage or a
    /// development build had none, so neither got any global shortcuts.
    #[test]
    fn an_entry_is_written_only_where_no_package_or_user_provides_one() {
        let home = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let dirs = [home.path().to_owned(), system.path().to_owned()];
        let own = home
            .path()
            .join("applications/com.arturgrr.librelog.desktop");
        let appimage = PathBuf::from("/home/a/Librelog.AppImage");

        assert_eq!(
            ensure(&launcher(&appimage, true), home.path(), &dirs).unwrap(),
            Entry::Written(own.clone())
        );
        assert_eq!(
            ensure(&launcher(&appimage, true), home.path(), &dirs).unwrap(),
            Entry::Current(own.clone())
        );
        // The AppImage moved: the entry follows it.
        let moved = PathBuf::from("/home/a/Apps/Librelog.AppImage");
        assert_eq!(
            ensure(&launcher(&moved, true), home.path(), &dirs).unwrap(),
            Entry::Written(own.clone())
        );
        assert!(
            std::fs::read_to_string(&own)
                .unwrap()
                .contains("Exec=/home/a/Apps/Librelog.AppImage %u")
        );

        // A package's entry wins, and Librelog's own would hide it.
        std::fs::create_dir_all(system.path().join("applications")).unwrap();
        let installed = system
            .path()
            .join("applications/com.arturgrr.librelog.desktop");
        std::fs::write(&installed, "[Desktop Entry]\n").unwrap();
        assert_eq!(
            ensure(&launcher(&moved, true), home.path(), &dirs).unwrap(),
            Entry::Installed(installed.clone())
        );
        assert!(!own.exists());

        // A user's own entry is never replaced.
        std::fs::remove_file(&installed).unwrap();
        std::fs::write(&own, "[Desktop Entry]\nName=Mine\n").unwrap();
        assert_eq!(
            ensure(&launcher(&moved, false), home.path(), &dirs).unwrap(),
            Entry::Unmanaged(own.clone())
        );
        assert_eq!(
            std::fs::read_to_string(&own).unwrap(),
            "[Desktop Entry]\nName=Mine\n"
        );
    }
}
