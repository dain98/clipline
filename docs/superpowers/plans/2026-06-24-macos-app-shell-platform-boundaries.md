# macOS App Shell And Platform Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the real `clipline-app` crate compile and launch the Tauri UI on macOS with honest capability stubs, while preserving the existing Windows app.

**Architecture:** Keep the current Windows implementation intact and introduce macOS-safe module paths plus a small platform facade. The first macOS app shell reuses the existing HTML/CSS/JS UI and app command surface, but capture/audio/game-window behavior reports unavailable stubs until later milestones replace them with ScreenCaptureKit/CoreAudio implementations.

**Tech Stack:** Rust 2021, Tauri 2, vanilla HTML/CSS/JS, existing Clipline crates, `tauri-plugin-global-shortcut`, `tauri-plugin-autostart`, `tauri-plugin-updater`, macOS stubs behind `#[cfg(target_os = "macos")]`, Windows implementations behind `#[cfg(windows)]`.

## Global Constraints

- The project remains one cross-platform Clipline repository.
- Windows feature behavior is not intentionally reduced.
- macOS feature parity is represented by concrete platform capabilities and implementation plans.
- The first implementation milestone explicitly compiles the real app crate on macOS with platform stubs and is narrow enough to verify without needing real capture.
- Keep the local-first privacy stance: no telemetry, no account required for recording, no game injection, no kernel driver, no memory reading.
- No GPL-only capture code or libobs.
- Linux/non-macOS non-Windows builds must keep the cheap stub path so Ubuntu CI does not need system webview libraries.
- Do not implement ScreenCaptureKit, CoreAudio capture, Keychain, VideoToolbox, or CGEventTap in this milestone; expose capability stubs instead.

---

## File Structure

- Modify `apps/clipline-app/Cargo.toml`: move app runtime dependencies from Windows-only to `cfg(any(windows, target_os = "macos"))`; keep `windows-sys` Windows-only.
- Modify `apps/clipline-app/build.rs`: run `tauri_build::build()` for Windows and macOS, keep Linux stub behavior.
- Modify `apps/clipline-app/src/main.rs`: run the real app for Windows/macOS, keep stub main for other targets, wire platform-specific module paths.
- Create `apps/clipline-app/tests/macos_shell_contract.rs`: static contract proving the app crate is no longer macOS-stub-only.
- Create `apps/clipline-app/src/platform/mod.rs`, `platform/types.rs`, `platform/windows.rs`, and `platform/macos.rs`: platform facade and capability stubs.
- Modify `apps/clipline-app/src/app.rs`: call the platform facade for displays, audio devices, memory, mic test, hotkey fallback, and game-window data.
- Modify `apps/clipline-app/src/games.rs` and `game_plugins.rs`: consume `platform::CapturableWindow` instead of `clipline_capture::windows::CapturableWindow`.
- Create `apps/clipline-app/src/service_macos.rs`: macOS recorder-service stub with the same app-facing types as `service.rs`.
- Create `apps/clipline-app/src/hotkeys_macos.rs` and `memory_macos.rs`: macOS-safe stubs.
- Modify `apps/clipline-app/src/settings/persistence.rs`: use macOS config folders and a Unix atomic replace path.
- Modify `apps/clipline-app/src/util.rs`: gate Win32 UTF-16 helpers to Windows.
- Modify `apps/clipline-app/src/library.rs`: gate Windows clipboard/Finder reveal differences and keep neutral library operations compiling on macOS.
- Modify `apps/clipline-app/src/cloud.rs`: gate Windows Credential Manager and return a clear macOS credential-store error until the Keychain milestone.
- Modify `apps/clipline-app/src/game_icon.rs`: gate Windows icon extraction and return `None` on macOS.
- Modify UI contract tests only where they assert platform copy.

---

### Task 1: Target And Entry Contract

**Files:**
- Create: `apps/clipline-app/tests/macos_shell_contract.rs`
- Modify: `apps/clipline-app/Cargo.toml`
- Modify: `apps/clipline-app/build.rs`
- Modify: `apps/clipline-app/src/main.rs`

**Interfaces:**
- Consumes: current Windows-only app crate.
- Produces: Windows/macOS app target gates and a static test that fails if macOS falls back to the old stub-only path.

- [ ] **Step 1: Write the failing shell contract**

Create `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
use std::fs;
use std::path::Path;

fn manifest() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read Cargo.toml")
}

fn main_rs() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("read src/main.rs")
}

#[test]
fn app_runtime_dependencies_are_available_on_macos() {
    let manifest = manifest();
    assert!(
        manifest.contains("[target.'cfg(any(windows, target_os = \"macos\"))'.dependencies]"),
        "Tauri and shared app dependencies must be available to both Windows and macOS"
    );
    assert!(
        manifest.contains("[target.'cfg(windows)'.dependencies]\nwindows-sys"),
        "windows-sys should remain Windows-only"
    );
}

#[test]
fn real_app_entrypoint_runs_on_windows_and_macos() {
    let main_rs = main_rs();
    assert!(
        main_rs.contains("#[cfg(any(windows, target_os = \"macos\"))]\nfn main()"),
        "Windows and macOS should run app::run()"
    );
    assert!(
        main_rs.contains("#[cfg(not(any(windows, target_os = \"macos\")))]\nfn main()"),
        "other targets should keep a stub main"
    );
    assert!(
        !main_rs.contains("clipline-app is Windows-only"),
        "macOS should no longer use the old Windows-only runtime message"
    );
}

#[test]
fn real_modules_are_declared_for_macos() {
    let main_rs = main_rs();
    for required in [
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod app;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod settings;",
        "#[cfg(any(windows, target_os = \"macos\"))]\nmod platform;",
        "#[cfg(target_os = \"macos\")]\n#[path = \"service_macos.rs\"]\nmod service;",
    ] {
        assert!(
            main_rs.contains(required),
            "missing macOS module declaration: {required}"
        );
    }
}
```

- [ ] **Step 2: Run the contract and verify it fails**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: FAIL because `Cargo.toml` only has `[target.'cfg(windows)'.dependencies]`, `main.rs` only runs `app::run()` on Windows, and the old Windows-only message is still present.

- [ ] **Step 3: Move target dependencies without pulling Windows APIs into macOS**

In `apps/clipline-app/Cargo.toml`, replace the current Windows dependency block with this structure:

```toml
# Tauri and the app runtime are enabled on Windows and macOS. Linux CI keeps
# the stub main so it does not need system webview libraries.
[target.'cfg(any(windows, target_os = "macos"))'.dependencies]
clipline-capture = { path = "../../crates/clipline-capture" }
clipline-events = { path = "../../crates/clipline-events" }
clipline-lol = { path = "../../crates/clipline-lol" }
clipline-mp4 = { path = "../../crates/clipline-mp4" }
clipline-storage = { path = "../../crates/clipline-storage" }
clipline-cloud-api = { git = "https://github.com/dain98/clipline-cloud", rev = "8dd49fcf35e5bb9acbf2681cc35c8d161cbfad5d" }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tauri = { version = "2", features = ["tray-icon", "protocol-asset"] }
tauri-plugin-autostart = "2"
tauri-plugin-global-shortcut = "2"
tauri-plugin-updater = "2"
chrono = "0.4.45"
rfd = "0.15"
png = "0.17"
base64 = "0.22"
rodio = { version = "0.17", default-features = false, features = ["vorbis"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_Security_Credentials",
    "Win32_Storage_FileSystem",
    "Win32_System_DataExchange",
    "Win32_System_Diagnostics_ToolHelp",
    "Win32_System_Memory",
    "Win32_System_Ole",
    "Win32_System_ProcessStatus",
    "Win32_System_SystemInformation",
    "Win32_System_Threading",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 4: Run Tauri build only for Windows/macOS**

Replace `apps/clipline-app/build.rs` with:

```rust
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" || target_os == "macos" {
        tauri_build::build();
    }
}
```

- [ ] **Step 5: Wire the entrypoint and module paths**

Replace `apps/clipline-app/src/main.rs` with:

```rust
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("clipline-app has a desktop runtime on Windows and macOS only");
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    app::run();
}

#[cfg(any(windows, target_os = "macos"))]
mod app;
#[cfg(any(windows, target_os = "macos"))]
mod cloud;
#[cfg(any(windows, target_os = "macos"))]
mod cloud_upload;
#[cfg(any(windows, target_os = "macos"))]
mod game_icon;
#[cfg(any(windows, target_os = "macos"))]
mod game_plugins;
#[cfg(any(windows, target_os = "macos"))]
mod games;
#[cfg(windows)]
mod hotkeys;
#[cfg(target_os = "macos")]
#[path = "hotkeys_macos.rs"]
mod hotkeys;
#[cfg(any(windows, target_os = "macos"))]
mod library;
#[cfg(any(windows, target_os = "macos"))]
mod markers;
#[cfg(windows)]
mod memory;
#[cfg(target_os = "macos")]
#[path = "memory_macos.rs"]
mod memory;
#[cfg(any(windows, target_os = "macos"))]
mod platform;
#[cfg(any(windows, target_os = "macos"))]
mod poster;
#[cfg(windows)]
mod service;
#[cfg(target_os = "macos")]
#[path = "service_macos.rs"]
mod service;
#[cfg(any(windows, target_os = "macos"))]
mod settings;
#[cfg(any(windows, target_os = "macos"))]
mod sound;
#[cfg(any(windows, target_os = "macos"))]
mod updates;
#[cfg(any(windows, target_os = "macos"))]
mod util;
```

- [ ] **Step 6: Run the contract**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: PASS. Full app compile may still fail until the platform/service stubs are added in later tasks.

- [ ] **Step 7: Commit**

```bash
git add apps/clipline-app/Cargo.toml apps/clipline-app/build.rs apps/clipline-app/src/main.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): target macos shell"
```

---

### Task 2: Platform Facade And Capabilities

**Files:**
- Create: `apps/clipline-app/src/platform/mod.rs`
- Create: `apps/clipline-app/src/platform/types.rs`
- Create: `apps/clipline-app/src/platform/windows.rs`
- Create: `apps/clipline-app/src/platform/macos.rs`
- Modify: `apps/clipline-app/src/games.rs`
- Modify: `apps/clipline-app/src/game_plugins.rs`

**Interfaces:**
- Consumes: `clipline_capture::windows::CapturableWindow` on Windows.
- Produces: `crate::platform::CapturableWindow`, `PlatformCapabilities`, display/audio facade functions, and macOS unavailable stubs.

- [ ] **Step 1: Write facade tests**

Append these tests to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn platform_facade_exposes_macos_capability_model() {
    let types = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/types.rs"),
    )
    .expect("read platform/types.rs");

    for required in [
        "pub struct PlatformCapabilities",
        "pub in_game_hotkey_fallback: CapabilityStatus",
        "pub hardware_encode: CapabilityStatus",
        "pub hdr_capture: CapabilityStatus",
        "pub player_decode: CapabilityStatus",
        "pub struct CapturableWindow",
    ] {
        assert!(types.contains(required), "missing platform type: {required}");
    }
}

#[test]
fn game_detection_uses_platform_window_type() {
    let games = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/games.rs"),
    )
    .expect("read games.rs");
    let plugins = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game_plugins.rs"),
    )
    .expect("read game_plugins.rs");

    assert!(games.contains("use crate::platform::CapturableWindow;"));
    assert!(plugins.contains("use crate::platform::CapturableWindow;"));
    assert!(
        !games.contains("clipline_capture::windows::CapturableWindow"),
        "game detection should not import Windows window types directly"
    );
    assert!(
        !plugins.contains("clipline_capture::windows::CapturableWindow"),
        "game plugins should not import Windows window types directly"
    );
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: FAIL because the platform module does not exist and games/plugins import Windows capture windows directly.

- [ ] **Step 3: Create platform types**

Create `apps/clipline-app/src/platform/types.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformOs {
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
```

- [ ] **Step 4: Create platform facade module**

Create `apps/clipline-app/src/platform/mod.rs`:

```rust
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
```

- [ ] **Step 5: Create Windows facade**

Create `apps/clipline-app/src/platform/windows.rs`:

```rust
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
```

- [ ] **Step 6: Create macOS facade**

Create `apps/clipline-app/src/platform/macos.rs`:

```rust
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
```

- [ ] **Step 7: Switch game detection to platform windows**

In `apps/clipline-app/src/games.rs`, replace the Windows import:

```rust
use clipline_capture::windows::{enumerate_capturable_windows, CapturableWindow};
```

with:

```rust
use crate::platform::{self, CapturableWindow};
```

Then replace both calls to `enumerate_capturable_windows()` with:

```rust
platform::enumerate_capturable_windows()
```

In `apps/clipline-app/src/game_plugins.rs`, replace both `clipline_capture::windows::CapturableWindow` imports with:

```rust
use crate::platform::CapturableWindow;
```

- [ ] **Step 8: Run facade tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add apps/clipline-app/src/platform apps/clipline-app/src/games.rs apps/clipline-app/src/game_plugins.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): add platform capability facade"
```

---

### Task 3: macOS Recorder Service Stub

**Files:**
- Create: `apps/clipline-app/src/service_macos.rs`

**Interfaces:**
- Consumes: app-facing uses of `crate::service` from `app.rs`, `settings/`, `library.rs`, and frontend commands.
- Produces: a macOS stub service with matching public types and command/event channels.

- [ ] **Step 1: Write service contract tests**

Append to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn macos_service_stub_exposes_app_facing_contract() {
    let service = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service_macos.rs"),
    )
    .expect("read service_macos.rs");

    for required in [
        "pub enum Cmd",
        "pub enum Event",
        "pub struct ServiceOptions",
        "pub enum CaptureBackend",
        "pub enum VideoEncoder",
        "pub fn spawn(opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>)",
        "pub fn default_clips_dir() -> PathBuf",
        "pub fn clips_dir(root: &Path) -> Result<PathBuf, String>",
        "pub fn available_encoder_options() -> Vec<EncoderOption>",
    ] {
        assert!(service.contains(required), "missing service contract: {required}");
    }
}
```

- [ ] **Step 2: Run and verify failure**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: FAIL because `service_macos.rs` does not exist.

- [ ] **Step 3: Add the macOS service stub**

Create `apps/clipline-app/src/service_macos.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

pub use clipline_capture::probe::Codec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRegion {
    pub display_id: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    PrimaryMonitor,
    WindowTitle(String),
    WindowHandle { hwnd: isize, title: String },
    DisplayRegion(CaptureRegion),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    #[default]
    Auto,
    Wgc,
    DesktopDuplication,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioChannelMode {
    #[default]
    Mono,
    Stereo,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    #[default]
    Auto,
    NvencH264,
    NvencHevc,
    NvencAv1,
    AmfH264,
    AmfHevc,
    AmfAv1,
    QuickSyncH264,
    QuickSyncHevc,
    QuickSyncAv1,
    SvtAv1,
}

pub fn codec_id(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "av1",
        Codec::Hevc => "hevc",
        Codec::H264 => "h264",
    }
}

#[derive(serde::Serialize)]
pub struct EncoderOption {
    pub id: String,
    pub name: String,
    pub codec: String,
}

pub fn available_encoder_options() -> Vec<EncoderOption> {
    Vec::new()
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioOptions {
    pub output_enabled: bool,
    pub output_device_id: Option<String>,
    pub output_volume: f64,
    pub split_output_by_process: bool,
    pub mic_enabled: bool,
    pub mic_device_id: Option<String>,
    pub mic_volume: f64,
    pub mic_channels: AudioChannelMode,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            output_enabled: true,
            output_device_id: None,
            output_volume: 1.0,
            split_output_by_process: false,
            mic_enabled: false,
            mic_device_id: None,
            mic_volume: 1.0,
            mic_channels: AudioChannelMode::Mono,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ReplayStorageOptions {
    #[default]
    Memory,
    Disk { dir: PathBuf, quota_bytes: u64 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordingMode {
    FullSession,
    #[default]
    ReplaysOnly,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum OutputResolution {
    #[default]
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "480p")]
    P480,
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Status {
        recording: bool,
        segments: usize,
        buffered_s: f64,
        buffered_mb: f64,
        #[serde(default)]
        full_session: bool,
        #[serde(default)]
        encoder: String,
    },
    Saved {
        path: String,
        seconds: f64,
        markers: usize,
        #[serde(default)]
        full_session: bool,
        gc_deleted: usize,
        gc_freed_bytes: u64,
        storage_total_bytes: u64,
        storage_quota_bytes: Option<u64>,
        storage_over_quota: bool,
    },
    Error { message: String },
}

pub enum Cmd {
    Save,
    Stop { announce: bool },
}

#[derive(Clone, Debug)]
pub struct ActiveGame {
    pub id: String,
    pub name: String,
}

pub struct ServiceOptions {
    pub capture_source: CaptureSource,
    pub capture_backend: CaptureBackend,
    pub active_game_plugin_id: Option<String>,
    pub active_game: Option<ActiveGame>,
    pub media_dir: PathBuf,
    pub lol_url: Option<String>,
    pub replay_window_s: f64,
    pub buffer_bytes: usize,
    pub replay_storage: ReplayStorageOptions,
    pub disk_quota_bytes: Option<u64>,
    pub recording_mode: RecordingMode,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub video_encoder: VideoEncoder,
    pub output_resolution: OutputResolution,
    pub decodable_codecs: Vec<Codec>,
    pub audio: AudioOptions,
}

pub const DEFAULT_DISK_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024;

impl Default for ServiceOptions {
    fn default() -> Self {
        Self {
            capture_source: CaptureSource::PrimaryMonitor,
            capture_backend: CaptureBackend::Auto,
            active_game_plugin_id: None,
            active_game: None,
            media_dir: default_clips_dir(),
            lol_url: None,
            replay_window_s: 60.0,
            buffer_bytes: 220 * 1024 * 1024,
            replay_storage: ReplayStorageOptions::Memory,
            disk_quota_bytes: Some(DEFAULT_DISK_QUOTA_BYTES),
            recording_mode: RecordingMode::ReplaysOnly,
            fps: 60,
            bitrate_bps: 12_000_000,
            video_encoder: VideoEncoder::Auto,
            output_resolution: OutputResolution::Source,
            decodable_codecs: vec![Codec::H264],
            audio: AudioOptions::default(),
        }
    }
}

pub fn default_clips_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Movies").join("Clipline"))
        .unwrap_or_else(|| std::env::temp_dir().join("Clipline"))
}

pub fn clips_dir(root: &Path) -> Result<PathBuf, String> {
    if root.as_os_str().is_empty() {
        return Err("media folder is required".into());
    }
    Ok(root.to_path_buf())
}

pub fn spawn(_opts: ServiceOptions) -> (Sender<Cmd>, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipline-recorder-stub".into())
        .spawn(move || {
            let _ = event_tx.send(Event::Status {
                recording: false,
                segments: 0,
                buffered_s: 0.0,
                buffered_mb: 0.0,
                full_session: false,
                encoder: "Unavailable on macOS M1".into(),
            });
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Cmd::Save => {
                        let _ = event_tx.send(Event::Error {
                            message: "macOS recording is not implemented in Milestone 1".into(),
                        });
                    }
                    Cmd::Stop { announce } => {
                        if announce {
                            let _ = event_tx.send(Event::Status {
                                recording: false,
                                segments: 0,
                                buffered_s: 0.0,
                                buffered_mb: 0.0,
                                full_session: false,
                                encoder: String::new(),
                            });
                        }
                        break;
                    }
                }
            }
        })
        .expect("spawn macOS recorder stub thread");
    (cmd_tx, event_rx)
}
```

- [ ] **Step 4: Run service contract**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/clipline-app/src/service_macos.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): add macos recorder service stub"
```

---

### Task 4: Cross-Platform App Command Wiring

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Create: `apps/clipline-app/src/hotkeys_macos.rs`
- Create: `apps/clipline-app/src/memory_macos.rs`

**Interfaces:**
- Consumes: `crate::platform::{DisplayInfo, AudioDeviceLists, capabilities, list_displays, list_audio_devices, memory_status}`.
- Produces: app command bodies that compile on macOS and report unavailable stubs.

- [ ] **Step 1: Write app platform contract tests**

Append to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn app_commands_use_platform_facade() {
    let app = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs"))
        .expect("read app.rs");

    for forbidden in [
        "clipline_capture::windows::display::enumerate_displays",
        "clipline_capture::windows::wasapi::enumerate_audio_devices",
        "crate::memory::current_process_tree_memory()",
        "clipline_capture::windows::qpc_now_ticks_100ns",
        "WasapiLoopback::start_microphone",
    ] {
        assert!(
            !app.contains(forbidden),
            "app.rs should call platform facade instead of {forbidden}"
        );
    }
    for required in [
        "use crate::platform::{AudioDeviceLists, DisplayInfo};",
        "crate::platform::list_displays()",
        "crate::platform::list_audio_devices()",
        "crate::platform::memory_status()",
    ] {
        assert!(app.contains(required), "missing platform app usage: {required}");
    }
}

#[test]
fn macos_hotkey_and_memory_stubs_exist() {
    let hotkeys = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hotkeys_macos.rs"),
    )
    .expect("read hotkeys_macos.rs");
    let memory = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/memory_macos.rs"),
    )
    .expect("read memory_macos.rs");

    assert!(hotkeys.contains("pub fn install_save_hook"));
    assert!(hotkeys.contains("focused-game hotkey fallback"));
    assert!(hotkeys.contains("pub fn set_save_hotkey"));
    assert!(memory.contains("pub struct MemoryStatus"));
    assert!(memory.contains("private_working_set_bytes"));
}
```

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: FAIL because `app.rs` still calls Windows display/audio/mic/memory APIs and the macOS stubs do not exist.

- [ ] **Step 3: Add macOS hotkey stub**

Create `apps/clipline-app/src/hotkeys_macos.rs`:

```rust
pub fn install_save_hook<F>(_hotkey: &str, _on_trigger: F) -> Result<(), String>
where
    F: Fn() + Send + Sync + 'static,
{
    Err("macOS focused-game hotkey fallback is not implemented in Milestone 1".into())
}

pub fn set_save_hotkey(_hotkey: &str) -> Result<(), String> {
    Ok(())
}
```

- [ ] **Step 4: Add macOS memory stub**

Create `apps/clipline-app/src/memory_macos.rs`:

```rust
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    Err("macOS memory status is not implemented in Milestone 1".into())
}
```

- [ ] **Step 5: Update app imports and command bodies**

In `apps/clipline-app/src/app.rs`, add the platform import near the existing crate imports:

```rust
use crate::platform::{AudioDeviceLists, DisplayInfo};
```

Remove the local `DisplayInfo`, `AudioDeviceInfo`, and `AudioDeviceLists` struct definitions from `app.rs`.

Replace `memory_status` with:

```rust
#[tauri::command]
fn memory_status() -> Result<crate::memory::MemoryStatus, String> {
    crate::platform::memory_status()
}
```

Replace `list_displays` with:

```rust
#[tauri::command]
fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    crate::platform::list_displays()
}
```

Replace `list_audio_devices` with:

```rust
#[tauri::command]
fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    crate::platform::list_audio_devices()
}
```

Replace the entire body of `start_microphone_test` with a platform-gated implementation:

```rust
#[tauri::command]
fn start_microphone_test<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<MicTestState>,
    device_id: Option<String>,
    volume: f64,
    mono: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        start_microphone_test_windows(app, state, device_id, volume, mono)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (app, state, device_id, volume, mono);
        Err("macOS microphone test is not implemented in Milestone 1".into())
    }
}

#[cfg(windows)]
fn start_microphone_test_windows<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<MicTestState>,
    device_id: Option<String>,
    volume: f64,
    mono: bool,
) -> Result<(), String> {
    state.stop();
    let channels = if mono {
        clipline_capture::windows::wasapi::WasapiChannelMode::Mono
    } else {
        clipline_capture::windows::wasapi::WasapiChannelMode::Stereo
    };
    let (stop_tx, stop_rx) = mpsc::channel();
    {
        let mut guard = state.0.lock().map_err(|_| "mic test state lock poisoned")?;
        *guard = Some(stop_tx);
    }
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let clock = clipline_capture::clock::RelativeClock::new(
                clipline_capture::windows::qpc_now_ticks_100ns().map_err(|e| e.to_string())?,
            );
            let mut source = clipline_capture::windows::wasapi::WasapiLoopback::start_microphone(
                clock,
                device_id.as_deref(),
                volume,
                channels,
            )
            .map_err(|e| e.to_string())?;
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                let packets = source.poll_pcm_packets(0.03).map_err(|e| e.to_string())?;
                for packet in packets {
                    let rms = packet.rms();
                    let peak = packet.peak();
                    let samples = packet
                        .samples
                        .iter()
                        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                        .collect::<Vec<_>>();
                    let _ = app.emit(
                        "mic-test",
                        MicMonitorEvent {
                            rms,
                            peak,
                            sample_count: samples.len(),
                            samples,
                        },
                    );
                }
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = app.emit("mic-test-error", e);
            let _ = app.emit("mic-test-stopped", ());
        }
    });
    Ok(())
}
```

If `poll_pcm_packets`, `rms`, or `peak` are private to the current function body, keep the existing Windows body exactly as it is and only wrap it in the `start_microphone_test_windows` helper.

- [ ] **Step 6: Run app platform contract**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/clipline-app/src/app.rs apps/clipline-app/src/hotkeys_macos.rs apps/clipline-app/src/memory_macos.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): route shell commands through platform stubs"
```

---

### Task 5: Cross-Platform Filesystem, Cloud, Icon, And Settings Compile

**Files:**
- Modify: `apps/clipline-app/src/settings/persistence.rs`
- Modify: `apps/clipline-app/src/util.rs`
- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/game_icon.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Consumes: neutral library/cloud/settings behavior and Windows-specific OS helpers.
- Produces: macOS-safe compile path with Finder/Keychain/clipboard unavailable where not implemented.

- [ ] **Step 1: Write static platform-safety tests**

Append to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn os_specific_helpers_are_cfg_gated() {
    let persistence = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings/persistence.rs"),
    )
    .expect("read persistence.rs");
    let util = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/util.rs"))
        .expect("read util.rs");
    let cloud = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud.rs"))
        .expect("read cloud.rs");
    let library = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs"))
        .expect("read library.rs");
    let game_icon = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/game_icon.rs"),
    )
    .expect("read game_icon.rs");

    assert!(persistence.contains("#[cfg(windows)]\nfn replace_file"));
    assert!(persistence.contains("#[cfg(not(windows))]\nfn replace_file"));
    assert!(util.contains("#[cfg(windows)]\npub(crate) fn wide_null"));
    assert!(cloud.contains("#[cfg(windows)]\nfn write_credential"));
    assert!(cloud.contains("#[cfg(target_os = \"macos\")]\nfn write_credential"));
    assert!(library.contains("#[cfg(windows)]\nfn copy_file_to_clipboard"));
    assert!(library.contains("#[cfg(target_os = \"macos\")]\nfn copy_file_to_clipboard"));
    assert!(game_icon.contains("#[cfg(target_os = \"macos\")]\npub fn extract_exe_icon_png"));
}
```

- [ ] **Step 2: Verify failure**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: FAIL because these helpers are not cfg-gated yet.

- [ ] **Step 3: Gate settings atomic replace and macOS config folder**

In `settings/persistence.rs`, gate the Windows import:

```rust
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
```

Replace `config_base()` with:

```rust
pub fn config_base() -> PathBuf {
    #[cfg(windows)]
    {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|home| home.join("AppData").join("Roaming"))
            })
            .unwrap_or_else(std::env::temp_dir)
            .join("Clipline");
    }

    #[cfg(target_os = "macos")]
    {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join("Clipline")
            })
            .unwrap_or_else(|| std::env::temp_dir().join("Clipline"));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::temp_dir().join("Clipline")
    }
}
```

Gate `replace_file`:

```rust
#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    let from = wide_null(from.as_os_str());
    let to = wide_null(to.as_os_str());
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(format!("replace settings file: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| format!("replace settings file: {e}"))
}
```

If `wide_null` is currently private in this file, keep the existing helper or call `crate::util::wide_null` after Task 5 Step 4 gates it.

- [ ] **Step 4: Gate util Win32 helpers**

In `util.rs`, change imports:

```rust
#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
```

Gate Win32 helpers:

```rust
#[cfg(windows)]
pub(crate) fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
pub(crate) fn last_os_error(action: &str) -> String {
    format!("{action}: {}", std::io::Error::last_os_error())
}
```

- [ ] **Step 5: Gate library Windows helpers**

In `library.rs`, wrap these imports in `#[cfg(windows)]`:

```rust
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::ptr;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
#[cfg(windows)]
use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, SetClipboardData};
#[cfg(windows)]
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
#[cfg(windows)]
use windows_sys::Win32::System::Ole::CF_HDROP;
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::DROPFILES;
```

Gate the existing Windows implementations:

```rust
#[cfg(windows)]
fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    // existing Windows CF_HDROP implementation stays here unchanged
}

#[cfg(target_os = "macos")]
fn copy_file_to_clipboard(_path: &Path) -> Result<(), String> {
    Err("Finder clipboard copy is not implemented in Milestone 1".into())
}
```

Add `#[cfg(windows)]` to `dropfiles_payload`, `shell_clipboard_path_wide`, `ClipboardTransfer`, and `ClipboardClose`.

Gate `open_folder_path`:

```rust
#[cfg(windows)]
fn open_folder_path(dir: &Path) -> Result<(), String> {
    Command::new("explorer")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("open folder {dir:?}: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_folder_path(dir: &Path) -> Result<(), String> {
    Command::new("open")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("open folder {dir:?}: {e}"))?;
    Ok(())
}
```

Add `#[cfg(windows)]` to tests that directly assert `CF_HDROP`, `DROPFILES`, or Windows path conversion.

- [ ] **Step 6: Gate cloud credentials**

In `cloud.rs`, gate Windows imports:

```rust
#[cfg(windows)]
use std::{ffi::OsStr, ptr, slice};
#[cfg(windows)]
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};
```

Add `#[cfg(windows)]` to existing `write_credential`, `read_credential`, `delete_credential`, and `CredentialFree`.

Add macOS stubs:

```rust
#[cfg(target_os = "macos")]
fn write_credential(_target: &str, _username: &str, _token: &str) -> Result<(), String> {
    Err("macOS Keychain storage is not implemented in Milestone 1".into())
}

#[cfg(target_os = "macos")]
fn read_credential(_target: &str) -> Result<String, String> {
    Err("macOS Keychain storage is not implemented in Milestone 1".into())
}

#[cfg(target_os = "macos")]
fn delete_credential(_target: &str) -> Result<(), String> {
    Ok(())
}
```

- [ ] **Step 7: Gate game icon extraction**

In `game_icon.rs`, gate all Windows imports and `icon_to_png` helpers with `#[cfg(windows)]`.

Keep `png_data_url` and `encode_rgba_png` shared.

Add:

```rust
#[cfg(target_os = "macos")]
pub fn extract_exe_icon_png(_exe_path: &str) -> Option<Vec<u8>> {
    None
}
```

Keep:

```rust
pub fn extract_exe_icon_data_url(exe_path: &str) -> Option<String> {
    extract_exe_icon_png(exe_path).map(|png| png_data_url(&png))
}
```

- [ ] **Step 8: Update UI copy contract only if copy changes**

If `index.html` still says `title="Show this clip in Explorer"`, change it to a platform-neutral label:

```html
<button id="open-folder" class="icon-button" title="Show this clip in folder">
```

If `tests/ui_contract.rs` asserts the Explorer copy, update the assertion to the same platform-neutral text. Do not add runtime frontend platform branching in M1.

- [ ] **Step 9: Run static safety tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add apps/clipline-app/src/settings/persistence.rs apps/clipline-app/src/util.rs apps/clipline-app/src/library.rs apps/clipline-app/src/cloud.rs apps/clipline-app/src/game_icon.rs apps/clipline-app/ui/index.html apps/clipline-app/tests/ui_contract.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): gate desktop os helpers"
```

---

### Task 6: macOS Compile And UI Launch Verification

**Files:**
- Modify as needed: files touched by prior tasks only.

**Interfaces:**
- Consumes: all M1 stubs.
- Produces: proof that the real `clipline-app` crate compiles on macOS and keeps existing contract tests passing.

- [ ] **Step 1: Run the app crate tests**

Run:

```bash
cargo test -p clipline-app
```

Expected: PASS for `player_core`, `ui_contract`, and `macos_shell_contract` on macOS. If compile errors mention a Windows-only import, fix by moving that API behind `#[cfg(windows)]` or a macOS stub, then rerun.

- [ ] **Step 2: Run workspace tests**

Run:

```bash
cargo test --workspace
```

Expected: PASS on macOS. This now includes the real app crate dependency graph for macOS, not only the old stub.

- [ ] **Step 3: Run clippy on changed crate**

Run:

```bash
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: PASS. Fix all warnings rather than allowing them.

- [ ] **Step 4: Launch the macOS app shell**

Run:

```bash
cargo run -p clipline-app
```

Expected: The Tauri window opens with the existing Clipline UI. Recording controls may report unavailable; Save Replay should show a visible error instead of crashing. Stop the app from the tray/menu or with `Ctrl+C` in the terminal after confirming first paint.

- [ ] **Step 5: Check Git state**

Run:

```bash
git status --short --branch
```

Expected: only intentional source changes remain.

- [ ] **Step 6: Commit final verification fixes**

If Task 6 required fixes, commit them:

```bash
git add apps/clipline-app
git commit -m "fix(app): compile macos shell"
```

If Task 6 required no fixes, do not create an empty commit.

---

## Plan Self-Review

- Spec coverage: This plan implements the first decomposition item from `docs/superpowers/specs/2026-06-23-macos-port-design.md`: real app crate compile on macOS, platform facade types, Windows preservation, and capability stubs. It intentionally leaves ScreenCaptureKit, CoreAudio, Keychain, VideoToolbox, and CGEventTap to later milestones.
- Red-flag phrase scan: No step uses the banned vague phrases from the planning checklist. Each stub has an explicit user-facing unavailable message.
- Type consistency: `PlatformCapabilities`, `CapabilityStatus`, `CapturableWindow`, `ServiceOptions`, `Cmd`, `Event`, `CaptureBackend`, `AudioChannelMode`, `VideoEncoder`, `RecordingMode`, and `OutputResolution` names match the spec and current app imports.
- Verification: The milestone is complete only when `cargo test -p clipline-app`, `cargo test --workspace`, `cargo clippy -p clipline-app --all-targets -- -D warnings`, and `cargo run -p clipline-app` have been run on macOS with the real app shell.
