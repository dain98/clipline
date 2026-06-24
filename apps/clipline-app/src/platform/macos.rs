use crate::memory::MemoryStatus;

use super::{
    AudioDeviceLists, CapabilityStatus, CapturableWindow, DisplayInfo, PermissionAction,
    PlatformCapabilities, PlatformOs,
};

pub fn capabilities() -> PlatformCapabilities {
    PlatformCapabilities {
        os: PlatformOs::Macos,
        display_capture: CapabilityStatus::needs_permission(
            "ScreenCaptureKit display capture is not implemented in Milestone 1",
            PermissionAction::OpenScreenRecordingSettings,
        ),
        window_capture: CapabilityStatus::needs_permission(
            "ScreenCaptureKit window capture is not implemented in Milestone 1",
            PermissionAction::OpenScreenRecordingSettings,
        ),
        display_region_capture: CapabilityStatus::needs_permission(
            "ScreenCaptureKit region capture is not implemented in Milestone 1",
            PermissionAction::OpenScreenRecordingSettings,
        ),
        system_audio: CapabilityStatus::unavailable(
            "macOS system audio capture is not implemented in Milestone 1",
        ),
        microphone: CapabilityStatus::needs_permission(
            "macOS microphone capture is not implemented in Milestone 1",
            PermissionAction::OpenMicrophoneSettings,
        ),
        per_process_audio: CapabilityStatus::unavailable(
            "macOS per-process output audio is not available in v1",
        ),
        global_hotkey: CapabilityStatus::available(),
        in_game_hotkey_fallback: CapabilityStatus::needs_permission(
            "macOS focused-game hotkey fallback is not implemented in Milestone 1",
            PermissionAction::OpenInputMonitoringSettings,
        ),
        startup_login_item: CapabilityStatus::available(),
        hardware_encode: CapabilityStatus::unavailable(
            "macOS encoder probing is not implemented in Milestone 1",
        ),
        hdr_capture: CapabilityStatus::unavailable("HDR capture is not implemented yet"),
        player_decode: CapabilityStatus::available(),
        file_clipboard: CapabilityStatus::unavailable(
            "Finder clipboard copy is not implemented in Milestone 1",
        ),
        updater: CapabilityStatus::available(),
    }
}

pub fn enumerate_capturable_windows() -> Vec<CapturableWindow> {
    Vec::new()
}

pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    Ok(Vec::new())
}

pub fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    Ok(AudioDeviceLists {
        outputs: Vec::new(),
        inputs: Vec::new(),
    })
}

pub fn memory_status() -> Result<MemoryStatus, String> {
    Err("macOS memory status is not implemented in Milestone 1".into())
}
