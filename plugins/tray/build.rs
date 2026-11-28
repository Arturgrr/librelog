const COMMANDS: &[&str] = &[
    "set_tray_icon_visible",
    "set_tray_recording_title",
    "set_tray_labels",
    "set_tray_paste_last_visible",
    "set_tray_capture_visible",
    "set_tray_dictation_presets",
    "set_tray_recent_dictations",
    "take_pending_tray_start",
    "tray_icon_shown_by_desktop",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
