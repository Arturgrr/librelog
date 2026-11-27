//! The app id a portal without the host app registry (xdg-desktop-portal
//! before 1.19: Kubuntu 24.04 and 24.10, Debian 12) gives Librelog. It reads
//! it from the process's systemd unit, which the desktop names after the
//! entry it launched (`app-<id>-<random>.scope`, `app-<id>@autostart.service`).
//! Started from a terminal, Librelog sits in the terminal's unit: no id, or
//! the terminal's own. KDE then keeps the shortcuts under a new
//! `token_<uuid>` component each session (or under Konsole's), and asks for
//! them again on every launch. So Librelog moves itself into a scope of its
//! own first, as a desktop launching it would have.

/// The app id xdg-desktop-portal reads from `/proc/<pid>/cgroup`: the last
/// unit in the path, when it is an `app-` unit, as its
/// `parse_app_id_from_unit_name` reads it.
pub fn app_id(cgroup: &str) -> Option<String> {
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let unit = path.rsplit('/').next()?;
    unit_app_id(unit)
}

fn unit_app_id(unit: &str) -> Option<String> {
    let alnum = |text: &str| text.chars().all(|c| c.is_ascii_alphanumeric());
    let body = unit.strip_prefix("app-")?;
    let (body, scope) = if let Some(body) = body.strip_suffix(".scope") {
        (body, true)
    } else if let Some(body) = body.strip_suffix(".service") {
        (body, false)
    } else {
        (body.strip_suffix(".slice")?, true)
    };
    // `app[-<launcher>]-<id>-<random>.scope`, `app[-<launcher>]-<id>[@<random>].service`.
    let id = |rest: &str| -> Option<String> {
        let id = if scope {
            rest.char_indices()
                .filter(|&(at, c)| c == '-' && at > 0 && alnum(&rest[at + 1..]))
                .map(|(at, _)| &rest[..at])
                .next()?
        } else {
            match rest.split_once('@') {
                Some((id, random)) if !id.is_empty() && alnum(random) => id,
                _ => rest,
            }
        };
        (!id.is_empty()).then(|| unescape(id))
    };
    body.split_once('-')
        .filter(|(launcher, _)| !launcher.is_empty() && alnum(launcher))
        .and_then(|(_, rest)| id(rest))
        .or_else(|| id(body))
}

fn unescape(name: &str) -> String {
    let mut out = Vec::with_capacity(name.len());
    let bytes = name.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\'
            && bytes.get(at + 1) == Some(&b'x')
            && let Some(byte) = name
                .get(at + 2..at + 4)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 4;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `app-<id>-<pid>.scope`, the id escaped as systemd unit names need.
pub fn unit_name(app_id: &str, pid: u32) -> String {
    let mut name = String::from("app-");
    for byte in app_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':') {
            name.push(byte as char);
        } else {
            name.push_str(&format!("\\x{byte:02x}"));
        }
    }
    name.push_str(&format!("-{pid}.scope"));
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Started from Konsole, xdg-desktop-portal 1.18 took Konsole's id for
    /// Librelog's shortcuts; from GNOME Terminal, none.
    #[test]
    fn reads_the_app_id_as_the_portal_does_and_names_a_scope_it_reads_back() {
        let cgroup = |unit: &str| {
            format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}\n")
        };
        for (unit, id) in [
            (
                "app-com.arturgrr.librelog@autostart.service",
                Some("com.arturgrr.librelog"),
            ),
            (
                "app-com.arturgrr.librelog-4a2b.scope",
                Some("com.arturgrr.librelog"),
            ),
            (
                "app-gnome-com.arturgrr.librelog-1234.scope",
                Some("com.arturgrr.librelog"),
            ),
            ("app-org.kde.konsole-1234.scope", Some("org.kde.konsole")),
            (
                "app-gnome-org.gnome.Evolution\\x2dalarm\\x2dnotify-2437.scope",
                Some("org.gnome.Evolution-alarm-notify"),
            ),
            ("vte-spawn-0f9c.scope", None),
            ("session-2.scope", None),
        ] {
            assert_eq!(app_id(&cgroup(unit)).as_deref(), id, "{unit}");
        }
        assert_eq!(app_id("1:name=systemd:/\n"), None);

        let unit = unit_name("com.arturgrr.librelog.dev", 77);
        assert_eq!(unit, "app-com.arturgrr.librelog.dev-77.scope");
        assert_eq!(
            app_id(&cgroup(&unit)).as_deref(),
            Some("com.arturgrr.librelog.dev")
        );
        let unit = unit_name("org.example.my-app", 77);
        assert_eq!(
            app_id(&cgroup(&unit)).as_deref(),
            Some("org.example.my-app")
        );
    }
}
