# League Event Rail Polish Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development and superpowers:systematic-debugging. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish the League match-events rail after real ARAM testing: friendly/enemy coloring must show, objective rows must include the relevant champion portrait plus event icon, and marker icons should use cleaner first-party-compatible CommunityDragon assets.

**Architecture:** Keep the row view model pure in `ui/player-core.js`, render DOM in `ui/main.js`, and keep League presentation assets behind the manifest paths already shipped in the bundled seed and standalone package. Existing installed manifests remain valid; package version bumps refresh the art.

**Sources:** CommunityDragon exposes League client assets through `raw.communitydragon.org`, including the match-history assets under `plugins/rcp-fe-lol-match-history/global/default/`.

---

### Task 1: Lock The Regressions In Tests

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a pure `gameEventRailItem` test for `TurretKilled` with an actor participant, expecting an actor/objective layout, actor portrait data, icon passthrough, and friendly/enemy allegiance from team data.
- [ ] Add UI contract assertions that objective rows have a stable DOM class and that friendly/enemy CSS selectors are specific to rail buttons so the accent backgrounds win over the base row rule.
- [ ] Run focused tests and confirm the new assertions fail before implementation.

### Task 2: Render Objective Rows

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`

- [ ] Extend `gameEventRailItem` to emit an actor-event layout for non-duel events that have an actor participant and an icon, such as turret, dragon, and baron events.
- [ ] Render actor-event rows as time, champion portrait/name, event icon, and compact objective text.
- [ ] Fix friendly/enemy/neutral row styling specificity and tune the event icon box so kill/death/tower art is centered at a useful size.
- [ ] Run focused player-core and UI contract tests until green.

### Task 3: Refresh Marker Art

**Files:**
- Modify binary marker assets in `apps/clipline-app/plugin-seeds/league_of_legends/assets/markers/`
- Modify binary marker assets in `C:\Users\dain\Projects\clipline-plugin-league-of-legends\package\assets\markers\`
- Modify: `apps/clipline-app/plugin-seeds/league_of_legends/clipline-plugin.json`
- Modify: `C:\Users\dain\Projects\clipline-plugin-league-of-legends\package\clipline-plugin.json`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `handoff.md`

- [ ] Replace marker PNGs with suitable CommunityDragon match-history assets where available while preserving manifest paths.
- [ ] Bump the bundled seed package version and standalone package version.
- [ ] Build a new standalone zip, compute the SHA-256, and update the known first-party package release.
- [ ] Update handoff notes with the new package version and asset source.

### Task 4: Verification And Runtime Check

**Files:**
- All changed files.

- [ ] Run `node --check` for changed JS.
- [ ] Run focused Rust/contract tests.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Relaunch `cargo run -p clipline-app` and ask the user to check the ARAM clip through Settings update/reset-to-seed.
