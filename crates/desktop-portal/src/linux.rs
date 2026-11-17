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

/// The app id to register as; `None` registers nothing. Set once at
/// startup, before the first portal request. See [`prepare_app_id`].
pub fn set_app_id(app_id: Option<String>) {
    if APP_ID.set(app_id).is_err() {
        tracing::warn!("the portal app id was already set");
    }
}

/// Makes sure `<identifier>.desktop` exists, writing Librelog's own entry
/// for an AppImage or a development build, then registers `identifier` as
/// the app id. Runs before any portal request; returns what to log once
/// logging runs.
pub fn prepare_app_id(identifier: &str, name: &str) -> Option<String> {
    if !crate::reverse_dns(identifier) {
        set_app_id(None);
        return Some(format!(
            "{identifier} is not a reverse-DNS app id; the portals will not know Librelog"
        ));
    }
    let message = (|| {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| home.map(|home| home.join(".local/share")))?;
        let dirs =
            crate::entry::data_dirs(&data_home, std::env::var_os("XDG_DATA_DIRS").as_deref());
        let appimage = std::env::var_os("APPIMAGE")
            .filter(|path| !path.is_empty())
            .map(std::path::PathBuf::from);
        let executable = match &appimage {
            Some(appimage) => appimage.clone(),
            None => std::env::current_exe().ok()?,
        };
        let window_class = std::env::current_exe()
            .ok()?
            .file_name()?
            .to_string_lossy()
            .into_owned();
        let icon = appimage
            .as_ref()
            .and_then(|_| copy_appimage_icon(identifier, &data_home));
        let launcher = crate::entry::Launcher {
            identifier,
            name,
            executable: &executable,
            visible: appimage.is_some(),
            icon: icon.as_deref(),
            window_class: &window_class,
        };
        Some(match crate::entry::ensure(&launcher, &data_home, &dirs) {
            Ok(crate::entry::Entry::Written(path)) => {
                format!("wrote the desktop entry {}", path.display())
            }
            Ok(_) => return None,
            Err(error) => format!("could not write Librelog's desktop entry: {error}"),
        })
    })();
    set_app_id(Some(identifier.to_owned()));
    message
}

/// The AppImage's icon, copied out of its mount (a new path each run).
fn copy_appimage_icon(identifier: &str, data_home: &std::path::Path) -> Option<std::path::PathBuf> {
    let source = std::path::PathBuf::from(std::env::var_os("APPDIR")?).join(".DirIcon");
    let bytes = std::fs::read(source).ok()?;
    let icon = data_home.join("icons").join(format!("{identifier}.png"));
    if std::fs::read(&icon).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::create_dir_all(icon.parent()?).ok()?;
        std::fs::write(&icon, bytes).ok()?;
    }
    Some(icon)
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

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
static PLAIN: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();

/// A session bus connection of its own: the tray check must not wait for,
/// or depend on, the portal registration.
async fn plain_connection() -> Option<zbus::Connection> {
    PLAIN
        .get_or_try_init(|| async {
            tokio::time::timeout(CALL_TIMEOUT, zbus::Connection::session())
                .await
                .map_err(|_| zbus::Error::Failure("the session bus did not answer".into()))?
        })
        .await
        .ok()
        .cloned()
}

/// Whether a StatusNotifier host shows tray icons: KDE, or GNOME with the
/// AppIndicator extension. GNOME alone has none, so a tray icon there is
/// invisible. `None` when the session bus did not answer.
pub async fn status_notifier_host() -> Option<bool> {
    let connection = plain_connection().await?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
    let owned = tokio::time::timeout(CALL_TIMEOUT, bus.name_has_owner(WATCHER.try_into().ok()?))
        .await
        .ok()?
        .ok()?;
    if !owned {
        return Some(false);
    }
    let registered = tokio::time::timeout(
        CALL_TIMEOUT,
        connection.call_method(
            Some(WATCHER),
            "/StatusNotifierWatcher",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(WATCHER, "IsStatusNotifierHostRegistered"),
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

/// Calls `changed` with the host's answer each time the watcher appears or
/// goes (the AppIndicator extension enabled or disabled), until the bus
/// connection ends.
pub async fn watch_status_notifier_host(changed: impl Fn(Option<bool>) + Send + 'static) {
    let Some(connection) = plain_connection().await else {
        return;
    };
    let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else {
        return;
    };
    use futures_util::StreamExt;
    let Ok(mut owners) = bus
        .receive_name_owner_changed_with_args(&[(0, WATCHER)])
        .await
    else {
        return;
    };
    while owners.next().await.is_some() {
        // The new watcher registers its host a moment after taking the name.
        tokio::time::sleep(Duration::from_millis(500)).await;
        changed(status_notifier_host().await);
    }
}
