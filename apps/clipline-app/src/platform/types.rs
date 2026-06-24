#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformOs {
    #[allow(dead_code)] // Staged cross-platform model: Windows variant is exercised on the Windows build.
    Windows,
    Macos,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityStatus {
    pub available: bool,
    pub reason: Option<String>,
    pub action: Option<PermissionAction>,
}

impl CapabilityStatus {
    pub fn available() -> Self {
        Self {
            available: true,
            reason: None,
            action: None,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            action: None,
        }
    }

    pub fn needs_permission(reason: impl Into<String>, action: PermissionAction) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            action: Some(action),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    OpenScreenRecordingSettings,
    OpenMicrophoneSettings,
    #[allow(dead_code)] // Staged macOS shell wiring: accessibility path is intentionally not represented yet.
    OpenAccessibilitySettings,
    OpenInputMonitoringSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlatformCapabilities {
    pub os: PlatformOs,
    pub display_capture: CapabilityStatus,
    pub window_capture: CapabilityStatus,
    pub display_region_capture: CapabilityStatus,
    pub system_audio: CapabilityStatus,
    pub microphone: CapabilityStatus,
    pub per_process_audio: CapabilityStatus,
    pub global_hotkey: CapabilityStatus,
    pub in_game_hotkey_fallback: CapabilityStatus,
    pub startup_login_item: CapabilityStatus,
    pub hardware_encode: CapabilityStatus,
    pub hdr_capture: CapabilityStatus,
    pub player_decode: CapabilityStatus,
    pub file_clipboard: CapabilityStatus,
    pub updater: CapabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioDeviceLists {
    pub outputs: Vec<AudioDeviceInfo>,
    pub inputs: Vec<AudioDeviceInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturableWindow {
    pub handle: isize,
    pub title: String,
    pub process_id: u32,
    pub exe_name: String,
    pub exe_path: Option<String>,
}
