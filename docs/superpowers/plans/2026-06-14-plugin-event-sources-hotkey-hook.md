# Plugin Event Sources + In-Game Hotkey Hook

## Goal

Put game-specific marker/event ingestion behind the same built-in game plugin boundary as
game detection, and make Save Replay work while League of Legends has focus.

## Scope

- Extend the built-in game plugin registry with optional event-source spawners.
- Move League Live Client marker startup behind the League plugin.
- Pass the active built-in plugin id into `ServiceOptions`; custom games keep recording without
  marker ingestion.
- Add a Windows low-level keyboard hook fallback for the configured Save Replay hotkey.
- Keep the existing Tauri global shortcut registration as the normal OS path.
- Debounce save requests centrally so the global shortcut and hook cannot double-save.

## Verification

- `cargo fmt`
- `cargo test -p clipline-app`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
