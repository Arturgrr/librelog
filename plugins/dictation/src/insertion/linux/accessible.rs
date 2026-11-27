//! The focused field through AT-SPI, and inserting into it with
//! `EditableText`, which needs no permission and leaves the clipboard alone.

use anlg_dictation_core::context::{WindowFacts, is_browser};
use atspi::proxy::{
    accessible::{AccessibleProxy, ObjectRefExt},
    application::ApplicationProxy,
    editable_text::EditableTextProxy,
    text::TextProxy,
};
use atspi::{AccessibilityConnection, Interface, ObjectRef, Role, State};
use std::future::Future;
use std::time::Duration;
use zbus::proxy::CacheProperties;

use super::route::App;
use crate::insertion::field::FieldRead;

/// Where the keyboard focus is, as far as AT-SPI tells.
pub enum Focus {
    Editable {
        object: ObjectRef,
        app: App,
    },
    Password,
    /// Focused but not editable: a terminal's screen, a button.
    Other {
        app: App,
    },
    /// No focused element: the app exposes none, or AT-SPI is off. The app
    /// of the active window, when one says it is active.
    Unknown {
        app: Option<App>,
    },
}

pub async fn focus() -> Focus {
    match tokio::time::timeout(Duration::from_secs(3), find_focus()).await {
        Ok(Ok(focus)) => focus,
        Ok(Err(error)) => {
            tracing::info!(%error, "AT-SPI is unavailable");
            Focus::Unknown { app: None }
        }
        Err(_) => {
            tracing::info!("finding the focused field through AT-SPI timed out");
            Focus::Unknown { app: None }
        }
    }
}

async fn find_focus() -> Result<Focus, String> {
    let connection = AccessibilityConnection::new()
        .await
        .map_err(|e| e.to_string())?;
    let bus = connection.connection();
    let root = AccessibleProxy::builder(bus)
        .destination("org.a11y.atspi.Registry")
        .map_err(|e| e.to_string())?
        .path("/org/a11y/atspi/accessible/root")
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let mut apps = Vec::new();
    let mut pending = Vec::new();
    for application in root.get_children().await.map_err(|e| e.to_string())? {
        let Ok(Some(app)) =
            tokio::time::timeout(NODE_TIMEOUT, describe(&connection, &application)).await
        else {
            continue;
        };
        apps.push(app);
        pending.push((application, apps.len() - 1));
    }
    let mut visited = std::collections::HashSet::new();
    let mut active = None;
    while let Some((object, app)) = pending.pop() {
        if !visited.insert(object.clone()) {
            continue;
        }
        // A stalled app should not use up the whole search.
        let Ok(Ok(node)) = tokio::time::timeout(NODE_TIMEOUT, inspect(bus, &object)).await else {
            continue;
        };
        match node {
            Node::Focused(field) => {
                let app = apps[app].clone();
                return Ok(match field {
                    Field::Password => Focus::Password,
                    Field::Editable => Focus::Editable { object, app },
                    Field::Other => Focus::Other { app },
                });
            }
            Node::Branch {
                children,
                active_window,
            } => {
                if active_window {
                    active.get_or_insert(app);
                }
                pending.extend(children.into_iter().map(|child| (child, app)));
            }
        }
    }
    Ok(Focus::Unknown {
        app: active.map(|app| apps[app].clone()),
    })
}

/// How long one app or element gets to answer during the search.
const NODE_TIMEOUT: Duration = Duration::from_millis(250);

enum Field {
    Editable,
    Password,
    Other,
}

enum Node {
    Focused(Field),
    /// Not focused: the children to search, none for an inactive window.
    Branch {
        children: Vec<ObjectRef>,
        active_window: bool,
    },
}

async fn describe(connection: &AccessibilityConnection, application: &ObjectRef) -> Option<App> {
    let name = application
        .as_accessible_proxy(connection.connection())
        .await
        .ok()?
        .name()
        .await
        .unwrap_or_default();
    let pid = process(connection, application.name.as_str()).await;
    Some(App {
        name,
        executable: pid.and_then(executable),
        pid,
        toolkit: toolkit(connection, application).await,
        class: None,
    })
}

async fn inspect(bus: &zbus::Connection, object: &ObjectRef) -> zbus::Result<Node> {
    let proxy = object.as_accessible_proxy(bus).await?;
    let states = proxy.get_state().await?;
    if states.contains(State::Focused) {
        if proxy.get_role().await.ok() == Some(Role::PasswordText) {
            return Ok(Node::Focused(Field::Password));
        }
        // An editable state without the interfaces would only fail when
        // inserting; such a field gets the paste keys instead.
        let editable = states.contains(State::Editable)
            && proxy.get_interfaces().await.is_ok_and(|interfaces| {
                interfaces.contains(Interface::EditableText) && interfaces.contains(Interface::Text)
            });
        return Ok(Node::Focused(if editable {
            Field::Editable
        } else {
            Field::Other
        }));
    }
    let window = matches!(
        proxy.get_role().await,
        Ok(Role::Frame | Role::Window | Role::Dialog)
    );
    if window && !states.contains(State::Active) {
        return Ok(Node::Branch {
            children: Vec::new(),
            active_window: false,
        });
    }
    Ok(Node::Branch {
        children: proxy.get_children().await.unwrap_or_default(),
        active_window: window,
    })
}

/// The toolkit the app reports (`Chromium`, `Gecko`, `GTK`, `Qt`), which
/// names Flatpak browsers and Electron apps their executable does not.
async fn toolkit(connection: &AccessibilityConnection, application: &ObjectRef) -> Option<String> {
    ApplicationProxy::builder(connection.connection())
        .destination(application.name.clone())
        .ok()?
        .path(application.path.clone())
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?
        .toolkit_name()
        .await
        .ok()
}

async fn process(connection: &AccessibilityConnection, owner: &str) -> Option<u32> {
    connection
        .connection()
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &owner,
        )
        .await
        .ok()?
        .body()
        .deserialize::<u32>()
        .ok()
}

fn executable(process: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{process}/exe")).ok()
}

/// The focused field, found once, for the reads one dictation makes.
pub struct FocusedField {
    focus: Focus,
}

pub async fn focused_field() -> FocusedField {
    FocusedField {
        focus: focus().await,
    }
}

impl crate::insertion::field::FieldSource for FocusedField {
    async fn facts(&self) -> Option<WindowFacts> {
        let (object, app) = match &self.focus {
            Focus::Editable { object, app } => (object, app),
            Focus::Password => {
                return Some(WindowFacts {
                    secure_field: Some(true),
                    ..WindowFacts::default()
                });
            }
            Focus::Other { .. } | Focus::Unknown { .. } => return None,
        };
        let name = Some(app.name.trim()).filter(|name| !name.is_empty());
        // The executable stands in for a bundle id, as on Windows; a Flatpak's is its sandbox
        // launcher, so its name ("Firefox") is tried as a desktop app id too.
        let candidates = [
            app.executable
                .as_deref()
                .and_then(std::path::Path::file_name)
                .map(|file| file.to_string_lossy().to_lowercase()),
            name.map(|name| name.to_lowercase().replace(' ', "-")),
        ];
        let browser = candidates.iter().flatten().find(|id| is_browser(id));
        let bundle_id = browser.or(candidates.iter().flatten().next()).cloned();
        let title = with_timeout(window_title(object)).await.flatten();
        Some(WindowFacts {
            bundle_id,
            app_name: name.map(str::to_owned),
            title,
            secure_field: Some(false),
            private_window: browser.is_none().then_some(false),
            browser_host: None,
        })
    }

    async fn read(&self, _deep: bool) -> Option<FieldRead> {
        let Focus::Editable { object, .. } = &self.focus else {
            return None;
        };
        with_timeout(read_text(object)).await.flatten()
    }

    async fn window_text(&self, max_chars: usize) -> Option<String> {
        let Focus::Editable { object, .. } = &self.focus else {
            return None;
        };
        with_timeout(window_text(object, max_chars)).await.flatten()
    }
}

async fn with_timeout<T>(read: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(STEP_TIMEOUT, read).await.ok()
}

async fn window_title(object: &ObjectRef) -> Option<String> {
    let connection = AccessibilityConnection::new().await.ok()?;
    let bus = connection.connection();
    let window = window_of(bus, object).await?;
    window
        .as_accessible_proxy(bus)
        .await
        .ok()?
        .name()
        .await
        .ok()
        .filter(|name| !name.trim().is_empty())
}

/// The frame, window or dialog `object` is in.
async fn window_of(bus: &zbus::Connection, object: &ObjectRef) -> Option<ObjectRef> {
    let mut node = object.clone();
    for _ in 0..32 {
        let parent = node
            .as_accessible_proxy(bus)
            .await
            .ok()?
            .parent()
            .await
            .ok()?;
        let role = parent.as_accessible_proxy(bus).await.ok()?.get_role().await;
        if matches!(role, Ok(Role::Frame | Role::Window | Role::Dialog)) {
            return Some(parent);
        }
        node = parent;
    }
    None
}

/// The field around its selection, in characters, as AT-SPI counts them.
async fn read_text(object: &ObjectRef) -> Option<FieldRead> {
    let connection = AccessibilityConnection::new().await.ok()?;
    let (text, _) = field(connection.connection(), object).await.ok()?;
    let length = text.character_count().await.ok()?;
    let (start, end) = if selection_count(&text).await.ok()? > 0 {
        let (a, b) = text.get_selection(0).await.ok()?;
        (a.min(b), a.max(b))
    } else {
        let caret = text.caret_offset().await.ok()?;
        (caret, caret)
    };
    let (start, end) = (start.clamp(0, length), end.clamp(0, length));
    let window = crate::insertion::field::FIELD_WINDOW as i32;
    let from = start.saturating_sub(window).max(0);
    let to = end.saturating_add(window).min(length);
    let value: Vec<char> = text.get_text(from, to).await.ok()?.chars().collect();
    // A bridge counting other units than characters (Qt's UTF-16) would split the text wrong.
    if value.len() != (to - from) as usize {
        return None;
    }
    let part = |a: i32, b: i32| {
        value[(a - from) as usize..(b - from) as usize]
            .iter()
            .collect()
    };
    Some(FieldRead {
        length: length as usize,
        start: start as usize,
        before: part(from, start),
        selected: part(start, end),
        after: part(end, to),
    })
}

/// How many elements of a window are read for its text, and how long that may take.
const WINDOW_NODES: usize = 800;
const WINDOW_TEXT_TIME: Duration = Duration::from_millis(250);

/// The labels and text of the field's window in reading order, the newest `max_chars`. Password
/// fields and the field itself are skipped.
async fn window_text(object: &ObjectRef, max_chars: usize) -> Option<String> {
    let connection = AccessibilityConnection::new().await.ok()?;
    let bus = connection.connection();
    let window = window_of(bus, object).await?;
    let deadline = tokio::time::Instant::now() + WINDOW_TEXT_TIME;
    let mut parts = Vec::new();
    let mut pending = vec![(window, 0)];
    let mut visited = 0;
    while let Some((node, depth)) = pending.pop() {
        if visited >= WINDOW_NODES || tokio::time::Instant::now() >= deadline {
            break;
        }
        visited += 1;
        if &node == object || depth > 30 {
            continue;
        }
        let Ok(Ok(proxy)) = tokio::time::timeout(NODE_TIMEOUT, node.as_accessible_proxy(bus)).await
        else {
            continue;
        };
        let Ok(Ok(role)) = tokio::time::timeout(NODE_TIMEOUT, proxy.get_role()).await else {
            continue;
        };
        if role == Role::PasswordText {
            continue;
        }
        if matches!(
            role,
            Role::Label | Role::Static | Role::Heading | Role::Paragraph | Role::Caption
        ) && let Ok(Ok(name)) = tokio::time::timeout(NODE_TIMEOUT, proxy.name()).await
            && !name.trim().is_empty()
        {
            parts.push(name.trim().to_owned());
        }
        let Ok(Ok(children)) = tokio::time::timeout(NODE_TIMEOUT, proxy.get_children()).await
        else {
            continue;
        };
        // Popped from the end: the first child is read first.
        pending.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    let text = parts.join("\n");
    let skip = text.chars().count().saturating_sub(max_chars);
    Some(text.chars().skip(skip).collect()).filter(|text: &String| !text.is_empty())
}

/// How long an edit the field accepted gets to show in its text.
const LANDING_WAIT: Duration = Duration::from_millis(300);

#[derive(Debug, PartialEq)]
pub enum Inserted {
    Yes,
    /// Focus moved to another element since the target was read.
    FocusChanged,
    Password,
    /// The field is still focused and its text unchanged; pasting may work.
    Refused(String),
    /// The field changed, but not by exactly the dictation (a length limit
    /// kept part of it), or could not be read back: pasting could insert it
    /// twice.
    Incomplete(String),
}

pub async fn insert(object: ObjectRef, text: &str) -> Inserted {
    match tokio::time::timeout(Duration::from_secs(3), find_focus()).await {
        Ok(Ok(Focus::Editable {
            object: focused, ..
        })) if focused == object => {}
        Ok(Ok(Focus::Password)) => return Inserted::Password,
        Ok(Ok(_)) => return Inserted::FocusChanged,
        Ok(Err(error)) => return Inserted::Refused(error),
        Err(_) => return Inserted::Refused("AT-SPI timed out".into()),
    }
    let connection = match AccessibilityConnection::new().await {
        Ok(connection) => connection,
        Err(error) => return Inserted::Refused(error.to_string()),
    };
    match field(connection.connection(), &object).await {
        Ok((text_proxy, editable)) => write(&text_proxy, &editable, text).await,
        Err(error) => Inserted::Refused(error.to_string()),
    }
}

async fn field<'a>(
    bus: &zbus::Connection,
    object: &ObjectRef,
) -> zbus::Result<(TextProxy<'a>, EditableTextProxy<'a>)> {
    // The landing check reads `CharacterCount` again after the edit; a
    // cached value never changes, so the edit would look ignored and be
    // pasted a second time. Some bridges also lack GetAll.
    let text = TextProxy::builder(bus)
        .destination(object.name.clone())?
        .path(object.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    let editable = EditableTextProxy::builder(bus)
        .destination(object.name.clone())?
        .path(object.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;
    Ok((text, editable))
}

/// atspi-proxies 0.9 calls `GetNselections`, which no AT-SPI bridge
/// answers: the method is `GetNSelections`.
async fn selection_count(text: &TextProxy<'_>) -> zbus::Result<i32> {
    text.inner().call("GetNSelections", &()).await
}

/// How long the field gets to answer each step of a write.
const STEP_TIMEOUT: Duration = Duration::from_secs(1);

async fn write(
    text_proxy: &TextProxy<'_>,
    editable: &EditableTextProxy<'_>,
    text: &str,
) -> Inserted {
    let before = async {
        // A paste replaces a selection, as the other platforms' insertions do.
        if selection_count(text_proxy).await? > 0 {
            return Ok(None);
        }
        Ok::<_, zbus::Error>(Some((
            text_proxy.caret_offset().await?,
            text_proxy.character_count().await?,
        )))
    };
    let (caret, count) = match tokio::time::timeout(STEP_TIMEOUT, before).await {
        Ok(Ok(Some(state))) => state,
        Ok(Ok(None)) => return Inserted::Refused("the field has a selection".into()),
        Ok(Err(error)) => return Inserted::Refused(error.to_string()),
        Err(_) => return Inserted::Refused("the field did not answer".into()),
    };
    // The length is in bytes, as GTK reads it. Qt reads UTF-16 units, so
    // Qt apps are pasted into instead: see `route`.
    let edit = editable.insert_text(caret, text, text.len() as i32);
    match tokio::time::timeout(STEP_TIMEOUT, edit).await {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => return Inserted::Refused("the field refused the text".into()),
        Ok(Err(error)) => return Inserted::Refused(error.to_string()),
        // It may still apply the edit.
        Err(_) => return Inserted::Incomplete("the field did not answer the edit".into()),
    }
    // Some apps report success and ignore the edit; others apply it a
    // moment later, and pasting then would insert the text twice. A field
    // with a length limit keeps only part of it.
    let inserted = text.chars().count() as i32;
    let expected = count.checked_add(inserted);
    let deadline = tokio::time::Instant::now() + LANDING_WAIT;
    loop {
        let now = tokio::time::timeout(STEP_TIMEOUT, text_proxy.character_count())
            .await
            .ok()
            .and_then(Result::ok);
        if now.is_some() && now == expected {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return if now == Some(count) {
                Inserted::Refused("the field reported an edit it ignored".into())
            } else {
                Inserted::Incomplete(format!(
                    "the field has {now:?} characters, not {expected:?}"
                ))
            };
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let _ = tokio::time::timeout(STEP_TIMEOUT, text_proxy.set_caret_offset(caret + inserted)).await;
    Inserted::Yes
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A GTK entry as its AT-SPI bridge serves it: methods named as the
    /// spec names them, `InsertText`'s length in bytes, and no
    /// `PropertiesChanged` when the text changes.
    #[derive(Default)]
    struct Entry {
        text: Vec<char>,
        caret: i32,
        max_length: Option<usize>,
    }

    struct Text(Arc<Mutex<Entry>>);
    struct Editable(Arc<Mutex<Entry>>);

    #[zbus::interface(name = "org.a11y.atspi.Text")]
    impl Text {
        #[zbus(name = "GetNSelections")]
        fn n_selections(&self) -> i32 {
            0
        }

        fn set_caret_offset(&self, offset: i32) -> bool {
            self.0.lock().unwrap().caret = offset;
            true
        }

        #[zbus(property)]
        fn caret_offset(&self) -> i32 {
            self.0.lock().unwrap().caret
        }

        #[zbus(property)]
        fn character_count(&self) -> i32 {
            self.0.lock().unwrap().text.len() as i32
        }
    }

    #[zbus::interface(name = "org.a11y.atspi.EditableText")]
    impl Editable {
        fn insert_text(&self, position: i32, text: &str, length: i32) -> bool {
            let mut entry = self.0.lock().unwrap();
            let room = entry
                .max_length
                .map_or(usize::MAX, |max| max - entry.text.len());
            let text: Vec<char> = text[..length as usize].chars().take(room).collect();
            let at = position as usize;
            entry.text.splice(at..at, text);
            true
        }
    }

    async fn write_into(entry: Entry, dictation: &str) -> (Inserted, String, i32) {
        let entry = Arc::new(Mutex::new(entry));
        let (app, client) = tokio::net::UnixStream::pair().unwrap();
        let (_app, client) = tokio::try_join!(
            zbus::connection::Builder::unix_stream(app)
                .server(zbus::Guid::generate())
                .unwrap()
                .p2p()
                .serve_at("/field", Text(entry.clone()))
                .unwrap()
                .serve_at("/field", Editable(entry.clone()))
                .unwrap()
                .build(),
            zbus::connection::Builder::unix_stream(client).p2p().build(),
        )
        .unwrap();
        let object: ObjectRef =
            serde_json::from_value(serde_json::json!([":1.5", "/field"])).unwrap();
        let (text, editable) = field(&client, &object).await.unwrap();
        let inserted = write(&text, &editable, dictation).await;
        let entry = entry.lock().unwrap();
        (inserted, entry.text.iter().collect(), entry.caret)
    }

    /// Every insert called `GetNselections`, failed and pasted; with that
    /// fixed, a cached `CharacterCount` made a landed edit look ignored, so
    /// it was pasted a second time.
    #[tokio::test]
    async fn an_edit_the_field_applied_is_not_pasted_again() {
        let entry = Entry {
            text: "Hello world".chars().collect(),
            caret: 5,
            ..Entry::default()
        };
        assert_eq!(
            write_into(entry, ", 世界").await,
            (Inserted::Yes, "Hello, 世界 world".into(), 9)
        );
    }

    #[tokio::test]
    async fn a_field_that_kept_part_of_the_dictation_is_not_pasted_into() {
        let entry = Entry {
            max_length: Some(4),
            ..Entry::default()
        };
        let (inserted, text, _) = write_into(entry, "Hello").await;
        assert!(matches!(inserted, Inserted::Incomplete(_)));
        assert_eq!(text, "Hell");
    }
}
