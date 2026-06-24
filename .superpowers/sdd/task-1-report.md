# Task 1 Report: Native macOS File Actions And Platform Copy

## What changed
- `reveal_clip` now reveals the selected clip itself instead of opening only the parent folder.
- macOS file reveal uses Finder-native `open -R`.
- macOS clipboard copy now uses `osascript` to place a Finder file object on the pasteboard.
- Added `escape_applescript_string` and a macOS unit test for quote/backslash escaping.
- Marked macOS file clipboard capability as available.
- Swapped the startup settings copy from hard-coded Windows text to runtime platform-aware copy in the settings UI.
- Added contract coverage for the native macOS file actions and the platform-specific startup copy.

## TDD evidence

### RED
1. `cargo test -p clipline-app --test macos_shell_contract macos_file_actions_are_native_and_available`
   - Failed as expected:
   - `reveal_clip should reveal the selected clip, not only open its parent folder`

2. `cargo test -p clipline-app library::tests::applescript_string_escapes_quotes_and_backslashes`
   - Failed as expected at compile time:
   - `cannot find function escape_applescript_string in this scope`

3. `cargo test -p clipline-app --test ui_contract general_settings_copy_is_platform_aware`
   - First attempt was a test-placement mistake and ran `0 tests`.
   - I split the assertions into a standalone test and reran; the standalone test then failed as expected:
   - `startup setting text should have ids for platform-specific copy`

### GREEN
1. `cargo test -p clipline-app --test macos_shell_contract macos_file_actions_are_native_and_available`
   - Passed.
2. `cargo test -p clipline-app library::tests::applescript_string_escapes_quotes_and_backslashes`
   - Passed.
3. `cargo test -p clipline-app --test ui_contract general_settings_copy_is_platform_aware`
   - Passed.
4. Broader verification:
   - `cargo test -p clipline-app --test macos_shell_contract`
   - `cargo test -p clipline-app --test ui_contract`
   - Both passed.

## Files changed
- `apps/clipline-app/src/library.rs`
- `apps/clipline-app/src/platform/macos.rs`
- `apps/clipline-app/tests/macos_shell_contract.rs`
- `apps/clipline-app/tests/ui_contract.rs`
- `apps/clipline-app/ui/index.html`
- `apps/clipline-app/ui/main.js`

## Self-review
- The macOS reveal and clipboard actions are native and isolated behind platform-specific helpers.
- Windows behavior remains intact: Windows still uses Explorer selection and CF_HDROP clipboard transfer.
- The UI copy now updates at runtime from `platform_capabilities` and still has a reasonable fallback if the capability call fails.
- The contract tests now cover the intended behavior directly instead of relying on incidental text.

## Concerns
- I did not run a full app launch/manual smoke test here; verification is focused on the contract and unit test surface.
- The startup copy is runtime-driven, so if platform capability fetching regresses the UI will fall back to Windows wording until fixed.
