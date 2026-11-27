//! Moving a capture to the Trash without GNOME's `gio`: through KDE's
//! `kioclient`, or the FreeDesktop Trash spec's home trash, which Dolphin,
//! Nautilus and the other file managers list.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// `$XDG_DATA_HOME/Trash`, else `~/.local/share/Trash`.
pub fn home_trash() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")))?;
    Some(data.join("Trash"))
}

/// Moves `path` into the home trash at `trash`, recording where it came
/// from and when (`deleted_at`, local time as `YYYY-MM-DDThh:mm:ss`) so the
/// file manager can restore it. Returns where the file now is. A file on
/// another file system is refused rather than copied.
pub fn move_to(trash: &Path, path: &Path, deleted_at: &str) -> io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let files = trash.join("files");
    let info = trash.join("info");
    for dir in [trash, &files, &info] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let contents = trash_info(&path, deleted_at);
    for attempt in 1..1000 {
        let candidate = if attempt == 1 {
            name.clone()
        } else {
            numbered(&name, attempt)
        };
        // Creating the info file first and exclusively reserves the name,
        // as the spec asks, so two deletions never pick the same one.
        let info_path = info.join(format!("{candidate}.trashinfo"));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&info_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let target = files.join(&candidate);
        if target.symlink_metadata().is_ok() {
            drop(file);
            let _ = fs::remove_file(&info_path);
            continue;
        }
        let moved = file
            .write_all(contents.as_bytes())
            .and_then(|()| fs::rename(&path, &target));
        if let Err(error) = moved {
            let _ = fs::remove_file(&info_path);
            return Err(error);
        }
        return Ok(target);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "the Trash has too many files with this name",
    ))
}

/// `shot.png` → `shot.2.png`, keeping the extension so the file manager
/// still shows the right icon.
fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem}.{n}.{extension}"),
        _ => format!("{name}.{n}"),
    }
}

/// The `.trashinfo` body: the original path percent-encoded as the spec
/// requires, so names with spaces or `%` restore to the right place.
fn trash_info(path: &Path, deleted_at: &str) -> String {
    let encoded = url::Url::from_file_path(path)
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|()| path.to_string_lossy().into_owned());
    format!("[Trash Info]\nPath={encoded}\nDeletionDate={deleted_at}\n")
}

/// Now in local time, the format `DeletionDate` takes.
pub fn now() -> String {
    let mut seconds: libc::time_t = 0;
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::time(&mut seconds);
        libc::localtime_r(&seconds, &mut local);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second capture with the same name overwrote the first one in the
    /// Trash, or its .trashinfo did not let the file manager restore it.
    #[test]
    fn trashed_captures_keep_their_own_name_and_restore_path() {
        let home = tempfile::tempdir().unwrap();
        let trash = home.path().join("Trash");
        let folder = home.path().join("My Captures");
        fs::create_dir(&folder).unwrap();
        let first = folder.join("shot 100%.png");
        fs::write(&first, b"one").unwrap();
        let in_trash = move_to(&trash, &first, "2026-11-27T09:05:00").unwrap();
        fs::write(&first, b"two").unwrap();
        let second = move_to(&trash, &first, "2026-11-27T09:06:00").unwrap();

        assert!(!first.exists());
        assert_eq!(in_trash, trash.join("files/shot 100%.png"));
        assert_eq!(second, trash.join("files/shot 100%.2.png"));
        assert_eq!(fs::read(&in_trash).unwrap(), b"one");
        assert_eq!(fs::read(&second).unwrap(), b"two");
        let info = fs::read_to_string(trash.join("info/shot 100%.2.png.trashinfo")).unwrap();
        let encoded = folder.to_string_lossy().replace(' ', "%20");
        assert_eq!(
            info,
            format!(
                "[Trash Info]\nPath={encoded}/shot%20100%25.png\nDeletionDate=2026-11-27T09:06:00\n"
            )
        );
    }
}
