#[allow(
    dead_code,
    reason = "used by fallback server tasks after the manifest contract is introduced"
)]
pub const FALLBACK_COMMANDS: &[&str] = &[
    "cache_cloud_clip_media",
    "check_for_updates",
    "choose_media_folder",
    "choose_replay_cache_folder",
    "clip_poster",
    "cloud_clip_thumbnail",
    "cloud_connect",
    "cloud_disconnect",
    "cloud_user_avatar",
    "cloud_user_profile",
    "copy_clip_to_clipboard",
    "delete_clip",
    "export_clip",
    "extract_window_icon",
    "frontend_ready",
    "get_autostart_status",
    "get_settings",
    "install_update",
    "list_audio_devices",
    "list_clips",
    "list_cloud_clips",
    "list_displays",
    "list_game_plugins",
    "list_game_windows",
    "memory_status",
    "minimize_main_window",
    "open_cloud_clip_url",
    "open_cloud_user_profile",
    "preview_clip_audio_tracks",
    "probe_encoders",
    "rename_clip",
    "report_decode_support",
    "reveal_clip",
    "save_replay",
    "save_settings",
    "set_recording",
    "start_microphone_test",
    "stop_microphone_test",
    "storage_status",
    "sync_cloud_clip_status",
    "upload_clip_to_cloud",
];

#[allow(
    dead_code,
    reason = "used by fallback server tasks after the manifest contract is introduced"
)]
pub const FALLBACK_EVENTS: &[&str] = &[
    "cloud-upload-progress",
    "error",
    "game-detection",
    "mic-test",
    "mic-test-error",
    "mic-test-stopped",
    "saved",
    "status",
];

#[allow(
    dead_code,
    reason = "used by fallback server tasks after the manifest contract is introduced"
)]
pub fn is_fallback_command(command: &str) -> bool {
    FALLBACK_COMMANDS.contains(&command)
}

#[allow(
    dead_code,
    reason = "used by fallback server tasks after the manifest contract is introduced"
)]
pub fn is_fallback_event(event: &str) -> bool {
    FALLBACK_EVENTS.contains(&event)
}
