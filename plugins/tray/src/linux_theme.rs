//! Which colour the tray glyph takes on Linux. The icons are white (macOS
//! template images), which suits GNOME's always-dark top bar but vanishes
//! on Plasma 6's default light Breeze panel. On KDE the panel follows the
//! colour scheme unless the Plasma style fixes it, so the glyph follows the
//! Settings portal's `org.freedesktop.appearance color-scheme` and the
//! Plasma style, and turns white-on-dark-outline when neither is known.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use ashpd::desktop::settings::{ColorScheme, Settings};
use futures_util::StreamExt;
use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// White, for dark panels.
    Light,
    /// Breeze's text colour, for light panels.
    Dark,
    /// White with a dark outline, readable on either.
    Outlined,
}

static GLYPH: AtomicU8 = AtomicU8::new(Glyph::Light as u8);

pub fn glyph() -> Glyph {
    match GLYPH.load(Ordering::SeqCst) {
        1 => Glyph::Dark,
        2 => Glyph::Outlined,
        _ => Glyph::Light,
    }
}

fn store(glyph: Glyph) -> bool {
    GLYPH.swap(glyph as u8, Ordering::SeqCst) != glyph as u8
}

/// The glyph for a panel: `plasma_style` is plasmarc's `[Theme] name`,
/// `scheme` the portal's answer, `kde_scheme` kdeglobals' `ColorScheme`.
pub fn glyph_for(
    kde: bool,
    plasma_style: Option<&str>,
    scheme: Option<ColorScheme>,
    kde_scheme: Option<&str>,
) -> Glyph {
    if !kde {
        return Glyph::Light;
    }
    match plasma_style {
        None | Some("default") => {}
        Some("breeze-dark") => return Glyph::Light,
        Some("breeze-light") => return Glyph::Dark,
        // A third-party style may draw its own panel whatever the colours.
        Some(_) => return Glyph::Outlined,
    }
    match (scheme, kde_scheme) {
        (Some(ColorScheme::PreferDark), _) => Glyph::Light,
        (Some(ColorScheme::PreferLight), _) => Glyph::Dark,
        (_, Some(name)) if name.to_ascii_lowercase().contains("dark") => Glyph::Light,
        (_, Some(_)) => Glyph::Dark,
        _ => Glyph::Outlined,
    }
}

fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| {
            desktop
                .split(':')
                .any(|name| name.eq_ignore_ascii_case("KDE"))
        })
        .unwrap_or(false)
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            inside = name == section;
        } else if inside
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn kde_setting(file: &str, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_home()?.join(file)).ok()?;
    ini_value(&text, section, key)
}

fn current(scheme: Option<ColorScheme>) -> Glyph {
    glyph_for(
        true,
        kde_setting("plasmarc", "Theme", "name").as_deref(),
        scheme,
        kde_setting("kdeglobals", "General", "ColorScheme").as_deref(),
    )
}

/// On KDE, picks the glyph now from Plasma's config, then follows the
/// portal's colour scheme, which a global theme switch also changes.
/// Elsewhere the glyph stays white.
pub fn watch(app: &AppHandle<tauri::Wry>) {
    if !is_kde() {
        return;
    }
    store(current(None));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = tokio::time::timeout(Duration::from_millis(1_500), async {
            let connection = anlg_desktop_portal::connection().await.ok()?;
            Settings::with_connection(connection).await.ok()
        })
        .await
        .ok()
        .flatten();
        let Some(settings) = settings else {
            return;
        };
        let scheme = tokio::time::timeout(Duration::from_millis(1_500), settings.color_scheme())
            .await
            .ok()
            .and_then(Result::ok);
        apply(&app, current(scheme));
        let Ok(mut changes) = settings.receive_color_scheme_changed().await else {
            return;
        };
        while let Some(scheme) = changes.next().await {
            apply(&app, current(Some(scheme)));
        }
    });
}

fn apply(app: &AppHandle<tauri::Wry>, glyph: Glyph) {
    if store(glyph) {
        tracing::info!(?glyph, "tray_glyph_follows_the_panel");
        let _ = crate::ext::refresh_tray_icon(app);
    }
}

const DARK: [f32; 3] = [0x23 as f32, 0x26 as f32, 0x29 as f32];

/// Recolours a white RGBA glyph in place.
pub fn paint(rgba: &mut [u8], width: usize, height: usize, glyph: Glyph) {
    match glyph {
        Glyph::Light => {}
        Glyph::Dark => {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel[..3].copy_from_slice(&DARK.map(|channel| channel as u8));
            }
        }
        Glyph::Outlined => {
            let alpha: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
            let outline = dilate(&alpha, width, height, (width / 20).max(1));
            for ((pixel, &glyph), &around) in rgba.chunks_exact_mut(4).zip(&alpha).zip(&outline) {
                let glyph = f32::from(glyph) / 255.0;
                let around = f32::from(around) / 255.0;
                let alpha = glyph + around * (1.0 - glyph);
                if alpha <= 0.0 {
                    continue;
                }
                for (channel, dark) in pixel[..3].iter_mut().zip(DARK) {
                    let value = (255.0 * glyph + dark * around * (1.0 - glyph)) / alpha;
                    *channel = value.round() as u8;
                }
                pixel[3] = (alpha * 255.0).round() as u8;
            }
        }
    }
}

/// A square max filter, one axis at a time.
fn dilate(alpha: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut rows = vec![0u8; alpha.len()];
    for y in 0..height {
        for x in 0..width {
            let from = x.saturating_sub(radius);
            let to = (x + radius).min(width - 1);
            rows[y * width + x] = alpha[y * width + from..=y * width + to]
                .iter()
                .copied()
                .max()
                .unwrap_or(0);
        }
    }
    let mut out = vec![0u8; alpha.len()];
    for x in 0..width {
        for y in 0..height {
            let from = y.saturating_sub(radius);
            let to = (y + radius).min(height - 1);
            out[y * width + x] = (from..=to)
                .map(|row| rows[row * width + x])
                .max()
                .unwrap_or(0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The white glyph vanished on Plasma 6's light Breeze panel.
    #[test]
    fn the_glyph_contrasts_with_the_panel_it_sits_on() {
        let light = Some(ColorScheme::PreferLight);
        let dark = Some(ColorScheme::PreferDark);
        for (kde, style, scheme, kde_scheme, expected) in [
            (false, None, light, None, Glyph::Light),
            (true, None, light, None, Glyph::Dark),
            (
                true,
                Some("default"),
                dark,
                Some("BreezeLight"),
                Glyph::Light,
            ),
            (true, Some("breeze-dark"), light, None, Glyph::Light),
            (true, Some("breeze-light"), dark, None, Glyph::Dark),
            (true, Some("Sweet"), light, None, Glyph::Outlined),
            (true, None, None, Some("BreezeDark"), Glyph::Light),
            (
                true,
                None,
                Some(ColorScheme::NoPreference),
                Some("Breeze"),
                Glyph::Dark,
            ),
            (true, None, None, None, Glyph::Outlined),
        ] {
            assert_eq!(
                glyph_for(kde, style, scheme, kde_scheme),
                expected,
                "{kde} {style:?} {scheme:?} {kde_scheme:?}"
            );
        }
    }

    #[test]
    fn an_outlined_glyph_stays_white_inside_a_dark_border() {
        let (width, height) = (40, 1);
        let mut rgba = vec![0u8; width * height * 4];
        rgba[20 * 4..21 * 4].copy_from_slice(&[255, 255, 255, 255]);
        paint(&mut rgba, width, height, Glyph::Outlined);
        let pixel = |x: usize| &rgba[x * 4..x * 4 + 4];
        assert_eq!(pixel(20), [255, 255, 255, 255]);
        assert_eq!(pixel(19), [0x23, 0x26, 0x29, 255]);
        assert_eq!(pixel(18)[3], 255);
        assert_eq!(pixel(17)[3], 0);
    }
}
