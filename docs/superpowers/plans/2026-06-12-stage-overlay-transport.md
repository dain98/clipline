# Stage Overlay Transport (Milestone 14) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Replace the text-button transport row with an icon-based control bar overlaid on the
video (user feedback 2026-06-12: "raw play button and +5/-5 look horrible… mute button with
volume slider is just… 2002"). **Exit criterion:** the transport lives on the stage as a
translucent hover overlay with SVG icons, fades while playing and idle, stays visible when
paused; the deck below keeps timeline + ruler + export row. Tests, clippy, user-driven live
check, push, CI green.

## Design

- **Overlay**: bottom-anchored bar inside `.stage` (`id="stage-overlay"`), gradient scrim
  (transparent → dark), YouTube-grammar visibility:
  - always visible while **paused**;
  - while **playing**, visible on pointer activity over the stage, fades out after 2 s idle
    or when the pointer leaves the stage.
  - Pure policy `overlayVisible(paused, idleMs)` + `OVERLAY_HIDE_MS` in `player-core.js`
    (Boa-tested); the wiring evaluates it from the existing playhead rAF loop and pointer
    events.
- **Icons**: hand-authored inline SVG (24×24, `currentColor`, no icon font, no npm):
  play/pause (swapped via a `.playing` class), replay-5 / forward-5 (arc arrow + "5"),
  prev/next marker (bar + triangle), volume / muted (speaker, speaker+X via `.muted` class).
  Buttons become borderless icon buttons with a soft hover wash.
- **Volume**: icon button + slim slider that expands on hover/focus of the volume group
  (width 0 → 80 px) — no permanently parked slider.
- **Rate select** stays but restyled translucent to blend with the bar.
- **Deck** keeps timeline, ruler, export row; `stage-note` moves to the top-left corner of
  the stage so the bar doesn't cover it.
- All element ids survive (`play-toggle`, `seek-back`, …) — keyboard shortcuts, `paintTimeline`
  and the contract stay intact; play/mute state moves from `textContent` to classes.

### Task 1: failing tests first

- [ ] `player_core.rs`: `overlayVisible` truth table (paused → always; playing fresh activity
  → visible; playing past `OVERLAY_HIDE_MS` → hidden); constant exposed.
- [ ] `ui_contract.rs`: `id="stage-overlay"` required; each transport button
  (`play-toggle`, `seek-back`, `seek-forward`, `prev-marker`, `next-marker`, `mute-toggle`)
  must contain an `<svg` before its `</button>` — icons are the contract, text labels are a
  regression.

### Task 2: implement

- [ ] `player-core.js`: `overlayVisible`, `OVERLAY_HIDE_MS`.
- [ ] `index.html`: transport moves into `.stage` under `#stage-overlay`; SVG icons inline.
- [ ] `styles.css`: scrim, icon buttons, fade transition (`opacity` + `pointer-events: none`
  when hidden), expanding volume slider, translucent rate select, stage-note to top-left.
- [ ] `main.js`: activity tracking (pointer events on the stage set a timestamp; leave forces
  idle), visibility evaluated in the rAF loop and on play/pause; `syncPlayState`/`syncVolume`
  toggle classes instead of text.
- [ ] All Task-1 tests green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`; clean clippy.
- [ ] Launch the app, hand the user a checklist (icon rendering, fade in/out timing, paused
  pinning, volume hover expand, no dead zones for the video click-to-toggle).
- [ ] Update `handoff.md`.
- [ ] Commit, push, CI green.

## Out of scope

- Fullscreen/theater beyond the existing focus mode; settings gear menu.
- Replacing header/export buttons with icons (text is right there).
- Timeline zoom (still the next milestone after this).
