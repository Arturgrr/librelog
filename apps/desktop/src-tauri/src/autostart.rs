//! Linux's "Start at login", answering `@tauri-apps/plugin-autostart`'s
//! commands under its name so the capability and the frontend stay as they
//! are: tauri-plugin-autostart names the entry after the product, and the
//! portals need it named after the app id. See
//! [`anlg_desktop_portal::autostart`].

use std::path::PathBuf;

use anlg_desktop_portal::autostart::{self, Autostart};
use tauri::{Manager, Runtime, plugin::TauriPlugin};

const ARGS: &[&str] = &["--background"];

struct Entry {
    identifier: String,
    name: String,
    executable: PathBuf,
}

impl Entry {
    fn run<T>(
        &self,
        act: impl FnOnce(&Autostart<'_>, &std::path::Path) -> std::io::Result<T>,
    ) -> Result<T, String> {
        let dir = autostart::dir(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").map(PathBuf::from).as_deref(),
        )
        .ok_or("the home folder is unknown")?;
        let autostart = Autostart {
            identifier: &self.identifier,
            name: &self.name,
            executable: &self.executable,
            args: ARGS,
        };
        act(&autostart, &dir).map_err(|error| error.to_string())
    }
}

#[tauri::command]
fn enable(entry: tauri::State<'_, Entry>) -> Result<(), String> {
    entry.run(|autostart, dir| autostart.enable(dir))
}

#[tauri::command]
fn disable(entry: tauri::State<'_, Entry>) -> Result<(), String> {
    entry.run(|autostart, dir| autostart.disable(dir))
}

#[tauri::command]
fn is_enabled(entry: tauri::State<'_, Entry>) -> Result<bool, String> {
    entry.run(|autostart, dir| Ok(autostart.is_enabled(dir)))
}

pub fn init<R: Runtime>(identifier: String, name: String) -> TauriPlugin<R> {
    tauri::plugin::Builder::new("autostart")
        .invoke_handler(tauri::generate_handler![enable, disable, is_enabled])
        .setup(move |app, _| {
            let executable = match app.env().appimage {
                Some(appimage) => PathBuf::from(appimage),
                None => std::env::current_exe()?,
            };
            let entry = Entry {
                identifier,
                name,
                executable,
            };
            match entry.run(|autostart, dir| autostart.migrate(dir)) {
                Ok(true) => tracing::info!("updated the start-at-login entry"),
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "could not update the start-at-login entry"),
            }
            app.manage(entry);
            Ok(())
        })
        .build()
}
