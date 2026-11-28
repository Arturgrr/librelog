/// The application that has focus: its display name, and its bundle
/// identifier (macOS), executable (Windows, Linux) when known.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TargetApp {
    pub name: String,
    pub id: Option<String>,
}

/// Librelog's own id as [`frontmost`] reports it: its bundle identifier on macOS, its
/// executable elsewhere.
pub fn own_id() -> Option<&'static str> {
    static OWN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    OWN.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            objc2::rc::autoreleasepool(|_| {
                objc2_app_kit::NSRunningApplication::currentApplication()
                    .bundleIdentifier()
                    .map(|id| id.to_string())
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let path = std::env::current_exe().ok()?;
            Some(path.file_name()?.to_string_lossy().into_owned())
        }
    })
    .as_deref()
}

#[cfg(target_os = "macos")]
pub async fn frontmost() -> Option<TargetApp> {
    tauri::async_runtime::spawn_blocking(|| {
        objc2::rc::autoreleasepool(|_| {
            let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
            let id = app.bundleIdentifier().map(|id| id.to_string());
            let name = app
                .localizedName()
                .map(|name| name.to_string())
                .or_else(|| id.clone())?;
            Some(TargetApp { name, id })
        })
    })
    .await
    .ok()
    .flatten()
}

#[cfg(target_os = "windows")]
pub async fn frontmost() -> Option<TargetApp> {
    tauri::async_runtime::spawn_blocking(windows::frontmost)
        .await
        .ok()
        .flatten()
}

#[cfg(target_os = "linux")]
pub async fn frontmost() -> Option<TargetApp> {
    let found = tokio::time::timeout(std::time::Duration::from_secs(2), linux::frontmost())
        .await
        .ok()
        .flatten();
    match found {
        Some(app) => Some(app),
        // Plasma keeps Qt apps off AT-SPI unless accessibility is turned on.
        None => linux::kwin_frontmost().await,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub async fn frontmost() -> Option<TargetApp> {
    None
}

#[cfg(target_os = "windows")]
mod windows {
    use windows::Win32::{
        Foundation::CloseHandle,
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };
    use windows::core::{HSTRING, PWSTR};

    use super::TargetApp;

    pub fn frontmost() -> Option<TargetApp> {
        let path = foreground_executable()?;
        let executable = std::path::Path::new(&path);
        let id = executable.file_name()?.to_string_lossy().into_owned();
        let name = file_description(&path)
            .or_else(|| Some(executable.file_stem()?.to_string_lossy().into_owned()))?;
        Some(TargetApp { name, id: Some(id) })
    }

    fn foreground_executable() -> Option<String> {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return None;
            }
            let mut process_id = 0_u32;
            GetWindowThreadProcessId(window, Some(&mut process_id));
            if process_id == 0 {
                return None;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
            let mut buffer = [0_u16; 1024];
            let mut length = buffer.len() as u32;
            let queried = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(process);
            queried.ok()?;
            Some(String::from_utf16_lossy(&buffer[..length as usize]))
        }
    }

    /// The executable's "FileDescription" version resource, the name Windows
    /// shows for an app in Task Manager.
    fn file_description(path: &str) -> Option<String> {
        let path = HSTRING::from(path);
        unsafe {
            let size = GetFileVersionInfoSizeW(&path, None);
            if size == 0 {
                return None;
            }
            let mut data = vec![0_u8; size as usize];
            GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()).ok()?;
            let mut translation = std::ptr::null_mut();
            let mut length = 0_u32;
            if !VerQueryValueW(
                data.as_ptr().cast(),
                &HSTRING::from("\\VarFileInfo\\Translation"),
                &mut translation,
                &mut length,
            )
            .as_bool()
                || length < 4
            {
                return None;
            }
            let [language, code_page] = *translation.cast::<[u16; 2]>();
            let mut value = std::ptr::null_mut();
            let mut length = 0_u32;
            if !VerQueryValueW(
                data.as_ptr().cast(),
                &HSTRING::from(format!(
                    "\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"
                )),
                &mut value,
                &mut length,
            )
            .as_bool()
                || length == 0
            {
                return None;
            }
            let text = std::slice::from_raw_parts(value.cast::<u16>(), length as usize);
            let text = String::from_utf16_lossy(text);
            let text = text.trim_end_matches('\0').trim();
            (!text.is_empty()).then(|| text.to_string())
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
    use atspi::{AccessibilityConnection, State};

    use super::TargetApp;

    /// The AT-SPI application owning the active window, named as it names
    /// itself, with its executable when the bus knows the process.
    pub async fn frontmost() -> Option<TargetApp> {
        let connection = AccessibilityConnection::new().await.ok()?;
        let bus = connection.connection();
        let root = AccessibleProxy::builder(bus)
            .destination("org.a11y.atspi.Registry")
            .ok()?
            .path("/org/a11y/atspi/accessible/root")
            .ok()?
            .build()
            .await
            .ok()?;
        for application in root.get_children().await.ok()? {
            let Ok(proxy) = application.clone().into_accessible_proxy(bus).await else {
                continue;
            };
            let Ok(windows) = proxy.get_children().await else {
                continue;
            };
            for window in windows {
                let Ok(window) = window.into_accessible_proxy(bus).await else {
                    continue;
                };
                if !window
                    .get_state()
                    .await
                    .is_ok_and(|states| states.contains(State::Active))
                {
                    continue;
                }
                let name = proxy.name().await.ok().filter(|name| !name.is_empty())?;
                let owner = application.name.to_string();
                let process = bus
                    .call_method(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        Some("org.freedesktop.DBus"),
                        "GetConnectionUnixProcessID",
                        &owner.as_str(),
                    )
                    .await
                    .ok()
                    .and_then(|reply| reply.body().deserialize::<u32>().ok());
                return Some(TargetApp {
                    name,
                    id: process.and_then(executable),
                });
            }
        }
        None
    }

    fn executable(process: u32) -> Option<String> {
        let path = std::fs::read_link(format!("/proc/{process}/exe")).ok()?;
        Some(path.file_name()?.to_string_lossy().into_owned())
    }

    /// KWin's active window, named as its desktop entry names it.
    pub async fn kwin_frontmost() -> Option<TargetApp> {
        let window = crate::kwin::active_window().await?;
        let name = window
            .desktop_file
            .as_deref()
            .and_then(entry_name)
            .or_else(|| Some(window.name.clone()).filter(|name| !name.is_empty()))
            .or_else(|| Some(window.class.clone()).filter(|class| !class.is_empty()))?;
        Some(TargetApp {
            name,
            id: window.pid.and_then(executable),
        })
    }

    /// `Name=` from `<id>.desktop` in the XDG data folders.
    fn entry_name(id: &str) -> Option<String> {
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join(".local/share"))
            });
        let dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let text = home
            .into_iter()
            .chain(dirs.split(':').map(std::path::PathBuf::from))
            .find_map(|dir| {
                std::fs::read_to_string(dir.join("applications").join(format!("{id}.desktop"))).ok()
            })?;
        let mut in_entry = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
            } else if in_entry && let Some(name) = line.strip_prefix("Name=") {
                return Some(name.trim().to_owned()).filter(|name| !name.is_empty());
            }
        }
        None
    }
}
