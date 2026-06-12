# Hotkey Recorder Plan

## Goal

Make Settings > Hotkeys record a shortcut from keyboard input instead of asking the user to type
the shortcut text manually. **Exit criterion:** clicking/focusing the Save Replay hotkey field
captures the next valid F-key or modifier-plus-F-key combo, displays it normalized, and saves
through the existing settings path.

## Scope

- Save Replay hotkey only.
- Use the Windows-friendly function-key lane: F1-F11 or F13-F24, with optional Ctrl/Alt/Shift.
- Reject F12 because Microsoft documents it as reserved for debuggers.
- Update backend parsing to match the recorder; keep registration/rebind plumbing unchanged.

## Non-goals

- Arbitrary letter/number shortcuts.
- Multiple configurable actions.
- Conflict detection beyond the existing Tauri registration failure.

## Tests

- [ ] `player_core.rs`: pure `hotkeyFromKeyEvent` formats `F10` and `Ctrl+Alt+F9`.
- [ ] `player_core.rs`: modifier-only keydown returns pending.
- [ ] `player_core.rs`: `Escape` cancels recording.
- [ ] `player_core.rs`: non-F-key combos and F12 return invalid.
- [ ] `ui_contract.rs`: Hotkeys section exposes a recorder status element and read-only input.

## Implementation Steps

- [ ] Add `hotkeyFromKeyEvent` to `ui/player-core.js`.
- [ ] Make `#set-hotkey` read-only and add `#hotkey-status`.
- [ ] Wire focus/click/keydown in `ui/main.js`:
  - focus/click enters recording state,
  - modifier-only updates status,
  - valid combo writes normalized shortcut and exits recording,
  - `Escape` exits recording without changing the value,
  - invalid combos stay recording and explain the accepted shape.
- [ ] Add focused styling for active/error/ready recorder states.
- [ ] Run app tests, workspace tests, clippy, and open the app for manual testing.
