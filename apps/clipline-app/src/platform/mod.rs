pub mod types;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use types::{
    AudioDeviceLists, CapabilityStatus, CapturableWindow, DisplayInfo,
    PermissionAction, PlatformCapabilities, PlatformOs,
};

// Keep this unconditional for Windows facade implementation; macOS currently
// keeps it unused because the full audio facade wiring is still staged.
#[cfg_attr(target_os = "macos", allow(unused_imports))]
pub use types::AudioDeviceInfo;

#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;
