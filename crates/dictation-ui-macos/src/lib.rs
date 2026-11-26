use std::time::Duration;

use swift_rs::{Bool, SRString, swift};

swift!(fn _dictation_hud_prewarm());
swift!(fn _dictation_hud_event(json: &SRString) -> Bool);
swift!(fn _dictation_hud_meter(rms: f64));
swift!(fn _capture_dictation_target() -> SRString);
swift!(fn _insert_dictation_text(target: &SRString, text: &SRString) -> SRString);
swift!(fn _wait_for_dictation_paste_read(timeout: f64) -> Bool);
swift!(fn _dictation_field_facts() -> SRString);
swift!(fn _dictation_field_read(window: isize, deep: Bool) -> SRString);
swift!(fn _dictation_window_text(max: isize) -> SRString);

pub fn capture_target() -> Result<String, String> {
    let value = unsafe { _capture_dictation_target() }.to_string();
    value.strip_prefix("ok:").map(str::to_owned).ok_or(value)
}

pub fn insert_text(target: &str, text: &str) -> Result<(), String> {
    let value = unsafe { _insert_dictation_text(&SRString::from(target), &SRString::from(text)) }
        .to_string();
    if value.is_empty() { Ok(()) } else { Err(value) }
}

/// Waits up to `timeout` for the app to read the text the last
/// [`insert_text`] pasted. True once it has, or when that insertion pasted
/// nothing. Call it off the main thread, where the read arrives.
pub fn wait_for_paste_read(timeout: Duration) -> bool {
    unsafe { _wait_for_dictation_paste_read(timeout.as_secs_f64()) }
}

/// The focused app, window and field as JSON (`bundleId`, `appName`, `title`, `secure`), or
/// `None` without Accessibility or a focused app.
pub fn field_facts() -> Option<String> {
    non_empty(swift_rs::autoreleasepool!({
        unsafe { _dictation_field_facts() }.to_string()
    }))
}

/// The focused text field around its selection, `window` UTF-16 units on each side, as JSON
/// (`length`, `start`, `before`, `selected`, `after`), or `None` when it can't be read. Secure
/// fields are never read. Without `deep`, apps whose text only paste reaches (web views,
/// Electron) are not asked.
pub fn field_read(window: usize, deep: bool) -> Option<String> {
    non_empty(swift_rs::autoreleasepool!({
        unsafe { _dictation_field_read(window as isize, deep) }.to_string()
    }))
}

/// The static text of the focused field's window, the newest `max_chars` characters.
pub fn window_text(max_chars: usize) -> Option<String> {
    non_empty(swift_rs::autoreleasepool!({
        unsafe { _dictation_window_text(max_chars as isize) }.to_string()
    }))
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// Builds the HUD's panel and compiles its shader ahead of the first
/// dictation.
pub fn prewarm_hud() {
    unsafe { _dictation_hud_prewarm() }
}

/// Hands the HUD one event as JSON (`{"type": "started"}`, …). Returns
/// whether the HUD could take it; a `started` it can't show returns false.
pub fn hud_event(json: &str) -> bool {
    swift_rs::autoreleasepool!({
        let json = SRString::from(json);
        unsafe { _dictation_hud_event(&json) }
    })
}

/// The microphone's RMS for one captured buffer.
pub fn hud_meter(rms: f32) {
    unsafe { _dictation_hud_meter(f64::from(rms)) }
}
