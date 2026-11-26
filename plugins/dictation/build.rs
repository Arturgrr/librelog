const COMMANDS: &[&str] = &[
    "hud",
    "start_recording",
    "start_system_recording",
    "stop_recording",
    "cancel_recording",
    "discard_recording",
    "check_insertion",
    "insert_text",
    "insert_dictation",
    "read_dictation_field",
    "frontmost_app",
    "keep_history_audio",
    "history_audio_path",
    "delete_history_audio",
    "prune_history_audio",
    "request_media_control",
    "apply_dictation_replacements",
    "fit_dictation_text",
    "join_dictation_transcript",
    "prepare_dictation_refinement",
    "resolve_dictation_refinement",
    "decide_dictation_context",
    "may_read_dictation_for_fitting",
    "extract_dictation_terms",
    "rank_dictation_terms",
    "record_dictation_terms",
    "suggest_dictation_terms",
    "kept_dictation_text",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
