# Settings Recording UX Plan

## Goal

Make Settings easier to read for non-technical users. **Exit criterion:** the display-region map
does not show internal scrollbars during window resize, and the Recording tab uses plain-language
controls for saved clip length, quality, and smoothness while persisting the same settings values.

## Scope

- Capture region map presentation only; region math and recorder behavior stay unchanged.
- Recording tab copy and controls only.
- Preserve existing setting ids (`set-buffer`, `set-replay`, `set-bitrate`, `set-fps`) so the
  save path stays stable.

## Tests

- [ ] `player_core.rs`: display map height grows from virtual desktop aspect ratio without using
  scrollable content.
- [ ] `player_core.rs`: recording duration labels use minutes/seconds instead of raw technical
  seconds where possible.
- [ ] `player_core.rs`: bitrate values map to plain quality preset stops.
- [ ] `player_core.rs`: smoothness slider stops map to valid FPS values.
- [ ] `ui_contract.rs`: Recording tab exposes summary/status ids, slider scales, and replay preset
  buttons.

## Implementation Steps

- [ ] Add pure helpers for map height, duration labels, and quality labels.
- [ ] Make `#display-map` overflow hidden and set its height from the monitor layout.
- [ ] Replace Recording tab copy with plain language.
- [ ] Remove Replay history from the user-facing controls; keep buffer sizing internal.
- [ ] Change bitrate from a technical number field to a quality slider with summary text.
- [ ] Add visible scale indicators under Recording sliders.
- [ ] Change smoothness from a dropdown to a slider with summary text.
- [ ] Add quick presets for Save Replay length.
- [ ] Wire summaries and replay presets in `main.js`.
- [ ] Run tests/clippy, visual-check Settings, then reopen the app for manual testing.
