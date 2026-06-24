pub mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use types::{
    AudioDeviceInfo, AudioDeviceLists, CapabilityStatus, CapturableWindow, DisplayInfo,
    PermissionAction, PlatformCapabilities, PlatformOs,
};

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;
