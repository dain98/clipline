# Final Review Fix Report

## Findings fixed

- Critical 1: macOS recording start now checks `service::ensure_recording_available()` before it can reuse or spawn the macOS stub service. The macOS service returns an unavailable Milestone 1 error, while Windows remains available.
- Critical 2: macOS cloud connect now fails before `connect_with_device_token` with a clear Keychain-not-implemented unavailable error.
- Important 3: Removed unreferenced stand-in modules `cloud_macos.rs`, `library_macos.rs`, and `game_icon_macos.rs`; shared modules remain wired through `main.rs`.
- Important 4: macOS capabilities for unimplemented capture/audio/hotkey fallback features now report unavailable instead of permission-action statuses.
- Minor 5: Startup registration save error copy is now platform-neutral.
- Minor 6: Expanded `macos_shell_contract` coverage for recording/cloud unavailable behavior and tracked `*_macos.rs` module wiring.

## Tests/checks run

- `cargo test -p clipline-app` - exit 0; 108 app unit tests, 13 macOS shell contract tests, 50 player-core tests, and 13 UI contract tests passed.
- `cargo test --workspace` - exit 0; workspace unit, integration, and doc tests passed.
- `cargo clippy -p clipline-app --all-targets -- -D warnings` - exit 0; no warnings.
- `cargo run -p clipline-app` smoke - exit 0; app launched, logged the expected macOS focused-game hotkey fallback warning, and was terminated after the smoke check.
- `git diff --check` - exit 0; no whitespace errors.

## Files changed

- `apps/clipline-app/src/app.rs`
- `apps/clipline-app/src/cloud.rs`
- `apps/clipline-app/src/platform/macos.rs`
- `apps/clipline-app/src/platform/mod.rs`
- `apps/clipline-app/src/platform/types.rs`
- `apps/clipline-app/src/service.rs`
- `apps/clipline-app/src/service_macos.rs`
- `apps/clipline-app/tests/macos_shell_contract.rs`
- Removed `apps/clipline-app/src/cloud_macos.rs`
- Removed `apps/clipline-app/src/library_macos.rs`
- Removed `apps/clipline-app/src/game_icon_macos.rs`

## Commit created

- `fix(app): make macos stubs explicit`

## Concerns

- None.
