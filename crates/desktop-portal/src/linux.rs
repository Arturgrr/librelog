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
        let icon = appimage
            .as_ref()
            .and_then(|_| copy_appimage_icon(identifier, &data_home));
        let launcher = crate::entry::Launcher {
            identifier,
            name,
            executable: &executable,
            visible: appimage.is_some(),
            icon: icon.as_deref(),
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
            let connection = session().await?;
            let Some(app_id) = APP_ID.get().cloned().flatten() else {
                return Ok(connection);
            };
            if register(&connection, &app_id).await == Registered::NoRegistry
                && own_scope(&connection, &app_id).await
            {
                // The portal reads a connection's app id once, when it first
                // hears from it: a new connection sees the new unit.
                return session().await;
            }
            Ok(connection)
        })
        .await
        .cloned()
}

async fn session() -> zbus::Result<zbus::Connection> {
    tokio::time::timeout(CALL_TIMEOUT, zbus::Connection::session())
        .await
        .map_err(|_| zbus::Error::Failure("the session bus did not answer".into()))?
}

#[derive(PartialEq, Eq)]
enum Registered {
    Yes,
    NoRegistry,
    No,
}

async fn register(connection: &zbus::Connection, app_id: &str) -> Registered {
    // A sandbox gives the app its id; the registry refuses sandboxed callers.
    if sandboxed() {
        return Registered::No;
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
        Ok(Ok(_)) => {
            tracing::info!(app_id, "registered with the desktop portal");
            Registered::Yes
        }
        Ok(Err(error)) if missing(&error) => {
            tracing::debug!(%error, "the desktop portal has no host app registry");
            Registered::NoRegistry
        }
        Ok(Err(error)) => {
            tracing::warn!(
                %error,
                app_id,
                "the desktop portal refused Librelog's app id; its requests go unregistered"
            );
            Registered::No
        }
        Err(_) => {
            tracing::warn!("registering with the desktop portal timed out");
            Registered::No
        }
    }
}

fn sandboxed() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Moves Librelog into a systemd scope named after `app_id` unless its unit
/// already names it (see [`crate::scope`]); returns whether it moved.
async fn own_scope(connection: &zbus::Connection, app_id: &str) -> bool {
    let unit_app_id =
        || crate::scope::app_id(&std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default());
    let current = unit_app_id();
    if sandboxed() || current.as_deref() == Some(app_id) {
        return false;
    }
    let pid = std::process::id();
    let unit = crate::scope::unit_name(app_id, pid);
    let properties: Vec<(&str, zvariant::Value<'_>)> = vec![
        ("PIDs", vec![pid].into()),
        ("Description", "Librelog".into()),
        ("CollectMode", "inactive-or-failed".into()),
    ];
    let aux: Vec<(&str, Vec<(&str, zvariant::Value<'_>)>)> = Vec::new();
    let body = (unit.as_str(), "fail", properties, aux);
    let call = connection.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &body,
    );
    match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, unit, "could not give Librelog a systemd scope of its own");
            return false;
        }
        Err(_) => {
            tracing::warn!(unit, "systemd did not answer for Librelog's scope");
            return false;
        }
    }
    // The job moves the process; wait until it has.
    for _ in 0..20 {
        if unit_app_id().as_deref() == Some(app_id) {
            tracing::info!(
                unit,
                from = current.as_deref().unwrap_or(""),
                "moved Librelog into its own scope for the desktop portal's app id"
            );
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tracing::warn!(
        unit,
        "Librelog's scope started but the process did not move"
    );
    false
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

/// The app id Librelog registers with, if any.
pub fn app_id() -> Option<String> {
    APP_ID.get().cloned().flatten()
}

const KGLOBALACCEL: &str = "org.kde.kglobalaccel";

/// Calls kglobalaccel, KDE's global shortcut daemon, only when it runs, so it
/// is never started outside KDE.
async fn kglobalaccel<B>(
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> zbus::Result<zbus::Message>
where
    B: zbus::export::serde::Serialize + zvariant::DynamicType,
{
    let unanswered = || zbus::Error::Failure("the session bus did not answer".into());
    let connection = plain_connection().await.ok_or_else(unanswered)?;
    let bus = zbus::fdo::DBusProxy::new(&connection).await?;
    let name = zbus::names::BusName::try_from(KGLOBALACCEL)?;
    let running = tokio::time::timeout(CALL_TIMEOUT, bus.name_has_owner(name))
        .await
        .map_err(|_| unanswered())??;
    if !running {
        return Err(zbus::Error::Failure("kglobalaccel is not running".into()));
    }
    let call = connection.call_method(Some(KGLOBALACCEL), path, Some(interface), method, body);
    tokio::time::timeout(CALL_TIMEOUT, call)
        .await
        .map_err(|_| zbus::Error::Failure("kglobalaccel did not answer".into()))?
}

/// The shortcut ids kglobalaccel keeps for `component`, active or not. KDE's
/// portal names an app's component after its app id.
pub async fn kde_shortcut_ids(component: &str) -> zbus::Result<Vec<String>> {
    let reply = kglobalaccel(
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
        "getComponent",
        &(component,),
    )
    .await?;
    let path: zvariant::OwnedObjectPath = reply.body().deserialize()?;
    let reply = kglobalaccel(
        path.as_str(),
        "org.kde.kglobalaccel.Component",
        "shortcutNames",
        &("default",),
    )
    .await?;
    reply.body().deserialize()
}

/// Removes shortcut `id` from kglobalaccel's `component`, which takes it out
/// of System Settings → Shortcuts.
pub async fn forget_kde_shortcut(component: &str, id: &str) -> zbus::Result<()> {
    let reply = kglobalaccel(
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
        "unregister",
        &(component, id),
    )
    .await?;
    if reply.body().deserialize::<bool>()? {
        Ok(())
    } else {
        Err(zbus::Error::Failure(
            "kglobalaccel kept the shortcut".into(),
        ))
    }
}
