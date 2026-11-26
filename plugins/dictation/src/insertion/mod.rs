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

/// Inserts a dictation where the focus is when its turn comes; returns what was typed.
pub async fn insert_dictation(
    text: String,
    mode: InsertMode,
    exclusions: Option<anlg_dictation_core::context::ContextExclusions>,
) -> Result<String, String> {
    prepared_insert_in(&LANE, &Native, async move {
        let source = source().await;
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
    /// once when it pasted nothing.
    fn paste_read(&self) -> impl Future<Output = ()> + Send;
}

async fn check_in(lane: &Mutex<()>, focus: &impl Focus) -> Result<(), String> {
    let Ok(_turn) = lane.try_lock() else {
        return Ok(());
    };
    focus.capture().await.map(drop)
}

async fn insert_in(lane: &Mutex<()>, focus: &impl Focus, text: String) -> Result<(), String> {
    prepared_insert_in(lane, focus, std::future::ready(Ok(text)))
        .await
        .map(drop)
}

/// Inserts what `prepare` settles on once the focus is read, inside the insertion's turn, so it
/// reads the field the text goes into; returns that text.
async fn prepared_insert_in(
    lane: &Mutex<()>,
    focus: &impl Focus,
    prepare: impl Future<Output = Result<String, String>>,
) -> Result<String, String> {
    let _turn = lane.lock().await;
    let target = focus.capture().await?;
    let text = prepare.await?;
    if text.is_empty() {
        return Ok(text);
    }
    focus.insert(target, text.clone()).await?;
    let _ = tokio::time::timeout(PASTE_READ_TIMEOUT, focus.paste_read()).await;
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

    async fn paste_read(&self) {
        let _ = tauri::async_runtime::spawn_blocking(|| {
            anlg_dictation_ui_macos::wait_for_paste_read(PASTE_READ_TIMEOUT)
        })
        .await;
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

    fn paste_read(&self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
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

    fn paste_read(&self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as Log};

    use tokio::sync::Notify;

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
            Ok(())
        }

        async fn paste_read(&self) {
            if let Some(read) = &self.read {
                read.notified().await;
            }
        }
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
