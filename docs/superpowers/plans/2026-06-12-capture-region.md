# Capture Region Selection Implementation Plan

> Follow the repo's plan-driven TDD convention. Execute task-by-task, write failing tests
> before implementation, and leave checkboxes unticked.

**Goal:** Add a settings-side display region capture mode: users can choose a monitor,
enter or drag a rectangular area, align the rectangle within a display, and save that
area as the recorder input. **Exit criterion:** `display_region` persists in settings,
the Capture settings tab renders a monitor map with x/y/width/height fields and a
right-click align/display menu, the service captures the selected monitor and GPU-crops
the chosen rectangle before encoding, tests/clippy are clean, and handoff is updated.

## Design

**Scope.** Region capture is one selected display plus one rectangle inside it. Cross-monitor
composition is out of scope for this milestone because WGC captures one monitor/window per
session today; stitching multiple monitor textures would need a compositor.

**Settings model.**

- Add `CaptureMode::DisplayRegion` serialized as `display_region`.
- Add `CaptureRegionSettings { display_id, x, y, width, height }`, defaulted for legacy
  settings files.
- `AppSettings::validate` checks positive finite dimensions and requires a useful rectangle
  only when `display_region` is active.
- `ServiceOptions` carries a `CaptureSource` enum instead of only `window_title`.

**Windows display plumbing.**

- Add `clipline-capture::windows::display`: enumerate monitors with virtual-desktop pixel
  coordinates and stable Win32 device ids (`\\.\DISPLAY1`, etc.).
- Add `WgcCapture::for_monitor_on` so the service can capture a selected monitor, not only
  the primary monitor at `(0, 0)`.

**GPU crop.**

- Extend `VideoConverter` with an optional source rect. The video processor sets
  `VideoProcessorSetStreamSourceRect`; output remains NV12 at the encoder dimensions.
- `MftH264Encoder::new_with_crop` accepts a capture-frame crop rect. The service derives it
  by subtracting the selected monitor's virtual origin from the saved region rectangle.
- Encode dimensions are based on the crop rectangle, still capped to 2560 wide and
  even-rounded.

**Settings UI.**

- Capture tab gets a third target option: `display region`.
- The region editor renders all displays in a scaled virtual-desktop map, a draggable capture
  rectangle, width/height/x/y numeric fields, and a current display label.
- Right-clicking the map opens an in-app context menu with:
  - `Align`: Left, Right, Top, Bottom, Center.
  - `Set to Display`: one item per enumerated display.
- Region math lives in `player-core.js` so it is Boa-tested without a browser.

## Task 1: failing tests first

- [ ] `settings.rs`: legacy settings still load; display-region settings round-trip;
  validation rejects zero/too-small region dimensions; service options carry region data.
- [ ] `player_core.rs`: display union/layout math; set-region-to-display; align left/right/top/bottom/center; clamping after drag/field edits.
- [ ] `ui_contract.rs`: capture-region DOM ids and `display_region` option are present.
- [ ] `nv12.rs`: converter can be constructed with a crop rect and still produces the requested output size on capable hardware.

## Task 2: implement

- [ ] Add settings/service data model and validation.
- [ ] Add Win32 monitor enumeration and WGC monitor selection.
- [ ] Add source-rect crop support to the GPU converter and encoder constructor.
- [ ] Wire service startup for display-region capture.
- [ ] Build the settings UI map, fields, drag/resize, and context menu.
- [ ] Add `list_displays` Tauri command.

## Task 3: gates and handoff

- [ ] `cargo test --workspace`.
- [ ] `cargo clippy --workspace --all-targets`.
- [ ] Launch app; verify display-region settings UI, save/restart wiring, and a short region recording on the dev machine.
- [ ] Update `handoff.md`.

## Out of scope

- Cross-monitor stitched capture.
- A dedicated full-screen region picker overlay.
- Persisting per-display friendly EDID names beyond Win32 device ids.
