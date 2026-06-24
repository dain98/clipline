use crate::memory::MemoryStatus;

use super::{
    AudioDeviceInfo, AudioDeviceLists, CapabilityStatus, CapturableWindow, DisplayInfo,
    PlatformCapabilities, PlatformOs,
};

pub fn capabilities() -> PlatformCapabilities {
    PlatformCapabilities {
        os: PlatformOs::Windows,
        display_capture: CapabilityStatus::available(),
        window_capture: CapabilityStatus::available(),
        display_region_capture: CapabilityStatus::available(),
        system_audio: CapabilityStatus::available(),
        microphone: CapabilityStatus::available(),
        per_process_audio: CapabilityStatus::available(),
        global_hotkey: CapabilityStatus::available(),
        in_game_hotkey_fallback: CapabilityStatus::available(),
        startup_login_item: CapabilityStatus::available(),
        hardware_encode: CapabilityStatus::available(),
        hdr_capture: CapabilityStatus::unavailable("HDR capture is not implemented yet"),
        player_decode: CapabilityStatus::available(),
        file_clipboard: CapabilityStatus::available(),
        updater: CapabilityStatus::available(),
    }
}

pub fn enumerate_capturable_windows() -> Vec<CapturableWindow> {
    clipline_capture::windows::enumerate_capturable_windows()
        .into_iter()
        .map(|window| CapturableWindow {
            handle: window.handle,
            title: window.title,
            process_id: window.process_id,
            exe_name: window.exe_name,
            exe_path: window.exe_path,
        })
        .collect()
}

pub fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    clipline_capture::windows::display::enumerate_displays()
        .map_err(|e| e.to_string())
        .map(|displays| {
            displays
                .into_iter()
                .map(|display| DisplayInfo {
                    id: display.id,
                    name: display.name,
                    x: display.x,
                    y: display.y,
                    width: display.width,
                    height: display.height,
                    is_primary: display.is_primary,
                })
                .collect()
        })
}

pub fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    clipline_capture::windows::wasapi::enumerate_audio_devices()
        .map_err(|e| e.to_string())
        .map(|devices| AudioDeviceLists {
            outputs: devices
                .outputs
                .into_iter()
                .map(|device| AudioDeviceInfo {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
            inputs: devices
                .inputs
                .into_iter()
                .map(|device| AudioDeviceInfo {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
        })
}

pub fn memory_status() -> Result<MemoryStatus, String> {
    crate::memory::current_process_tree_memory()
}
