//! The active window on KWin (KDE Plasma), which nothing else names there:
//! Qt apps join AT-SPI only when a screen reader or the accessibility setting
//! is on, off by default on Plasma, and XWayland sees only its own windows.
//! KWin runs a short script that reads its active window and calls back over
//! the session bus, as kdotool does. The window's title is not read.

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const KWIN: &str = "org.kde.KWin";
const INTERFACE: &str = "librelog.ActiveWindow";
/// How long KWin gets to load, run and answer.
const TIMEOUT: Duration = Duration::from_secs(1);

/// The active window as KWin describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub pid: Option<u32>,
    /// `resourceClass`: the Wayland app id or the X11 class, `org.kde.konsole`.
    pub class: String,
    /// `resourceName`: the X11 instance, or the executable's name, `konsole`.
    pub name: String,
    /// The desktop entry KWin matched, `org.kde.konsole`, when it found one.
    pub desktop_file: Option<String>,
}

impl Window {
    fn new(pid: &str, class: &str, name: &str, desktop_file: &str) -> Self {
        Self {
            pid: pid.parse().ok().filter(|pid| *pid > 0),
            class: class.to_owned(),
            name: name.to_owned(),
            desktop_file: (!desktop_file.is_empty()).then(|| desktop_file.to_owned()),
        }
    }
}

/// Whether the desktop is KDE Plasma, from `XDG_CURRENT_DESKTOP`.
pub fn on_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktops| {
        desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("kde"))
    })
}

/// KWin's scripting interface, behind a trait so the probe's clean-up can be
/// tested without KWin.
trait Scripting {
    fn load(&self, file: &Path, plugin: &str) -> impl Future<Output = Result<i32, String>>;
    fn run(&self, id: i32) -> impl Future<Output = Result<(), String>>;
    fn unload(&self, plugin: &str) -> impl Future<Output = ()>;
}

/// Loads and runs the script, waits for its answer, and always unloads it: a
/// script left loaded would stay in KWin until it restarts.
async fn probe(
    kwin: &impl Scripting,
    file: &Path,
    plugin: &str,
    answer: impl Future<Output = Option<Window>>,
) -> Option<Window> {
    let id = match kwin.load(file, plugin).await {
        // KWin answers -1 when it could not load it.
        Ok(id) if id >= 0 => id,
        Ok(_) | Err(_) => {
            kwin.unload(plugin).await;
            return None;
        }
    };
    let window = match kwin.run(id).await {
        Ok(()) => tokio::time::timeout(TIMEOUT, answer).await.ok().flatten(),
        Err(error) => {
            tracing::debug!(%error, "KWin did not run the active-window script");
            None
        }
    };
    kwin.unload(plugin).await;
    window
}

/// The script, calling back `destination` at `path`. Plasma 5 names the
/// active window `activeClient`.
fn script(destination: &str, path: &str) -> String {
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    format!(
        "var w = workspace.activeWindow || workspace.activeClient;\n\
         callDBus({}, {}, {}, \"Active\",\n\
         w ? String(w.pid) : \"\", w ? String(w.resourceClass) : \"\",\n\
         w ? String(w.resourceName) : \"\", w && w.desktopFileName ? String(w.desktopFileName) : \"\");\n",
        quote(destination),
        quote(path),
        quote(INTERFACE),
    )
}

struct Bus<'a>(&'a zbus::Connection);

impl Scripting for Bus<'_> {
    async fn load(&self, file: &Path, plugin: &str) -> Result<i32, String> {
        let file = file.to_str().ok_or("the script's path is not UTF-8")?;
        self.0
            .call_method(
                Some(KWIN),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "loadScript",
                &(file, plugin),
            )
            .await
            .and_then(|reply| reply.body().deserialize::<i32>())
            .map_err(|e| e.to_string())
    }

    async fn run(&self, id: i32) -> Result<(), String> {
        // Plasma 6, then Plasma 5.
        let mut error = String::new();
        for path in [format!("/Scripting/Script{id}"), format!("/{id}")] {
            match self
                .0
                .call_method(
                    Some(KWIN),
                    path.as_str(),
                    Some("org.kde.kwin.Script"),
                    "run",
                    &(),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => error = e.to_string(),
            }
        }
        Err(error)
    }

    async fn unload(&self, plugin: &str) {
        let _ = self
            .0
            .call_method(
                Some(KWIN),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "unloadScript",
                &plugin,
            )
            .await;
    }
}

struct Answer(std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Window>>>);

#[zbus::interface(name = "librelog.ActiveWindow")]
impl Answer {
    #[zbus(name = "Active")]
    fn active(&self, pid: &str, class: &str, name: &str, desktop_file: &str) {
        if let Some(sender) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(Window::new(pid, class, name, desktop_file));
        }
    }
}

/// The active window, when KWin can say; `None` elsewhere or when it does
/// not answer in time.
pub async fn active_window() -> Option<Window> {
    if !on_kde() {
        return None;
    }
    match tokio::time::timeout(TIMEOUT * 2, ask()).await {
        Ok(Ok(window)) => window,
        Ok(Err(error)) => {
            tracing::debug!(%error, "KWin did not name the active window");
            None
        }
        Err(_) => None,
    }
}

async fn ask() -> Result<Option<Window>, String> {
    static PROBES: AtomicU64 = AtomicU64::new(0);
    let probe_id = PROBES.fetch_add(1, Ordering::Relaxed);
    let plugin = format!("librelog-active-window-{}-{probe_id}", std::process::id());
    let path = format!("/librelog/kwin/probe{probe_id}");
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let destination = connection
        .unique_name()
        .ok_or("no unique name on the session bus")?
        .to_string();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    connection
        .object_server()
        .at(path.as_str(), Answer(std::sync::Mutex::new(Some(sender))))
        .await
        .map_err(|e| e.to_string())?;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = dir.join(format!("{plugin}.js"));
    write_private(&file, &script(&destination, &path)).map_err(|e| e.to_string())?;
    let window = probe(&Bus(&connection), &file, &plugin, async {
        receiver.await.ok()
    })
    .await;
    let _ = std::fs::remove_file(&file);
    let _ = connection
        .object_server()
        .remove::<Answer, _>(path.as_str())
        .await;
    Ok(window)
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Fake {
        load: Option<i32>,
        runs: bool,
        unloaded: Mutex<Vec<String>>,
    }

    impl Scripting for Fake {
        async fn load(&self, _: &Path, _: &str) -> Result<i32, String> {
            self.load.ok_or_else(|| "no KWin".to_owned())
        }
        async fn run(&self, _: i32) -> Result<(), String> {
            if self.runs {
                Ok(())
            } else {
                Err("no such object".into())
            }
        }
        async fn unload(&self, plugin: &str) {
            self.unloaded.lock().unwrap().push(plugin.to_owned());
        }
    }

    /// Every dictation probes once or twice: a script left loaded after a
    /// failed run or a missing answer would pile up in KWin.
    #[tokio::test(start_paused = true)]
    async fn the_script_is_unloaded_whatever_kwin_answers() {
        let konsole = Window::new("42", "org.kde.konsole", "konsole", "org.kde.konsole");
        let answered = Fake {
            load: Some(3),
            runs: true,
            ..Fake::default()
        };
        let window = probe(&answered, Path::new("/s.js"), "p", async {
            Some(konsole.clone())
        })
        .await;
        assert_eq!(window, Some(konsole));
        assert_eq!(window.unwrap().pid, Some(42));

        let silent = Fake {
            load: Some(3),
            runs: true,
            ..Fake::default()
        };
        let window = probe(&silent, Path::new("/s.js"), "p", std::future::pending()).await;
        assert_eq!(window, None);

        let broken = Fake {
            load: Some(3),
            ..Fake::default()
        };
        assert_eq!(
            probe(&broken, Path::new("/s.js"), "p", std::future::pending()).await,
            None
        );
        let refused = Fake {
            load: Some(-1),
            ..Fake::default()
        };
        assert_eq!(
            probe(&refused, Path::new("/s.js"), "p", std::future::pending()).await,
            None
        );
        for fake in [answered, silent, broken, refused] {
            assert_eq!(*fake.unloaded.lock().unwrap(), ["p"]);
        }
    }
}
