# Settings Page (Milestone 16) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Settings stops being a sidebar fold and becomes its own page in the main pane (user
request 2026-06-12). **Exit criterion:** gear (rail), a Settings entry (full sidebar), and a
close control navigate a dedicated settings view; the review state survives the round-trip;
all settings behavior (validate/save/restart service) is unchanged. Tests, clippy, user check,
push, CI green.

## Design

- **View model:** the main pane shows exactly one of `#review-empty`, `#review-viewer`,
  `#settings-page`. A `settingsOpen` flag plus the existing `currentClip` drive an
  `updateViews()`; opening settings pauses playback but does **not** close the clip — leaving
  settings returns to the paused player exactly as it was.
- **Navigation in:** `#open-settings` (full-sidebar bottom row: gear + label, replaces the
  `<details>` fold) and `#rail-settings` (now navigates instead of expanding the sidebar).
  Both toggle.
- **Navigation out:** `#settings-close` (✕ icon in the page header), `Esc` (takes priority
  over closing the clip; player shortcuts are inert while the page is open), or opening any
  clip from the library.
- **Page layout:** roomier card (max ~640 px) with grouped sections — Capture (target, window
  title), Recording (buffer, replay, bitrate, fps), Storage (quota), Hotkey — same field ids,
  same `fillSettings`/`readSettings`/save wiring, same service-restart semantics.

### Task 1: failing checks first

- [ ] `ui_contract.rs`: required ids gain `settings-page`, `open-settings`, `settings-close`;
  `open-settings` joins the SVG-icon list; `settings-fold` must be gone; settings field ids
  (`set-capture` … `settings-save`) become required (they were never in the contract — they
  are now load-bearing for the page).

### Task 2: implement

- [ ] `index.html`: settings page section in `<main>`; sidebar fold → bottom Settings button.
- [ ] `styles.css`: page layout, sections, sidebar bottom row; drop fold styles.
- [ ] `main.js`: `settingsOpen` + `updateViews()`, pause-on-enter, Esc priority and player
  shortcut suppression, nav wiring (gear, sidebar row, close, open-clip exits).
- [ ] All checks green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`; clean clippy; launch; user checklist; handoff; commit; push;
  CI green.

## Out of scope

- New settings; settings search; per-section save. Behavior parity only.
- Timeline zoom (still queued).
