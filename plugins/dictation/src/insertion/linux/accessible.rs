//! The focused field through AT-SPI, and inserting into it with
//! `EditableText`, which needs no permission and leaves the clipboard alone.

use atspi::proxy::{
    accessible::{AccessibleProxy, ObjectRefExt},
    application::ApplicationProxy,
    editable_text::EditableTextProxy,
    text::TextProxy,
};
use atspi::{AccessibilityConnection, Interface, ObjectRef, Role, State};
use std::time::Duration;
use zbus::proxy::CacheProperties;

use super::route::App;

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
