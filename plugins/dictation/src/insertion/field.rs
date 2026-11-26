//! Reads of the focused field and its window for a dictation: the selection a voice edit
//! rewrites, the text before the caret a dictation is fitted to, and the window's text that
//! helps cleanup spell its names. Every privacy fact is settled before any text is read: a
//! secure field, a password manager, a private window or an excluded app is never read, and
//! window text only when the person turned context on and the window may be read.

use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};

use anlg_dictation_core::context::{
    ContextAccess, ContextExclusions, ContextSnapshot, WindowFacts, decide_context,
    may_read_for_fitting,
};
use anlg_dictation_core::corrections::FieldText;
use anlg_dictation_core::terms::OrdinaryWords;
use anlg_dictation_core::{CaretFit, fit_to_caret};

/// How much of the field on each side of the selection a read keeps.
pub const FIELD_WINDOW: usize = 4_000;
/// The longest selection a voice edit may send to the person's model.
const MAX_SELECTION_CHARS: usize = 20_000;
/// How much window text is read for a context snapshot, which keeps less.
pub const MAX_WINDOW_TEXT_CHARS: usize = 4_000;

pub const SELECTION_CHANGED: &str =
    "The selected text changed before the edit was ready. It was left unchanged.";

/// The focused field around its selection, as read now, bounded by [`FIELD_WINDOW`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRead {
    /// The whole value's length, in the platform's own units.
    pub length: usize,
    /// Where the selection starts, in the same units.
    pub start: usize,
    pub before: String,
    pub selected: String,
    pub after: String,
}

impl FieldRead {
    /// Tells whether the field still reads the same when the edit is written.
    pub fn fingerprint(&self) -> String {
        let mut hasher = DefaultHasher::new();
        (
            self.length,
            self.start,
            &self.before,
            &self.selected,
            &self.after,
        )
            .hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// What the focused field and its window may be read for.
pub trait FieldSource {
    /// The app, window and field, or `None` when they can't be told.
    fn facts(&self) -> impl Future<Output = Option<WindowFacts>> + Send;
    /// The field around its selection. `deep` also reads apps that are only read for an
    /// option the person turned on (web views, whose accessibility trees cost to build).
    fn read(&self, deep: bool) -> impl Future<Output = Option<FieldRead>> + Send;
    /// The window's static text, the newest `max_chars` of it.
    fn window_text(&self, max_chars: usize) -> impl Future<Output = Option<String>> + Send;
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldRequest {
    pub operation_id: String,
    /// The person's opt-in to window context for cleanup.
    pub context: bool,
    /// The person's opt-in to editing a selection by voice.
    pub selection: bool,
    /// `None` while the exclusions can't be read, which denies every read.
    pub exclusions: Option<ContextExclusions>,
}

/// The focused field as a dictation starts. Empty where it may not be read.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DictationField {
    /// The text around the caret, for learning what the person kept of a dictation.
    pub text: Option<FieldText>,
    /// The selected text, when a voice edit may rewrite it.
    pub selection: Option<String>,
    /// Pass to `insert_dictation` so the edit is written only over the same selection.
    pub fingerprint: Option<String>,
    /// What cleanup may use of the window.
    pub context: Option<ContextSnapshot>,
}

pub async fn read(source: &impl FieldSource, request: &FieldRequest) -> DictationField {
    let exclusions = request.exclusions.as_ref();
    let Some(facts) = source.facts().await else {
        return DictationField::default();
    };
    if !may_read_for_fitting(&facts, exclusions) {
        return DictationField::default();
    }
    let Some(read) = source.read(request.context || request.selection).await else {
        return DictationField::default();
    };
    let access = decide_context(true, &facts, exclusions);
    let allowed = access == ContextAccess::Allowed;
    let edit = request.selection
        && allowed
        && !read.selected.is_empty()
        && read.selected.chars().count() <= MAX_SELECTION_CHARS;
    let context = if request.context && allowed {
        let window = source.window_text(MAX_WINDOW_TEXT_CHARS).await;
        ContextSnapshot::collect(
            &request.operation_id,
            now_ms(),
            &access,
            &facts,
            Some(&read.before),
            window.as_deref(),
            ordinary_words(),
        )
    } else {
        None
    };
    DictationField {
        fingerprint: edit.then(|| read.fingerprint()),
        selection: edit.then(|| read.selected.clone()),
        text: Some(FieldText {
            before: read.before,
            selected: read.selected,
            after: read.after,
        }),
        context,
    }
}

/// `text` spaced and cased for where the caret is now, or as given where the field may not be
/// read. A dictation never goes over text before the caret, so it never joins a sentence.
pub async fn fit(
    source: &impl FieldSource,
    text: &str,
    exclusions: Option<&ContextExclusions>,
) -> String {
    let before = match source.facts().await {
        Some(facts) if may_read_for_fitting(&facts, exclusions) => {
            source.read(false).await.map(|read| read.before)
        }
        _ => None,
    };
    let CaretFit { text, .. } = fit_to_caret(text, before.as_deref(), false);
    text
}

/// Fails unless the field still has the selection `fingerprint` was read from.
pub async fn check_selection(source: &impl FieldSource, fingerprint: &str) -> Result<(), String> {
    match source.read(true).await {
        Some(read) if read.fingerprint() == fingerprint => Ok(()),
        _ => Err(SELECTION_CHANGED.into()),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The system word list, which tells ordinary capitalized words from names. Without one,
/// capitalized words are left out of extracted terms.
pub fn ordinary_words() -> Option<&'static OrdinaryWords> {
    static WORDS: std::sync::LazyLock<Option<OrdinaryWords>> = std::sync::LazyLock::new(|| {
        std::fs::read_to_string("/usr/share/dict/words")
            .ok()
            .map(|lines| OrdinaryWords::from_lines(&lines))
    });
    WORDS.as_ref()
}

/// Where nothing can be read: Windows, whose dictation types at the caret as said.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct Unreadable;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl FieldSource for Unreadable {
    fn facts(&self) -> impl Future<Output = Option<WindowFacts>> + Send {
        std::future::ready(None)
    }

    fn read(&self, _deep: bool) -> impl Future<Output = Option<FieldRead>> + Send {
        std::future::ready(None)
    }

    fn window_text(&self, _max_chars: usize) -> impl Future<Output = Option<String>> + Send {
        std::future::ready(None)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Fake {
        facts: Option<WindowFacts>,
        read: Option<FieldRead>,
        window: Option<String>,
        reads: Mutex<Vec<&'static str>>,
    }

    impl FieldSource for Fake {
        async fn facts(&self) -> Option<WindowFacts> {
            self.facts.clone()
        }

        async fn read(&self, _deep: bool) -> Option<FieldRead> {
            self.reads.lock().unwrap().push("field");
            self.read.clone()
        }

        async fn window_text(&self, _max_chars: usize) -> Option<String> {
            self.reads.lock().unwrap().push("window");
            self.window.clone()
        }
    }

    fn notes() -> WindowFacts {
        WindowFacts {
            bundle_id: Some("com.apple.Notes".into()),
            app_name: Some("Notes".into()),
            title: Some("Launch plan".into()),
            secure_field: Some(false),
            private_window: Some(false),
            browser_host: None,
        }
    }

    fn field(before: &str, selected: &str) -> FieldRead {
        FieldRead {
            length: before.len() + selected.len(),
            start: before.len(),
            before: before.into(),
            selected: selected.into(),
            after: String::new(),
        }
    }

    fn request(exclusions: ContextExclusions) -> FieldRequest {
        FieldRequest {
            operation_id: "op".into(),
            context: true,
            selection: true,
            exclusions: Some(exclusions),
        }
    }

    /// Context and voice edit read the window and field only once the privacy decision allows
    /// it: an excluded app, or a window whose privacy can't be told, is never read.
    #[tokio::test]
    async fn a_window_context_denies_is_never_read() {
        let excluded = Fake {
            facts: Some(notes()),
            read: Some(field("Ship it with ", "Kubernetes")),
            window: Some("Mina Okafor".into()),
            ..Fake::default()
        };
        let exclusions = ContextExclusions {
            apps: vec!["notes".into()],
            websites: Vec::new(),
        };
        let denied = read(&excluded, &request(exclusions)).await;
        assert!(denied.text.is_none() && denied.selection.is_none() && denied.context.is_none());
        assert!(excluded.reads.lock().unwrap().is_empty());

        let browser = Fake {
            facts: Some(WindowFacts {
                bundle_id: Some("com.google.Chrome".into()),
                private_window: None,
                ..notes()
            }),
            ..Fake::default()
        };
        read(&browser, &request(ContextExclusions::default())).await;
        assert!(browser.reads.lock().unwrap().is_empty());

        let allowed = Fake {
            facts: Some(notes()),
            read: Some(field("Ship it with ", "Kubernetes")),
            window: Some("Mina Okafor reviewed the Kubernetes rollout".into()),
            ..Fake::default()
        };
        let read = read(&allowed, &request(ContextExclusions::default())).await;
        assert_eq!(read.selection.as_deref(), Some("Kubernetes"));
        assert!(read.context.is_some_and(|context| context.screen.is_some()));
    }

    #[tokio::test]
    async fn an_edit_is_written_only_over_the_selection_it_was_read_from() {
        let source = Fake {
            facts: Some(notes()),
            read: Some(field("Ship it with ", "Kubernetes")),
            ..Fake::default()
        };
        let fingerprint = read(
            &source,
            &FieldRequest {
                context: false,
                ..request(ContextExclusions::default())
            },
        )
        .await
        .fingerprint
        .unwrap();
        assert_eq!(check_selection(&source, &fingerprint).await, Ok(()));
        let changed = Fake {
            read: Some(field("Ship it with ", "Nomad")),
            ..source
        };
        assert!(check_selection(&changed, &fingerprint).await.is_err());
    }
}
