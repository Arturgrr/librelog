//! The focused field through AT-SPI, and inserting into it with
//! `EditableText`, which needs no permission and leaves the clipboard alone.

use atspi::proxy::{
    accessible::{AccessibleProxy, ObjectRefExt},
    editable_text::EditableTextProxy,
    text::TextProxy,
};
use atspi::{AccessibilityConnection, ObjectRef, Role, State};

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
    match tokio::time::timeout(std::time::Duration::from_secs(3), find_focus()).await {
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
        let name = match application.clone().into_accessible_proxy(bus).await {
            Ok(proxy) => proxy.name().await.unwrap_or_default(),
            Err(_) => continue,
        };
        let pid = process(&connection, application.name.as_str()).await;
        apps.push(App {
            name,
            executable: pid.and_then(executable),
            pid,
        });
        pending.push((application, apps.len() - 1));
    }
    let mut visited = std::collections::HashSet::new();
    let mut active = None;
    while let Some((object, app)) = pending.pop() {
        if !visited.insert((object.name.to_string(), object.path.to_string())) {
            continue;
        }
        let Ok(proxy) = object.clone().into_accessible_proxy(bus).await else {
            continue;
        };
        let Ok(states) = proxy.get_state().await else {
            continue;
        };
        if states.contains(State::Focused) {
            let app = apps[app].clone();
            if proxy.get_role().await.ok() == Some(Role::PasswordText) {
                return Ok(Focus::Password);
            }
            if !states.contains(State::Editable) {
                return Ok(Focus::Other { app });
            }
            return Ok(Focus::Editable { object, app });
        }
        let role = proxy.get_role().await.ok();
        if matches!(role, Some(Role::Frame | Role::Window | Role::Dialog)) {
            if !states.contains(State::Active) {
                continue;
            }
            active.get_or_insert(app);
        }
        if let Ok(children) = proxy.get_children().await {
            pending.extend(children.into_iter().map(|child| (child, app)));
        }
    }
    Ok(Focus::Unknown {
        app: active.map(|app| apps[app].clone()),
    })
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

pub enum Inserted {
    Yes,
    /// Focus moved to another element since the target was read.
    FocusChanged,
    Password,
    /// The field is still focused but did not take the text; pasting may.
    Refused(String),
}

pub async fn insert(object: ObjectRef, text: &str) -> Inserted {
    match tokio::time::timeout(std::time::Duration::from_secs(3), insert_into(object, text)).await {
        Ok(Ok(inserted)) => inserted,
        Ok(Err(error)) => Inserted::Refused(error),
        Err(_) => Inserted::Refused("AT-SPI timed out".into()),
    }
}

async fn insert_into(object: ObjectRef, text: &str) -> Result<Inserted, String> {
    match find_focus().await? {
        Focus::Editable {
            object: focused, ..
        } if focused == object => {}
        Focus::Password => return Ok(Inserted::Password),
        _ => return Ok(Inserted::FocusChanged),
    }
    let connection = AccessibilityConnection::new()
        .await
        .map_err(|e| e.to_string())?;
    let editable = EditableTextProxy::builder(connection.connection())
        .destination(object.name.clone())
        .map_err(|e| e.to_string())?
        .path(object.path.clone())
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let text_proxy = TextProxy::builder(connection.connection())
        .destination(object.name)
        .map_err(|e| e.to_string())?
        .path(object.path)
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())?;
    // A paste replaces a selection, as the other platforms' insertions do.
    if text_proxy
        .get_nselections()
        .await
        .map_err(|e| e.to_string())?
        > 0
    {
        return Ok(Inserted::Refused("the field has a selection".into()));
    }
    let caret = text_proxy.caret_offset().await.map_err(|e| e.to_string())?;
    let count = text_proxy.character_count().await.ok();
    if !editable
        .insert_text(caret, text, text.len() as i32)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Inserted::Refused("the field refused the text".into()));
    }
    // Some apps report success and ignore the edit.
    if count.is_some() && text_proxy.character_count().await.ok() == count {
        return Ok(Inserted::Refused(
            "the field reported an edit it ignored".into(),
        ));
    }
    let _ = text_proxy
        .set_caret_offset(caret + text.chars().count() as i32)
        .await;
    Ok(Inserted::Yes)
}
