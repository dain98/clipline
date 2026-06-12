# Recording Controls Cleanup Plan

## Goal

Make recording controls match the way users think about the app. **Exit criterion:** Settings exposes
only the save length users care about, capped at two minutes; Smoothness includes 30/60/90/120 FPS;
the Settings page has no redundant close button; and the sidebar shows capture target, storage, clip
count, and a real recording stop/start control.

## Scope

- Recording settings UI and validation ranges.
- Sidebar status presentation.
- Minimal Tauri command wiring to stop/start the recorder from the sidebar.
- No changes to capture pipelines, saved clip format, or library behavior.

## Tests

- [ ] `player_core.rs`: Smoothness presets include 90 FPS.
- [ ] `player_core.rs`: capture source labels are user-readable.
- [ ] `ui_contract.rs`: Save length slider is capped at 120 seconds and has no 5-minute preset.
- [ ] `settings.rs`: legacy replay windows over two minutes clamp on load.

## Implementation Steps

- [ ] Remove the user-facing Replay history control and keep the internal buffer at two minutes.
- [ ] Cap Save length at 120 seconds and update slider markers/presets.
- [ ] Add the 90 FPS Smoothness stop.
- [ ] Remove the Settings X button.
- [ ] Replace sidebar diagnostics with capture target, storage, clips, and Save Replay.
- [ ] Add `set_recording` so the sidebar dot/status can stop and start recording.
- [ ] Run tests/clippy, visual-check Settings/sidebar, then reopen the app for manual testing.
