//! Starting Librelog at login, through `~/.config/autostart`. The entry is
//! `<identifier>.desktop`: desktops start it as
//! `app-<identifier>@autostart.service`, and xdg-desktop-portal before 1.19
//! (no host app registry, as on Kubuntu 24.04) takes the app id from that
//! unit's name. tauri-plugin-autostart named it after the product
//! (`Librelog.desktop`), so a Librelog started at login had another app id
//! than one started from the menu: a second approval prompt, and a second set
//! of shortcuts in the desktop's settings.

use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Autostart<'a> {
    pub identifier: &'a str,
    /// The product name, which the old entry was named after.
    pub name: &'a str,
    /// The AppImage itself, or the executable.
    pub executable: &'a Path,
    pub args: &'a [&'a str],
}

/// `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`.
pub fn dir(config_home: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    config_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|config| config.join("autostart"))
}

impl Autostart<'_> {
    fn path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.desktop", self.identifier))
    }

    /// tauri-plugin-autostart's entry, recognised by the comment it writes,
    /// so an entry the user made under that name is left alone.
    fn legacy(&self, dir: &Path) -> Option<PathBuf> {
        let path = dir.join(format!("{}.desktop", self.name));
        let comment = format!("Comment={}startup script", self.name);
        std::fs::read_to_string(&path)
            .ok()?
            .lines()
            .any(|line| line.trim() == comment)
            .then_some(path)
    }

    pub fn entry(&self) -> String {
        let mut exec = crate::entry::exec_value(self.executable);
        for arg in self.args {
            exec.push(' ');
            exec.push_str(arg);
        }
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec={exec}\nTerminal=false\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n{}\n",
            self.name,
            crate::entry::MANAGED,
        )
    }

    pub fn is_enabled(&self, dir: &Path) -> bool {
        self.path(dir).is_file() || self.legacy(dir).is_some()
    }

    pub fn enable(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = self.path(dir);
        let staged = dir.join(format!(
            ".{}.desktop.{}",
            self.identifier,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&staged);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        file.write_all(self.entry().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&staged, &path)?;
        if let Some(legacy) = self.legacy(dir) {
            std::fs::remove_file(legacy)?;
        }
        Ok(())
    }

    pub fn disable(&self, dir: &Path) -> std::io::Result<()> {
        for path in [Some(self.path(dir)), self.legacy(dir)]
            .into_iter()
            .flatten()
        {
            match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    /// Replaces the old product-named entry with this one, and rewrites an
    /// entry of ours that starts another program (the AppImage moved).
    /// Returns whether it wrote anything.
    pub fn migrate(&self, dir: &Path) -> std::io::Result<bool> {
        let current = std::fs::read_to_string(self.path(dir)).ok();
        let ours = current.as_deref().is_some_and(|text| {
            text.lines()
                .any(|line| line.trim() == crate::entry::MANAGED)
        });
        let stale = ours && current.as_deref() != Some(self.entry().as_str());
        if self.legacy(dir).is_none() && !stale {
            return Ok(false);
        }
        if current.is_some() && !ours {
            // The user's own entry under the new name already starts it.
            if let Some(legacy) = self.legacy(dir) {
                std::fs::remove_file(legacy)?;
            }
            return Ok(true);
        }
        self.enable(dir)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn autostart(executable: &Path) -> Autostart<'_> {
        Autostart {
            identifier: "com.arturgrr.librelog",
            name: "Librelog",
            executable,
            args: &["--background"],
        }
    }

    const LEGACY: &str = "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Librelog\nComment=Librelogstartup script\nExec=/usr/bin/librelog --background\nStartupNotify=false\nTerminal=false";

    /// tauri-plugin-autostart wrote `Librelog.desktop`, so on portals without
    /// the host app registry a Librelog started at login got the app id
    /// `Librelog` and asked for its shortcuts and approvals again.
    #[test]
    fn the_login_entry_is_named_after_the_app_id_and_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let exe = PathBuf::from("/usr/bin/librelog");
        let autostart = autostart(&exe);
        let new = dir.path().join("com.arturgrr.librelog.desktop");
        let old = dir.path().join("Librelog.desktop");

        std::fs::write(&old, LEGACY).unwrap();
        assert!(autostart.is_enabled(dir.path()));
        assert!(autostart.migrate(dir.path()).unwrap());
        assert!(!old.exists());
        let entry = std::fs::read_to_string(&new).unwrap();
        assert!(entry.contains("\nName=Librelog\n"));
        assert!(entry.contains("\nExec=/usr/bin/librelog --background\n"));
        assert!(!autostart.migrate(dir.path()).unwrap());

        // The AppImage moved: the entry follows it.
        let moved = PathBuf::from("/home/a/My Apps/Librelog.AppImage");
        let moved_autostart = Autostart {
            executable: &moved,
            ..self::autostart(&exe)
        };
        assert!(moved_autostart.migrate(dir.path()).unwrap());
        assert!(
            std::fs::read_to_string(&new)
                .unwrap()
                .contains("Exec=\"/home/a/My Apps/Librelog.AppImage\" --background")
        );

        std::fs::write(&old, LEGACY).unwrap();
        autostart.disable(dir.path()).unwrap();
        assert!(!new.exists() && !old.exists());
        assert!(!autostart.is_enabled(dir.path()));

        // Entries the user made are never touched.
        std::fs::write(&old, "[Desktop Entry]\nName=Mine\n").unwrap();
        assert!(!autostart.migrate(dir.path()).unwrap());
        assert!(!autostart.is_enabled(dir.path()));
        autostart.enable(dir.path()).unwrap();
        autostart.disable(dir.path()).unwrap();
        assert!(old.exists());
    }
}
