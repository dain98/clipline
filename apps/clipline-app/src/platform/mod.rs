pub mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use types::{
    AudioDeviceLists, CapabilityStatus, CapturableWindow, DisplayInfo,
    PermissionAction, PlatformCapabilities, PlatformOs,
};

#[cfg(target_os = "macos")]
#[allow(unused_imports)] // Staged facade API: AudioDeviceInfo is part of the public contract for later macOS shell wiring.
pub use types::AudioDeviceInfo;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;
