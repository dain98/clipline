# Simple Trim Timeline Design

## Goal

Make the review timeline feel like a lightweight clipping tool by default, while keeping the current zoomable navigator editor behind a legacy setting.

## User Experience

The default review deck opens in a browse mode:

- The main timeline spans the whole clip.
- The playhead and event markers stay visible.
- Advanced navigator controls, snap toggle, and zoom buttons are hidden.
- The primary action is a scissors button that enters trim mode.

When the user presses the trim button, Clipline enters a local trim mode:

- The visible timeline frames a short window around the current playhead.
- Clip markers inside that local window remain visible.
- In/out handles and the selected range appear.
- The primary export action reads as `Create Clip` and uses the existing trim/export backend.
- Pressing the trim button again exits trim mode and returns to the whole-clip browse timeline.

The current editor remains available as `Legacy timeline editor` in General settings. Enabling it restores the existing navigator, zoom controls, snapping toggle, always-visible trim handles, and current shortcut behavior.

## Architecture

This is a UI-mode change, not a media-pipeline change. `player-core.js` keeps the pure trim math and gains one helper for choosing the default local trim range around a playhead. `main.js` owns the mode state, toggles deck classes, and continues to call the existing `export_clip` command with `trimStart`/`trimEnd`. `AppSettings` gains one persisted boolean, loaded through the existing manual `load_from_object` path.

## Data Flow

`get_settings` returns `legacy_timeline_editor`. `fillSettings` copies it to `currentSettings`, checks the General settings checkbox, and calls `applyTimelineEditorPreference`. `readSettings` persists the checkbox value. The deck reads the setting only to choose between legacy and simple timeline presentation; clip loading, marker rendering, seeking, trimming, and export keep their existing data paths.

## Testing

Tests cover three layers:

- Settings defaults and legacy loading in `apps/clipline-app/src/settings/tests.rs`.
- Pure trim-window math in `apps/clipline-app/tests/player_core.rs`.
- DOM/JS/CSS wiring in `apps/clipline-app/tests/ui_contract.rs`.

Manual verification should open the app, review a clip, confirm the default timeline is simple, press the scissors button to enter local trim mode, export a clip, then enable the legacy setting and confirm the old navigator/zoom/snap editor returns.
