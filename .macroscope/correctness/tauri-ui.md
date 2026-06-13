---
include:
  - "apps/clipline-app/src/**"
  - "apps/clipline-app/ui/**"
  - "apps/clipline-app/tests/**"
---

Review Tauri app and UI changes for:

- Service-thread state races, stale capture status, hotkey rebinding bugs, tray behavior, settings persistence, and validated file/folder paths.
- Non-Windows build stubs and `cfg` gates that keep Ubuntu CI compiling.
- The review player contract described in `handoff.md`: no native video controls, stable DOM IDs, keyboard behavior, hidden stacked views, WebView2 media-scope assumptions, and responsive layout without overlapping text or controls.
- Pure player logic belongs in `ui/player-core.js` with Rust Boa tests in `apps/clipline-app/tests/player_core.rs`.
