# Plugin Review Layout Implementation Plan

> Superseded by `docs/superpowers/plans/2026-06-29-first-party-supported-games.md`.
> Clipline is no longer pursuing installable game presentation plugins in this branch.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move plugin timeline events into a playback-synced right rail and make the bottom game metadata area declarative through plugin presentation config.

**Architecture:** Keep plugin manifests declarative and no-JS. `player-core.js` owns pure metadata formatting helpers, while `main.js` maps manifest presentation config to DOM. The existing `ui_contract` test continues guarding DOM shape and plugin-driven behavior.

**Tech Stack:** Rust/Tauri manifest seed, vanilla HTML/CSS/JS, Boa-backed `tests/player_core.rs`, `tests/ui_contract.rs`.

---

### Task 1: Guard Plugin Review Layout

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `apps/clipline-app/tests/player_core.rs`

- [ ] **Step 1: Add failing UI contract assertions**

Assert that the review player exposes a right-side `game-event-rail` with event list ids, a bottom `game-metadata-panel`, and plugin-driven functions for `renderGameEventRail`, `syncGameEventRail`, and `renderGameMetadataPanel`.

- [ ] **Step 2: Add failing pure metadata formatter test**

Add a Boa test that calls a new `PlayerCore.playerSummaryFields(summary, fields)` helper and expects declarative fields like champion and KDA to format from existing `player_summary` data.

- [ ] **Step 3: Run targeted tests and verify RED**

Run `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture` and `cargo test -p clipline-app player_summary_fields -- --nocapture`; expect failures for missing ids/functions/helper.

### Task 2: Implement Declarative Bottom Metadata

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `apps/clipline-app/plugin-seeds/league_of_legends/clipline-plugin.json`

- [ ] **Step 1: Add `PlayerCore.playerSummaryFields`**

Support declarative field types `champion`, `kda`, and `stat`, reading only allowlisted `player_summary` paths. Missing values produce empty strings so old clips degrade cleanly.

- [ ] **Step 2: Replace bottom game panel with metadata panel**

Rename the bottom DOM to `game-metadata-panel`, render fields from `presentation.metadata_panel.fields`, and keep League configured for portrait/champion/KDA using current sidecar data.

- [ ] **Step 3: Run player-core test and verify GREEN**

Run `cargo test -p clipline-app player_summary_fields -- --nocapture`.

### Task 3: Implement Right Event Rail

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `apps/clipline-app/plugin-seeds/league_of_legends/clipline-plugin.json`

- [ ] **Step 1: Move event panel into right rail DOM**

Add `game-event-rail`, `game-event-rail-title`, `game-event-rail-summary`, and `game-event-list` beside the stage.

- [ ] **Step 2: Render all markers and sync active event**

`renderGameEventRail` renders every marker, each row seeks on click, and `syncGameEventRail` highlights the latest marker at or before current playback time and scrolls it into view while playing.

- [ ] **Step 3: Run UI contract and verify GREEN**

Run `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture` and `cargo test -p clipline-app --test ui_contract ui_is_split_into_markup_styles_and_logic -- --nocapture`.

### Task 4: Verify

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Update handoff**

Add a short note that plugin presentation now has a right event rail and declarative bottom metadata strip.

- [ ] **Step 2: Run app tests**

Run `cargo test -p clipline-app`.

- [ ] **Step 3: Run workspace tests**

Run `cargo test --workspace`.

- [ ] **Step 4: Run clippy**

Run `cargo clippy --workspace --all-targets -- -D warnings`.

- [ ] **Step 5: Relaunch app**

Stop existing `clipline-app.exe`, then run `cargo run -p clipline-app` for manual inspection.
