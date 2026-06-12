# Sidebar Rail & Header Cleanup (Milestone 15) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests or repeatable checks before implementation, and leave checkboxes unticked.

**Goal:** Per user feedback (2026-06-12): the Focus button becomes a hamburger in the sidebar
that collapses it to an **icon-only rail** (not a full collapse); Open Folder becomes a folder
icon; Copy Path is removed; the Close button is removed in favor of clicking the active clip
row again. **Exit criterion:** rail toggles via hamburger and `F`, rail exposes
status/save/settings as icons, the review header is two controls (folder icon, Delete), and
re-clicking the open clip closes it. Tests, clippy, user check, push, CI green.

## Design

- **Sidebar structure:** a persistent top row (`#sidebar-toggle` hamburger + brand) above two
  alternative bodies — `.sidebar-full` (today's content) and `.sidebar-rail` (icon column).
  `.app.rail` switches the grid column to 52 px and swaps the bodies. The clip stays open;
  nothing else changes — the old `.app.focus` full-collapse dies.
- **Rail contents (top to bottom):** recording dot (`#rail-dot`, mirrors `#dot`),
  `#rail-save` (record icon → `save_replay`), spacer, `#rail-settings` (gear → expands the
  sidebar and opens the settings fold). All with tooltips.
- **Keyboard:** `F` keeps its `toggle-focus` intent, now wired to the rail toggle. `Esc`
  still closes the clip (and no longer touches sidebar state — rail collapse is a user
  preference that survives open/close).
- **Review header:** `#open-folder` becomes a folder-icon button; `#copy-path` and
  `#close-review` are deleted. The path remains visible in `#pmeta` (selectable text is the
  copy story now: make `#pmeta` `user-select: text`).
- **Close-by-toggle:** clicking the library row of the currently open clip calls
  `closeReview()`; any other row opens it. The active highlight already signals state.

### Task 1: failing checks first

- [ ] `ui_contract.rs`: required ids gain `sidebar-toggle`, `rail-save`, `rail-settings`,
  `rail-dot`; drop `copy-path`, `close-review`, `focus-toggle`; SVG requirement extends to
  `sidebar-toggle`, `open-folder`, `rail-save`, `rail-settings`; assert `id="copy-path"` and
  `id="close-review"` are gone (the cleanup is the contract).
- [ ] No `player-core.js` changes (the `toggle-focus` intent is reused), so no new Boa tests.

### Task 2: implement

- [ ] `index.html`: sidebar top row + rail column (hamburger/folder/record/gear SVGs); header
  loses Copy Path/Close, Open Folder becomes an icon.
- [ ] `styles.css`: `.app.rail` grid (52 px), body swap, rail icon column; drop `.app.focus`;
  `#pmeta { user-select: text }`.
- [ ] `main.js`: `toggleRail`, rail save/settings/dot wiring, row-click toggle, remove
  copy-path/close listeners (Esc path stays).
- [ ] All checks green.

### Task 3: gates and handoff

- [ ] `cargo test --workspace`; clean clippy; launch; user checklist; handoff; commit; push;
  CI green.

## Out of scope

- Persisting rail state across launches (session-only for now).
- Library access from rail mode (expand to browse — that's the point of the rail).
- Timeline zoom (still queued next).
