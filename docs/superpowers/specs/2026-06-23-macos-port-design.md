# Clipline macOS Port Design

## Purpose

Port Clipline to macOS without creating a drifting macOS-only fork. Every Windows Clipline feature must become possible on macOS through a native implementation, a capability-aware equivalent, or an explicit user-facing fallback where macOS does not expose the same OS primitive.

The approved repository shape is one cross-platform Clipline codebase. The existing Windows implementation remains first-class, and the macOS port adds platform adapters around the tested neutral crates and UI.

## Current Evidence

- `apps/clipline-app/src/main.rs` currently compiles the real app only on Windows and prints a non-Windows stub message elsewhere.
- `apps/clipline-app/Cargo.toml` currently gates `clipline-capture`, `clipline-events`, `clipline-lol`, `clipline-mp4`, `clipline-storage`, Tauri, and the app's runtime dependencies behind `[target.'cfg(windows)'.dependencies]`.
- `apps/clipline-app/src/service.rs`, `hotkeys.rs`, `memory.rs`, `library.rs`, `cloud.rs`, settings persistence, and app-facing copy/tests contain Windows-specific APIs or assumptions. `app.rs` and `games.rs` are closer to facades, but they are still gated behind `#[cfg(windows)]` today.
- The neutral crates already build and pass on macOS: `clipline-events`, `clipline-lol`, `clipline-buffer`, `clipline-storage`, `clipline-mp4`, and most of `clipline-capture`.
- Baseline command on macOS: `cargo test --workspace` passed with all current tests, while `clipline-app` contributed only stub/non-platform tests. This is not proof that the real app layer compiles on macOS; the first milestone must make that compile happen for the first time.
- Apple documents ScreenCaptureKit for high-performance macOS screen, window, and audio capture: <https://developer.apple.com/documentation/ScreenCaptureKit>.
- Tauri 2 supports macOS application bundles, updater, autostart, and global shortcut plugins: <https://v2.tauri.app/distribute/macos-application-bundle/>, <https://v2.tauri.app/plugin/autostart/>, <https://v2.tauri.app/plugin/global-shortcut/>, <https://v2.tauri.app/plugin/updater/>.

## Design Decision

Use a shared Rust/Tauri app with platform adapters.

Clipline keeps one UI, one library/review/cloud/settings surface, one MP4/trim/storage/event schema, and one recorder pipeline contract. Platform-specific code lives behind small interfaces for capture, audio, displays/windows, hotkeys, startup, file reveal/copy, memory status, package/update behavior, and permissions.

The abstraction must be capability-based, not lowest-common-denominator. Windows and macOS should each report the concrete capabilities available on the current machine, and the UI should show enabled controls, warnings, or fallbacks from those capabilities.

## Goals

1. Run the real Clipline Tauri UI on macOS instead of the current stub.
2. Preserve existing Windows behavior and tests.
3. Reuse neutral Rust crates and vanilla HTML/CSS/JS UI wherever behavior is not OS-specific.
4. Add macOS-native equivalents for capture, audio, hotkeys, tray/menu, startup, window/game detection, memory status, file reveal/copy, packaging, updates, and permissions.
5. Keep the local-first privacy stance: no telemetry, no account required for recording, no game injection, no kernel driver, no memory reading.
6. Keep every parity gap visible in capability status, settings copy, and tests until the native implementation lands.

## Non-Goals

- Do not create a separate macOS-only application with duplicated UI and business logic.
- Do not replace Tauri with Swift/AppKit for the first port.
- Do not adopt libobs or GPL-only capture code.
- Do not fake support for unavailable macOS capabilities; unsupported paths must report a concrete status and fallback.

## Feature Parity Map

| Clipline feature | macOS design |
|---|---|
| Real app shell, hidden-at-start, tray/menu open | Run the same Tauri UI on macOS. Use macOS menu bar/tray behavior through Tauri tray APIs, with platform copy that says Finder instead of Explorer. |
| Save replay hotkey | Keep `tauri-plugin-global-shortcut` as the main shortcut path on both Windows and macOS. Windows also has a `WH_KEYBOARD_LL` fallback for focused games; macOS needs a separate in-game-hotkey capability for any CGEventTap/Input Monitoring fallback. If macOS cannot install that fallback, report the gap and keep tray/menu/manual save available. |
| Replay buffer | Reuse `clipline-buffer` and `clipline-capture::pipeline`; platform capture feeds the same encoded GOP segments. |
| Full-session recording | Reuse the existing full-session sink after macOS capture and encoder packets enter the shared recorder. |
| Display capture | Implement a macOS ScreenCaptureKit display stream adapter. Surface Screen Recording permission errors with a Settings action. |
| Window capture | Implement a ScreenCaptureKit window content filter adapter. Use macOS window ids/titles/process metadata instead of HWNDs. |
| Display region capture | Reuse the UI and settings model, with macOS display enumeration and region crop mapping. Crop before encode or through the capture configuration when the API allows it. |
| Capture backend selector | Replace Windows-specific WGC/DXGI copy with platform-aware options. macOS initially exposes ScreenCaptureKit only. |
| Hardware/software encoding | Keep the `Encoder` trait and FFmpeg subprocess path. Add macOS encoder capabilities in the probe list and in `PlatformCapabilities`, starting with FFmpeg hardware/software encoders available on the machine. Evaluate VideoToolbox as a later native backend behind the same trait. |
| Explicit SDR color metadata | Preserve Rec.709 limited metadata in `clipline-mp4`; macOS capture adapter must normalize or label source color consistently before encode. HDR is a later capability flag, not the initial default. |
| System/output audio | Use ScreenCaptureKit stream audio when available. If a macOS version or permission does not provide system audio, report unavailable and allow mic-only/video-only recording. |
| Microphone capture/test | Add a macOS CoreAudio/AVAudioEngine-backed mic source and level monitor behind the existing mic UI contract. |
| Per-process output audio tracks | Treat as a capability. The expected macOS v1 outcome is `per_process_audio: unavailable` with a mixed system output plus mic fallback, because the Windows implementation depends on WASAPI process loopback and there is no stable public macOS equivalent. Future work can revisit this only if a supportable OS path is identified. |
| Multi-track MP4/review/upload selection | Reuse `clipline-mp4` multitrack support, sidecar `audio_tracks`, review checklists, preview remux, and cloud upload remux once macOS audio sources produce labeled tracks. |
| League event markers | Reuse `clipline-lol` and `clipline-events`; League's local Live Client API remains loopback HTTP and platform-neutral. |
| Built-in League plugin auto-recording | Match League's macOS game process/window metadata and reuse the plugin registry. The plugin must not match Riot launcher/client-only windows as a recording target. |
| Custom game detection | Replace Win32 window enumeration with macOS shareable window/application enumeration. Persist the same logical custom game settings with macOS process path, executable name, title, and icon where available. |
| Library/gallery/review player | Reuse existing UI, player-core logic, marker sidecars, poster generation, rename/delete/export flows, grouping, searching, sorting, and cloud badges. |
| WKWebView playback | Treat review playback as a macOS parity gate. The UI uses a WebView `<video>` element, and WKWebView codec support differs from WebView2; macOS must verify H.264 playback and warn/fallback for codecs the local WebView cannot decode. |
| Lossless trim/remux | Reuse `clipline-mp4::trim_keyframe_aligned` and selected-audio remux logic. |
| File reveal/copy | Replace Windows Explorer/CF_HDROP code with macOS Finder reveal and pasteboard file URL behavior. Prefer a small platform interface over UI branching. Update `index.html` copy and `tests/ui_contract.rs` together because the UI contract asserts this surface. |
| Storage quota/session folders/recovery | Reuse `clipline-storage`, session folder labels, `.mp4.recording` recovery, media folder setting, and disk replay cache logic. Adjust default media folder to the macOS Movies directory when available. |
| Cloud connect/upload/status | Reuse cloud UI and upload commands. Replace Windows Credential Manager storage with macOS Keychain storage through a credential interface. |
| Open on startup | Keep the setting and use Tauri autostart on macOS. If release signing/packaging affects behavior, report the effective status instead of silently claiming success. |
| Updater | Keep Tauri updater and GitHub Releases flow, adding macOS bundle artifacts and signatures to release configuration. |
| Memory readout | Replace ToolHelp/process APIs with a macOS process-memory implementation. Until complete, show an unavailable status instead of a fake number. |
| Permissions and signing | Add macOS permission checks/status for Screen Recording, Microphone, and any needed Accessibility/Input Monitoring path. Use a signed development build as soon as permission testing starts, because TCC grants are tied to app identity and unsigned rebuilds can invalidate prior grants. |

## Architecture

### Shared App Layer

The app layer should be split so Tauri commands and UI events are platform-neutral wherever possible:

- `app.rs` owns Tauri setup, tray/menu, commands, runtime state, and event wiring.
- `service.rs` owns recorder lifecycle and command/event channels.
- `settings/` owns persisted settings, validation, and service-option conversion.
- `library.rs`, `cloud.rs`, `games.rs`, `memory.rs`, `hotkeys.rs`, and `util.rs` should stop importing Windows APIs directly. Each should call a platform interface instead.

The first implementation slice should move code only as far as needed to let macOS compile and open the real UI with capability stubs. Native media capture comes after the boundary exists.

### Platform Layer

Create platform modules with one public facade:

```text
apps/clipline-app/src/platform/mod.rs
apps/clipline-app/src/platform/windows.rs
apps/clipline-app/src/platform/macos.rs
apps/clipline-app/src/platform/types.rs
```

The facade exposes stable app-facing functions and structs:

- `platform::list_displays() -> Result<Vec<DisplayInfo>, String>`
- `platform::list_capture_windows() -> Vec<GameWindowInfo>`
- `platform::extract_window_icon(path: &str) -> Result<Option<String>, String>`
- `platform::list_audio_devices() -> Result<AudioDeviceLists, String>`
- `platform::start_microphone_test(...) -> Result<MicMonitorHandle, String>`
- `platform::memory_status() -> Result<MemoryStatus, String>`
- `platform::reveal_path(path: &Path) -> Result<(), String>`
- `platform::copy_file_to_clipboard(path: &Path) -> Result<(), String>`
- `platform::credential_store() -> impl CredentialStore`
- `platform::capabilities() -> PlatformCapabilities`

Recorder media adapters belong in `clipline-capture`, not `clipline-app`, once they are real:

```text
crates/clipline-capture/src/macos/mod.rs
crates/clipline-capture/src/macos/screencapturekit.rs
crates/clipline-capture/src/macos/audio.rs
crates/clipline-capture/src/macos/display.rs
crates/clipline-capture/src/macos/window.rs
crates/clipline-capture/src/macos/videotoolbox.rs
```

`clipline-capture::FrameData` must gain a macOS GPU/IOSurface/CVPixelBuffer-backed variant only when the encoder can consume it safely. The initial ScreenCaptureKit path may use a CPU/NV12 staging route through FFmpeg if that gets a correct, testable recorder sooner.

### Capability Model

Add a serializable `PlatformCapabilities` model:

```rust
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
```

Each `CapabilityStatus` includes `available: bool`, `reason: Option<String>`, and `action: Option<PermissionAction>`. The UI uses this for disabled controls and warnings. This keeps macOS differences honest without splitting the frontend.

## Data Flow

1. macOS capture creates timestamped video frames from ScreenCaptureKit.
2. macOS audio sources create timestamped Opus packets or PCM that is encoded through existing audio helpers.
3. The shared `Recorder` groups encoded packets into GOP-aligned segments.
4. Segments fan out to the replay ring and optional full-session writer.
5. Save replay/export/library/cloud flows continue using existing MP4, marker sidecar, storage, and cloud code.
6. UI receives the same `status`, `saved`, `error`, `game-detection`, `mic-test`, and `cloud-upload-progress` events.

## Error Handling And Permissions

macOS errors must be actionable:

- Missing Screen Recording permission: keep the app open, stop capture, show a settings-facing error with a concrete action.
- Missing Microphone permission: disable mic capture/test, allow video/system audio if available.
- Global shortcut registration failure: continue running, keep manual save available, and let the user rebind.
- In-game hotkey fallback unavailable: keep the primary global shortcut and manual save path, but show that focused fullscreen games may not trigger Save Replay reliably on this machine.
- System audio unavailable: record video and any enabled mic; sidecars must accurately describe available audio tracks.
- Player decode unsupported for the selected codec: allow recording only when the user explicitly chooses that codec after a warning, and make Automatic prefer codecs the local WebView can play.
- Capture stream interruption or display/window disappearance: finalize any active full session, fall back to configured display capture when safe, and emit a visible warning.
- Keychain/credential failures: leave recording features unaffected and show cloud as disconnected or errored.

No platform fallback may silently record the wrong target. Display capture must remain visibly different from window/game capture because it can include notifications or other apps.

## Testing Strategy

The port keeps the existing green baseline and adds platform-specific proof in layers:

1. `cargo test --workspace` on macOS and Windows remains required.
2. UI contract tests cover platform copy changes, disabled capability states, and settings visibility.
3. Platform facade unit tests use pure data where possible: display bounds, window matching, settings conversion, capability rendering, and path validation.
4. macOS capture smoke tests are ignored by default or self-skip without permissions/hardware, matching the current Windows device-test pattern.
5. Milestone 1 must prove the real `clipline-app` crate compiles on macOS with platform stubs; the current non-Windows stub is not sufficient evidence.
6. Real macOS validation records short clips for display capture, window capture, system audio, mic audio, replay save, full-session finalize, trim export, and library playback.
7. Review-player validation must open a recorded clip in WKWebView, verify playback, and verify the codec warning path for unsupported codecs.
8. Permission validation should use a signed development app identity once Screen Recording, Microphone, or Input Monitoring prompts are under test.
9. Release validation builds a macOS `.app`/DMG or configured Tauri bundle, confirms updater metadata, and verifies startup behavior in an installed app context.

## Implementation Decomposition

This port is too large for one implementation plan. It should be split into independently testable plans:

1. **macOS app shell and platform boundaries**: make `clipline-app` compile on macOS for the first time with the real Tauri UI, introduce platform facade types, keep Windows behavior intact, and show capability stubs. This plan must move the necessary app dependencies out from Windows-only target dependencies without pulling Windows APIs into macOS builds.
2. **macOS filesystem, settings, credentials, and app lifecycle**: Finder reveal, pasteboard copy, Movies default folder, Keychain cloud credential storage, startup/login status, memory status, updater/bundle config, signed-development-app prerequisites for permission testing, and platform copy cleanup.
3. **macOS display/window enumeration and game detection**: ScreenCaptureKit shareable content or AppKit/CoreGraphics metadata, custom game rules, League plugin matching, and game icon extraction.
4. **macOS recording core v1**: ScreenCaptureKit display/window capture, FFmpeg encode path, replay save, full-session recording, quota/recovery, and real smoke tests.
5. **macOS audio v1**: system audio when available, microphone capture/test, mixed output/mic track parity, explicit `per_process_audio: unavailable` fallback unless a stable public API is found, and sidecar labels.
6. **macOS review/cloud parity hardening**: WKWebView playback compatibility, multitrack preview/upload behavior, file locks, updater artifacts, and release verification.
7. **macOS advanced parity**: per-process audio where publicly supportable, VideoToolbox native encoder, HDR handling, and any feature that needs separate OS-version gating.

The immediate next plan should be item 1.

## Risks

- ScreenCaptureKit permissions and OS-version behavior can make capture unavailable until the user grants access. The app must handle this as normal state.
- The current app crate has never compiled the real app layer on macOS. Milestone 1 is first-ever app-crate compilation with stubs, not merely preservation of the existing green workspace test baseline.
- Per-process output audio is expected to be unavailable in macOS v1 because there is no stable public equivalent to the Windows process-loopback path. The design allows a truthful fallback while preserving future room.
- The Windows low-level save-hotkey fallback has no direct macOS equivalent. A CGEventTap-style fallback may require Accessibility/Input Monitoring permission and may still be blocked by secure input, so this remains a named capability risk.
- Tauri/macOS WKWebView playback support differs from WebView2. Automatic encoder selection must continue to prefer player-decodable codecs and warn for limited playback, and M6 must verify actual library/review playback.
- The existing app layer has direct Windows imports in many files. The first boundary slice must be conservative to avoid breaking Windows while making macOS compile.
- Release behavior and permission testing on macOS depend on signing/notarization choices. Development builds can run unsigned, but TCC permission grants and login-item behavior are more reliable once the app has a stable signed identity, and user-distributed builds need a signed/notarized path.

## Acceptance Criteria For This Design

- The project remains one cross-platform Clipline repository.
- Windows feature behavior is not intentionally reduced.
- macOS feature parity is represented by concrete platform capabilities and implementation plans.
- The first implementation milestone explicitly compiles the real app crate on macOS with platform stubs and is narrow enough to verify without needing real capture.
- Later milestones can prove parity feature-by-feature against the map above.
