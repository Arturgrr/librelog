use tauri::{Result, image::Image};

pub enum TrayIconState {
    Default,
    Degraded,
    UpdateAvailable,
}

pub const RECORDING_FRAMES: &[&[u8]] = &[
    include_bytes!("../icons/tray_recording_0.png"),
    include_bytes!("../icons/tray_recording_1.png"),
    include_bytes!("../icons/tray_recording_2.png"),
];

impl TrayIconState {
    pub fn to_image(&self) -> Result<Image<'static>> {
        let bytes: &[u8] = match self {
            TrayIconState::Default => include_bytes!("../icons/tray_default.png"),
            TrayIconState::Degraded => include_bytes!("../icons/tray_degraded.png"),
            TrayIconState::UpdateAvailable => include_bytes!("../icons/tray_update.png"),
        };
        image(bytes)
    }
}

/// An icon in the colour that shows on this desktop's panel.
pub fn image(bytes: &[u8]) -> Result<Image<'static>> {
    let image = Image::from_bytes(bytes)?;
    #[cfg(target_os = "linux")]
    {
        let glyph = crate::linux_theme::glyph();
        if glyph != crate::linux_theme::Glyph::Light {
            let (width, height) = (image.width(), image.height());
            let mut rgba = image.rgba().to_vec();
            crate::linux_theme::paint(&mut rgba, width as usize, height as usize, glyph);
            return Ok(Image::new_owned(rgba, width, height));
        }
    }
    Ok(image)
}
