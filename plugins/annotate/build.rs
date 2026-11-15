const COMMANDS: &[&str] = &[
    "screen_recording",
    "request_screen_recording",
    "open_screen_recording_settings",
    "start",
    "stop",
    "finalize",
    "cancel",
    "set_ink_mode",
    "status",
    "set_retention",
    "annotations_dir",
    "latest",
    "surface",
    "ink_pointer",
    "ink_escape",
    "capsule_pressed",
    "surface_ready",
    "forget_screen_choice",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
