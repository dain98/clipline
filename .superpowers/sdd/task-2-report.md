# Task 2 Report: Platform Facade And Capabilities

## What I implemented

- Added the Task 2 facade contract tests to `apps/clipline-app/tests/macos_shell_contract.rs`.
- Replaced the temporary flat platform placeholder with the planned module structure:
  - `apps/clipline-app/src/platform/mod.rs`
  - `apps/clipline-app/src/platform/types.rs`
  - `apps/clipline-app/src/platform/windows.rs`
  - `apps/clipline-app/src/platform/macos.rs`
- Added shared platform facade types:
  - `PlatformOs`
  - `CapabilityStatus`
  - `PermissionAction`
  - `PlatformCapabilities`
  - `DisplayInfo`
  - `AudioDeviceInfo`
  - `AudioDeviceLists`
  - `CapturableWindow`
- Implemented the Windows facade by adapting existing `clipline_capture::windows` APIs into the shared facade shape while preserving Windows behavior.
- Implemented the macOS facade with honest Milestone 1 capability stubs and unavailable/permission-gated responses where real platform APIs are not implemented yet.
- Switched `games.rs` and `game_plugins.rs` to use `crate::platform::CapturableWindow` instead of importing `clipline_capture::windows::CapturableWindow` directly.
- Routed game-window enumeration in `games.rs` through `crate::platform::enumerate_capturable_windows()`.

## What I tested and test results

- Ran the focused Task 2 command:
  - `cargo test -p clipline-app --test macos_shell_contract`
- Result after implementation:
  - PASS
  - `5 passed; 0 failed`

## TDD Evidence

### RED

- Command:
  - `cargo test -p clipline-app --test macos_shell_contract`
- Output summary:
  - `platform_facade_exposes_macos_capability_model` failed because `src/platform/types.rs` did not exist.
  - `game_detection_uses_platform_window_type` failed because `src/games.rs` did not yet contain `use crate::platform::CapturableWindow;`.
- Why it failed as expected:
  - The facade module/files had not been created yet.
  - Game detection still imported the Windows capture window type directly.

### GREEN

- Command:
  - `cargo test -p clipline-app --test macos_shell_contract`
- Output summary:
  - All 5 tests passed.
  - The crate compiled successfully on this macOS host.
  - The run emitted warnings about currently unused facade types/functions on macOS, but no errors.

## Files changed

- `apps/clipline-app/src/game_plugins.rs`
- `apps/clipline-app/src/games.rs`
- `apps/clipline-app/src/platform/mod.rs`
- `apps/clipline-app/src/platform/types.rs`
- `apps/clipline-app/src/platform/windows.rs`
- `apps/clipline-app/src/platform/macos.rs`
- `apps/clipline-app/src/platform.rs` (removed)
- `apps/clipline-app/tests/macos_shell_contract.rs`
- `.superpowers/sdd/task-2-report.md`

## Self-review findings

- The implementation matches the brief’s requested module layout and platform type surface.
- Windows capture behavior remains adapted from the existing `clipline_capture::windows` APIs rather than reduced.
- macOS behavior is explicit about what is unavailable or permission-gated in Milestone 1 and does not pretend capture/audio support exists yet.
- The current macOS shell still has unused facade warnings because this task introduces the facade boundary before the rest of the app fully consumes all new capability/display/audio helpers.
- I did not touch unrelated modified files already present in the working tree.

## Issues or concerns

- The focused verification command passes, but the new facade currently produces compile-time warnings for unused exports/types/functions on macOS.
- Task 1 macOS stub modules such as `games_macos.rs` and `game_plugins_macos.rs` remain in place; this task established the facade and rerouted the shared detection code, but did not widen macOS module wiring beyond the brief.

## Warning Fix (Task 2 noise pass)

- What changed:
  - Routed app commands for displays, audio devices, and memory through the platform facade in `apps/clipline-app/src/app.rs` (`list_displays`, `list_audio_devices`, `memory_status`).
  - Added a new `platform_capabilities` command so `crate::platform::capabilities()` is now consumed by the shell command surface.
  - Updated `apps/clipline-app/src/games_macos.rs` to consume `platform::enumerate_capturable_windows()` so the macOS game-window listing is wired to the facade.
  - Kept the macOS facade behavior stubbed while reusing the shared memory shim in `apps/clipline-app/src/platform/macos.rs`.
  - Added narrow, documented staging suppressions for intentionally unused staged symbols:
    - `AudioDeviceInfo` re-export path in `apps/clipline-app/src/platform/mod.rs`
    - `PlatformOs::Windows`
    - `PermissionAction::OpenAccessibilitySettings`
- Test command and warning status:
  - `cargo test -p clipline-app --test macos_shell_contract` → PASS, 5 passed, **0 warnings**.
- Files changed:
  - `apps/clipline-app/src/app.rs`
  - `apps/clipline-app/src/games_macos.rs`
  - `apps/clipline-app/src/platform/macos.rs`
  - `apps/clipline-app/src/platform/mod.rs`
  - `apps/clipline-app/src/platform/types.rs`
- Commit created:
  - `c0d2728` — fix(app): quiet staged platform facade warnings
- Concerns:
  - None beyond the already-staged facade intentionally limited to stubbed macOS behavior.
