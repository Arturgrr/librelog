//! Every insertion, a dictation's or paste-last's, reads the focus and
//! inserts at it inside one lane, one at a time, in the order they asked.
//!
//! Reading the focus is not free of side effects everywhere. On macOS it
//! replaces the one pending insertion token, so a read while another
//! insertion waits (for the shortcut's keys to be released, up to two
//! seconds) makes that insertion fail. Windows and Linux identify the focused
//! field and refuse one that changed since it was read, so a target read
//! when a dictation started fails once you move on. Two Linux insertions at
//! once would also read the same caret. So a target is read right before its
//! own insertion, inside the lane, and the check when a capture starts runs
//! only while the lane is idle: it never waits for an insertion, so the
//! microphone opens at once, and never disturbs one.

pub mod field;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::set_data_dir;
#[cfg(target_os = "windows")]
mod windows;

use std::future::Future;
use std::time::Duration;

use tokio::sync::Mutex;

static LANE: Mutex<()> = Mutex::const_new(());

/// A macOS paste puts the text on the clipboard and posts Command-V, and the
/// app reads the clipboard a moment later; an insertion holds the lane until
/// then so the next one never replaces the text first, but no longer than
/// this, for an app that ignores the paste. It returns only afterwards, so a
/// dictation that waits for a capture to end checks right before its own
/// turn. Linux pastes the same way when AT-SPI cannot set the text, and
/// restores the clipboard before its insertion returns. Elsewhere text is
/// typed or set in place, in order.
const PASTE_READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Fails when text could not go where the focus is now; passes without
/// looking while an insertion is in flight.
pub async fn check() -> Result<(), String> {
    check_in(&LANE, &Native).await
}

/// Inserts `text` where the focus is when its turn comes.
pub async fn insert(text: String) -> Result<(), String> {
    insert_in(&LANE, &Native, text).await
}

/// How long after a dictation lands Return waits, so an app that reads a paste a moment later
/// (Electron, browsers) does not see Return first and send an empty message.
const SEND_SETTLE: Duration = Duration::from_millis(150);

/// Pressing Return once a dictation is in, so a chat or a coding agent sends it.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SendAfterInsert {
    /// App names or bundle ids (executables on Windows and Linux) where it sends, matched
    /// case-insensitively; empty sends in every app.
    pub apps: Vec<String>,
}

impl SendAfterInsert {
    /// Whether a dictation into `app` sends. With apps listed, an app that can't be told never
    /// does, and Librelog's own windows, `own`, never do: Return there could send a chat
    /// message or submit a form nobody meant to.
    fn allows(&self, app: Option<&crate::target_app::TargetApp>, own: Option<&str>) -> bool {
        if let (Some(app), Some(own)) = (app, own)
            && app
                .id
                .as_deref()
                .is_some_and(|id| app_key(id) == app_key(own))
        {
            return false;
        }
        if self.apps.is_empty() {
            return true;
        }
        let Some(app) = app else {
            return false;
        };
        let names = [Some(app.name.as_str()), app.id.as_deref()];
        self.apps
            .iter()
            .map(|listed| app_key(listed))
            .any(|listed| {
                !listed.is_empty() && names.iter().flatten().any(|name| app_key(name) == listed)
            })
    }
}

/// An app's name as listed or reported: Finder may show `Ghostty.app` and Windows `Code.exe`
/// for the app that calls itself `Ghostty` or `Code`.
fn app_key(name: &str) -> String {
    let name = name.trim().to_lowercase();
    [".app", ".exe"]
        .iter()
        .find_map(|extension| name.strip_suffix(extension))
        .unwrap_or(&name)
        .to_owned()
}

/// How a dictation's text goes into the focused field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InsertMode {
    /// Typed at the caret, spaced and cased for the text before it where that may be read.
    Fit,
    /// Typed over the selection a voice edit rewrote, only while the field still reads as it
    /// did then.
    ReplaceSelection { fingerprint: String },
}

/// Inserts a dictation where the focus is when its turn comes; returns what was typed. `send`
/// presses Return after it, never after a voice edit.
pub async fn insert_dictation(
    text: String,
    mode: InsertMode,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
    send: Option<SendAfterInsert>,
) -> Result<String, String> {
    dictation_in(&LANE, &Native, source(), text, mode, exclusions, send).await
}

async fn dictation_in(
    lane: &Mutex<()>,
    focus: &impl Focus,
    source: impl Future<Output = impl field::FieldSource>,
    text: String,
    mode: InsertMode,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
    send: Option<SendAfterInsert>,
) -> Result<String, String> {
    // A voice edit rewrote a selection: the person reads it before anything is sent.
    let send = send.filter(|_| matches!(mode, InsertMode::Fit));
    prepared_insert_in(lane, focus, send.as_ref(), async move {
        let source = source.await;
        match mode {
            InsertMode::Fit => Ok(field::fit(&source, &text, exclusions.as_ref()).await),
            InsertMode::ReplaceSelection { fingerprint } => {
                field::check_selection(&source, &fingerprint).await?;
                Ok(text)
            }
        }
    })
    .await
}

/// The focused field as a dictation starts, read only as far as `request` and the privacy
/// decision allow.
pub async fn read_field(request: field::FieldRequest) -> field::DictationField {
    field::read(&source().await, &request).await
}

#[cfg(target_os = "macos")]
async fn source() -> impl field::FieldSource {
    MacField
}

#[cfg(target_os = "linux")]
async fn source() -> impl field::FieldSource {
    linux::focused_field().await
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
async fn source() -> impl field::FieldSource {
    field::Unreadable
}

#[cfg(target_os = "macos")]
struct MacField;

#[cfg(target_os = "macos")]
impl field::FieldSource for MacField {
    async fn facts(&self) -> Option<anlg_dictation_core::context::WindowFacts> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Facts {
            bundle_id: Option<String>,
            app_name: Option<String>,
            title: Option<String>,
            secure: bool,
        }
        let facts = tauri::async_runtime::spawn_blocking(anlg_dictation_ui_macos::field_facts)
            .await
            .ok()??;
        let facts: Facts = serde_json::from_str(&facts).ok()?;
        let browser = facts
            .bundle_id
            .as_deref()
            .is_some_and(anlg_dictation_core::context::is_browser);
        Some(anlg_dictation_core::context::WindowFacts {
            bundle_id: facts.bundle_id,
            app_name: facts.app_name,
            title: facts.title,
            secure_field: Some(facts.secure),
            // Accessibility can't tell a private browser window from another, so a browser's
            // is unknown, which denies the read; other apps have no private windows.
            private_window: (!browser).then_some(false),
            browser_host: None,
        })
    }

    async fn read(&self, deep: bool) -> Option<field::FieldRead> {
        let read = tauri::async_runtime::spawn_blocking(move || {
            anlg_dictation_ui_macos::field_read(field::FIELD_WINDOW, deep)
        })
        .await
        .ok()??;
        serde_json::from_str(&read).ok()
    }

    async fn window_text(&self, max_chars: usize) -> Option<String> {
        tauri::async_runtime::spawn_blocking(move || {
            anlg_dictation_ui_macos::window_text(max_chars)
        })
        .await
        .ok()
        .flatten()
    }
}

trait Focus {
    /// Reads where text would go and checks it can; returns that target.
    fn capture(&self) -> impl Future<Output = Result<String, String>> + Send;
    fn insert(
        &self,
        target: String,
        text: String,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Resolves once the app has read what the last insertion pasted, at
    /// once when it pasted nothing; false when it never did.
    fn paste_read(&self) -> impl Future<Output = bool> + Send;
    /// The app that has keyboard focus now, for the apps a dictation sends in.
    fn app(&self) -> impl Future<Output = Option<crate::target_app::TargetApp>> + Send;
    /// Presses Return where `target`'s insertion went, only while that field (or, where fields
    /// can't be told apart, that app) still has focus and is not a password field.
    fn press_return(&self, target: String) -> impl Future<Output = Result<(), String>> + Send;
}

async fn check_in(lane: &Mutex<()>, focus: &impl Focus) -> Result<(), String> {
    let Ok(_turn) = lane.try_lock() else {
        return Ok(());
    };
    focus.capture().await.map(drop)
}

async fn insert_in(lane: &Mutex<()>, focus: &impl Focus, text: String) -> Result<(), String> {
    prepared_insert_in(lane, focus, None, std::future::ready(Ok(text)))
        .await
        .map(drop)
}

/// Inserts what `prepare` settles on once the focus is read, inside the insertion's turn, so it
/// reads the field the text goes into; returns that text. With `send`, Return follows a whole
/// insertion the app took, in its turn, so the next insertion never lands before it. A Return
/// that can't go is only logged: the text is in.
async fn prepared_insert_in(
    lane: &Mutex<()>,
    focus: &impl Focus,
    send: Option<&SendAfterInsert>,
    prepare: impl Future<Output = Result<String, String>>,
) -> Result<String, String> {
    let _turn = lane.lock().await;
    let target = focus.capture().await?;
    let text = prepare.await?;
    if text.is_empty() {
        return Ok(text);
    }
    let sends = match send {
        Some(send) => send.allows(focus.app().await.as_ref(), crate::target_app::own_id()),
        None => false,
    };
    focus.insert(target.clone(), text.clone()).await?;
    let read = tokio::time::timeout(PASTE_READ_TIMEOUT, focus.paste_read())
        .await
        .unwrap_or(false);
    if sends {
        if read {
            tokio::time::sleep(SEND_SETTLE).await;
            if let Err(reason) = focus.press_return(target).await {
                tracing::info!(%reason, "did not press Return after the dictation");
            }
        } else {
            tracing::info!("the app never read the pasted dictation; Return was not pressed");
        }
    }
    Ok(text)
}

struct Native;

#[cfg(target_os = "macos")]
impl Focus for Native {
    async fn capture(&self) -> Result<String, String> {
        tauri::async_runtime::spawn_blocking(anlg_dictation_ui_macos::capture_target)
            .await
            .map_err(|error| error.to_string())?
    }

    async fn insert(&self, target: String, text: String) -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(move || {
            anlg_dictation_ui_macos::insert_text(&target, &text)
        })
        .await
        .map_err(|error| error.to_string())?
    }

    async fn paste_read(&self) -> bool {
        tauri::async_runtime::spawn_blocking(|| {
            anlg_dictation_ui_macos::wait_for_paste_read(PASTE_READ_TIMEOUT)
        })
        .await
        .unwrap_or(false)
    }

    // The app Accessibility gives focus to, which follows panels such as Spotlight that the
    // frontmost app does not.
    async fn app(&self) -> Option<crate::target_app::TargetApp> {
        use field::FieldSource as _;
        let facts = MacField.facts().await?;
        Some(crate::target_app::TargetApp {
            name: facts.app_name.or_else(|| facts.bundle_id.clone())?,
            id: facts.bundle_id,
        })
    }

    async fn press_return(&self, _target: String) -> Result<(), String> {
        tauri::async_runtime::spawn_blocking(anlg_dictation_ui_macos::press_return)
            .await
            .map_err(|error| error.to_string())?
    }
}

#[cfg(target_os = "linux")]
impl Focus for Native {
    fn capture(&self) -> impl Future<Output = Result<String, String>> + Send {
        linux::capture()
    }

    fn insert(
        &self,
        target: String,
        text: String,
    ) -> impl Future<Output = Result<(), String>> + Send {
        linux::insert(target, text)
    }

    fn paste_read(&self) -> impl Future<Output = bool> + Send {
        std::future::ready(true)
    }

    fn app(&self) -> impl Future<Output = Option<crate::target_app::TargetApp>> + Send {
        crate::target_app::frontmost()
    }

    fn press_return(&self, target: String) -> impl Future<Output = Result<(), String>> + Send {
        linux::press_return(target)
    }
}

#[cfg(target_os = "windows")]
impl Focus for Native {
    fn capture(&self) -> impl Future<Output = Result<String, String>> + Send {
        windows::capture()
    }

    fn insert(
        &self,
        target: String,
        text: String,
    ) -> impl Future<Output = Result<(), String>> + Send {
        windows::insert(target, text)
    }

    fn paste_read(&self) -> impl Future<Output = bool> + Send {
        std::future::ready(true)
    }

    fn app(&self) -> impl Future<Output = Option<crate::target_app::TargetApp>> + Send {
        crate::target_app::frontmost()
    }

    fn press_return(&self, target: String) -> impl Future<Output = Result<(), String>> + Send {
        windows::press_return(target)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as Log};

    use tokio::sync::Notify;

    use super::field::FieldSource as _;
    use super::*;

    /// A focus where each target is a fresh token and only the newest one
    /// inserts, as on macOS. `hold` keeps an insertion waiting, as it does
    /// until the shortcut's keys are released, and `read` each paste until
    /// the app reads it.
    #[derive(Clone, Default)]
    struct Fake {
        token: Arc<Log<u32>>,
        log: Arc<Log<Vec<String>>>,
        at: Arc<Log<Vec<tokio::time::Instant>>>,
        hold: Option<Arc<Notify>>,
        read: Option<Arc<Notify>>,
        app: Option<&'static str>,
        /// The app took only part of the text.
        partial: bool,
        /// The paste never reached the app.
        unread: bool,
        /// Focus moves to another field right after the text lands.
        moves: bool,
    }

    impl Focus for Fake {
        async fn capture(&self) -> Result<String, String> {
            let mut token = self.token.lock().unwrap();
            *token += 1;
            Ok(token.to_string())
        }

        async fn insert(&self, target: String, text: String) -> Result<(), String> {
            tokio::task::yield_now().await;
            if let Some(hold) = &self.hold {
                hold.notified().await;
            }
            if target != self.token.lock().unwrap().to_string() {
                return Err(format!("{text} lost its target"));
            }
            self.log.lock().unwrap().push(text);
            self.at.lock().unwrap().push(tokio::time::Instant::now());
            if self.moves {
                *self.token.lock().unwrap() += 1;
            }
            if self.partial {
                return Err("part of it".into());
            }
            Ok(())
        }

        async fn paste_read(&self) -> bool {
            if let Some(read) = &self.read {
                read.notified().await;
            }
            !self.unread
        }

        async fn app(&self) -> Option<crate::target_app::TargetApp> {
            self.app.map(|name| crate::target_app::TargetApp {
                name: name.into(),
                id: Some(format!("{}.exe", name.to_lowercase())),
            })
        }

        async fn press_return(&self, target: String) -> Result<(), String> {
            if target != self.token.lock().unwrap().to_string() {
                return Err("focus moved".into());
            }
            self.log.lock().unwrap().push("⏎".into());
            Ok(())
        }
    }

    /// A focused field that reads `selected` as its selection.
    struct Field(&'static str);

    impl field::FieldSource for Field {
        async fn facts(&self) -> Option<anlg_dictation_core::context::WindowFacts> {
            None
        }

        async fn read(&self, _deep: bool) -> Option<field::FieldRead> {
            Some(field::FieldRead {
                length: self.0.len(),
                start: 0,
                before: String::new(),
                selected: self.0.into(),
                after: String::new(),
            })
        }

        async fn window_text(&self, _max_chars: usize) -> Option<String> {
            None
        }
    }

    async fn dictate(focus: &Fake, mode: InsertMode, apps: &[&str]) -> Vec<String> {
        let lane = Mutex::new(());
        let send = SendAfterInsert {
            apps: apps.iter().map(|app| (*app).into()).collect(),
        };
        let field = async { Field("Kubernetes") };
        let _ = dictation_in(
            &lane,
            focus,
            field,
            "Ship it.".into(),
            mode,
            None,
            Some(send),
        )
        .await;
        focus.log.lock().unwrap().clone()
    }

    #[tokio::test(start_paused = true)]
    async fn return_follows_only_a_whole_dictation_into_the_still_focused_listed_app() {
        let fit = || InsertMode::Fit;
        let slack = || Fake {
            app: Some("Slack"),
            ..Fake::default()
        };
        assert_eq!(dictate(&slack(), fit(), &[]).await, ["Ship it.", "⏎"]);
        assert_eq!(
            dictate(&slack(), fit(), &["slack.app"]).await,
            ["Ship it.", "⏎"]
        );
        assert_eq!(
            dictate(&slack(), fit(), &["SLACK.EXE"]).await,
            ["Ship it.", "⏎"]
        );
        assert_eq!(dictate(&slack(), fit(), &["Notes"]).await, ["Ship it."]);
        assert_eq!(
            dictate(&Fake::default(), fit(), &["Slack"]).await,
            ["Ship it."]
        );
        for focus in [
            Fake {
                partial: true,
                ..slack()
            },
            Fake {
                unread: true,
                ..slack()
            },
            Fake {
                moves: true,
                ..slack()
            },
        ] {
            assert_eq!(dictate(&focus, fit(), &[]).await, ["Ship it."]);
        }
        let fingerprint = Field("Kubernetes").read(true).await.unwrap().fingerprint();
        let edit = InsertMode::ReplaceSelection { fingerprint };
        assert_eq!(dictate(&slack(), edit, &[]).await, ["Ship it."]);
    }

    #[test]
    fn an_app_is_listed_by_its_name_or_id_and_neither_an_unknown_one_nor_librelog_sends() {
        use crate::target_app::TargetApp;
        let ghostty = TargetApp {
            name: "Ghostty".into(),
            id: Some("com.mitchellh.ghostty".into()),
        };
        let listed = |apps: &[&str], app: Option<&TargetApp>| {
            SendAfterInsert {
                apps: apps.iter().map(|app| (*app).into()).collect(),
            }
            .allows(app, Some("Librelog.exe"))
        };
        assert!(listed(&[], None));
        assert!(listed(&[" Ghostty.app "], Some(&ghostty)));
        assert!(listed(&["COM.MITCHELLH.GHOSTTY"], Some(&ghostty)));
        assert!(!listed(&["Ghost"], Some(&ghostty)));
        assert!(!listed(&["", "  "], Some(&ghostty)));
        assert!(!listed(&["Ghostty"], None));
        // Librelog's own windows never send, even when every app does or it is listed.
        let librelog = TargetApp {
            name: "Librelog".into(),
            id: Some("librelog.exe".into()),
        };
        assert!(!listed(&[], Some(&librelog)));
        assert!(!listed(&["Librelog"], Some(&librelog)));
    }

    #[tokio::test]
    async fn overlapping_insertions_each_land_in_the_order_they_asked() {
        static LANE: Mutex<()> = Mutex::const_new(());
        let focus = Fake::default();
        let insertions = ["First.", "Second.", "Pasted again."].map(|text| {
            let focus = focus.clone();
            tokio::spawn(async move { insert_in(&LANE, &focus, text.into()).await })
        });
        for insertion in insertions {
            insertion.await.unwrap().unwrap();
        }
        assert_eq!(
            *focus.log.lock().unwrap(),
            ["First.", "Second.", "Pasted again."]
        );
    }

    #[tokio::test]
    async fn a_capture_starting_mid_insertion_neither_waits_nor_voids_it() {
        static LANE: Mutex<()> = Mutex::const_new(());
        let hold = Arc::new(Notify::new());
        let focus = Fake {
            hold: Some(hold.clone()),
            ..Fake::default()
        };
        let waiting = tokio::spawn({
            let focus = focus.clone();
            async move { insert_in(&LANE, &focus, "First.".into()).await }
        });
        while LANE.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }

        tokio::time::timeout(Duration::from_millis(100), check_in(&LANE, &focus))
            .await
            .unwrap()
            .unwrap();
        hold.notify_one();
        waiting.await.unwrap().unwrap();
        assert_eq!(*focus.log.lock().unwrap(), ["First."]);
    }

    /// A fixed wait after a paste let the next insertion replace the
    /// clipboard before a slow app read it, which then pasted the wrong text.
    #[tokio::test(start_paused = true)]
    async fn the_next_insertion_waits_for_a_paste_to_be_read_but_not_past_the_timeout() {
        static LANE: Mutex<()> = Mutex::const_new(());
        let read = Arc::new(Notify::new());
        let focus = Fake {
            read: Some(read.clone()),
            ..Fake::default()
        };
        let late = Duration::from_millis(1_500);
        tokio::spawn(async move {
            tokio::time::sleep(late).await;
            read.notify_one();
        });
        let insertions = ["Read late.", "Never read.", "Third."].map(|text| {
            let focus = focus.clone();
            tokio::spawn(async move { insert_in(&LANE, &focus, text.into()).await })
        });
        for insertion in insertions {
            insertion.await.unwrap().unwrap();
        }
        let at = focus.at.lock().unwrap();
        assert!(at[1] - at[0] >= late && at[1] - at[0] < PASTE_READ_TIMEOUT);
        assert!(at[2] - at[1] >= PASTE_READ_TIMEOUT);
    }
}
