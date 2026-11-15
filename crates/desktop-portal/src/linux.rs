use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use ashpd::zbus::{self, zvariant};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const REGISTRY: &str = "org.freedesktop.host.portal.Registry";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

static APP_ID: OnceLock<Option<String>> = OnceLock::new();
static CONNECTION: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();

/// The app id to register as, from [`crate::app_id`]; `None` registers
/// nothing. Set once at startup, before the first portal request.
pub fn set_app_id(app_id: Option<String>) {
    if APP_ID.set(app_id).is_err() {
        tracing::warn!("the portal app id was already set");
    }
}

/// The session bus connection every portal request goes through,
/// registered with the portal's host app registry when it opened. A failed
/// connection is tried again by the next call.
pub async fn connection() -> zbus::Result<zbus::Connection> {
    CONNECTION
        .get_or_try_init(|| async {
            let connection = tokio::time::timeout(CALL_TIMEOUT, zbus::Connection::session())
                .await
                .map_err(|_| zbus::Error::Failure("the session bus did not answer".into()))??;
            if let Some(app_id) = APP_ID.get().cloned().flatten() {
                register(&connection, &app_id).await;
            }
            Ok(connection)
        })
        .await
        .cloned()
}

async fn register(connection: &zbus::Connection, app_id: &str) {
    // A sandbox gives the app its id; the registry refuses sandboxed callers.
    if std::path::Path::new("/.flatpak-info").exists() {
        return;
    }
    let body = (app_id, HashMap::<&str, zvariant::Value<'_>>::new());
    let call = connection.call_method(
        Some(DESKTOP),
        DESKTOP_PATH,
        Some(REGISTRY),
        "Register",
        &body,
    );
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(_)) => tracing::info!(app_id, "registered with the desktop portal"),
        Ok(Err(error)) if missing(&error) => {
            tracing::debug!(%error, "the desktop portal has no host app registry");
        }
        Ok(Err(error)) => tracing::warn!(
            %error,
            app_id,
            "the desktop portal refused Librelog's app id; its requests go unregistered"
        ),
        Err(_) => tracing::warn!("registering with the desktop portal timed out"),
    }
}

/// Portals before the registry (1.19) answer that the method or interface
/// does not exist, and a desktop without portals has no such service.
fn missing(error: &zbus::Error) -> bool {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => name.as_str(),
        zbus::Error::FDO(error) => return missing_fdo(error),
        _ => return false,
    };
    [
        "org.freedesktop.DBus.Error.UnknownMethod",
        "org.freedesktop.DBus.Error.UnknownInterface",
        "org.freedesktop.DBus.Error.UnknownObject",
        "org.freedesktop.DBus.Error.ServiceUnknown",
    ]
    .contains(&name)
}

fn missing_fdo(error: &zbus::fdo::Error) -> bool {
    matches!(
        error,
        zbus::fdo::Error::UnknownMethod(_)
            | zbus::fdo::Error::UnknownInterface(_)
            | zbus::fdo::Error::UnknownObject(_)
            | zbus::fdo::Error::ServiceUnknown(_)
    )
}

/// Whether a StatusNotifier host shows tray icons: KDE, or GNOME with the
/// AppIndicator extension. GNOME alone has none, so a tray icon there is
/// invisible. `None` when the session bus did not answer.
pub async fn status_notifier_host() -> Option<bool> {
    let connection = connection().await.ok()?;
    let watcher = "org.kde.StatusNotifierWatcher";
    let owned = tokio::time::timeout(
        CALL_TIMEOUT,
        connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &watcher,
        ),
    )
    .await
    .ok()?
    .ok()?
    .body()
    .deserialize::<bool>()
    .ok()?;
    if !owned {
        return Some(false);
    }
    let registered = tokio::time::timeout(
        CALL_TIMEOUT,
        connection.call_method(
            Some(watcher),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(watcher, "IsStatusNotifierHostRegistered"),
        ),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|reply| reply.body().deserialize::<zvariant::OwnedValue>().ok())
    .and_then(|value| bool::try_from(value).ok());
    // A watcher that cannot say is taken at its word that it exists.
    Some(registered.unwrap_or(true))
}
