# League Kill Feed Event Rail Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render League match events in the right rail as a compact kill-feed-style list with champion portraits, event icons, and friendly/enemy visual treatment.

**Architecture:** Keep the event rail plugin-driven through `presentation.event_rail`, but compute the per-row layout in pure `player-core.js` so it can be tested without DOM or Tauri. The League adapter enriches `PlayerSummary` with optional participant and team data from Live Client `/playerlist`; old sidecars keep deserializing through serde defaults. `main.js` renders the pure row view model, using Data Dragon for champion portraits and manifest-provided marker icons for event symbols.

**Tech Stack:** Rust serde structs and Live Client parsing, vanilla JavaScript, Boa-backed `player-core.js` tests, `ui_contract.rs`, manifest JSON in `apps/clipline-app/plugin-seeds` and `C:\Users\dain\Projects\clipline-plugin-league-of-legends`.

---

### Task 1: Preserve Participant And Team Data

**Files:**
- Modify: `crates/clipline-events/src/markers.rs`
- Modify: `crates/clipline-events/src/lib.rs`
- Modify: `crates/clipline-lol/src/raw.rs`
- Modify: `crates/clipline-lol/src/client.rs`
- Modify: existing Rust tests with `PlayerSummary` literals

- [ ] **Step 1: Write failing serde and Live Client tests**

Add tests that prove:

```rust
// Old sidecars without participant/team fields still deserialize.
let old = r#"{"champion_name":"Nautilus","kills":3,"deaths":4,"assists":23}"#;
let summary: PlayerSummary = serde_json::from_str(old).unwrap();
assert!(summary.participants.is_empty());
assert!(summary.team.is_empty());
```

```rust
// Live Client playerlist team fields are retained.
let json = r#"[{
  "summonerName": "dain",
  "riotId": "Dain#NA1",
  "championName": "Nautilus",
  "team": "ORDER",
  "scores": { "kills": 3, "deaths": 4, "assists": 23 }
}]"#;
let players: Vec<PlayerListEntry> = serde_json::from_str(json).unwrap();
assert_eq!(players[0].team, "ORDER");
```

```rust
// The local summary carries every participant for later UI name->champion lookup.
let summary = player_summary_from_list(&players, "dain#NA1").unwrap();
assert_eq!(summary.player_name, "dain");
assert_eq!(summary.team, "ORDER");
assert_eq!(summary.participants.len(), players.len());
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-events player_summary_defaults_missing_participant_data -- --nocapture
cargo test -p clipline-lol parses_player_list_entries_for_summary -- --nocapture
cargo test -p clipline-lol player_summary_carries_participants_and_team -- --nocapture
```

Expected: the new participant/team fields do not exist yet.

- [ ] **Step 3: Implement minimal data model**

Add `PlayerParticipant { player_name, champion_name, team }` and optional serde-defaulted `player_name`, `team`, and `participants` fields to `PlayerSummary`. Parse `PlayerListEntry.team`, export `PlayerParticipant`, and update `player_summary_from_list` to populate participants with non-empty player and champion names.

- [ ] **Step 4: Run tests to verify they pass**

Run the same three focused commands. Expected: PASS.

### Task 2: Pure Kill Feed Row View Model

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/player-core.js`

- [ ] **Step 1: Write failing pure JS tests**

Add tests for `PlayerCore.gameEventRailItem(marker, summary, presentation, options)`:

```javascript
PlayerCore.gameEventRailItem(
  { kind: 'ChampionKill', actor: 'dain', victim: 'Soupmaster', t_s: 162 },
  {
    player_name: 'dain',
    team: 'ORDER',
    participants: [
      { player_name: 'dain', champion_name: 'Nautilus', team: 'ORDER' },
      { player_name: 'Soupmaster', champion_name: 'Ahri', team: 'CHAOS' }
    ]
  },
  {
    marker_kinds: {
      ChampionKill: { category: 'kill', icon: 'data:image/png;base64,kill' }
    }
  },
  { data_dragon: { version: '16.13.1' } }
)
```

Expected JSON includes:

```json
{
  "layout": "duel",
  "allegiance": "friendly",
  "icon": "data:image/png;base64,kill",
  "actor": {
    "name": "dain",
    "champion": "Nautilus",
    "asset": "https://ddragon.leagueoflegends.com/cdn/16.13.1/img/champion/Nautilus.png"
  },
  "victim": {
    "name": "Soupmaster",
    "champion": "Ahri",
    "asset": "https://ddragon.leagueoflegends.com/cdn/16.13.1/img/champion/Ahri.png"
  }
}
```

Add separate assertions for `ChampionDeath` returning `allegiance: "enemy"` and for missing participants returning a text fallback with no portraits.

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app game_event_rail_item -- --nocapture
```

Expected: FAIL because `gameEventRailItem` is not exported.

- [ ] **Step 3: Implement minimal pure helper support**

Use existing marker category and Data Dragon helpers. Match participants by normalized player name, build actor/victim slots for `ChampionKill` and `ChampionDeath`, pass through manifest marker icons, and return neutral text rows for non-duel events or unknown participants.

- [ ] **Step 4: Run test to verify it passes**

Run the same command. Expected: PASS.

### Task 3: DOM Rendering And Styling

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI contract assertions**

Extend the review UI contract so it guards:

```rust
main_js().contains("gameEventRailItem")
main_js().contains("game-event-duel")
main_js().contains("game-event-portrait")
main_js().contains("game-event-kind-icon")
styles_css().contains(".game-event-row-friendly")
styles_css().contains(".game-event-row-enemy")
styles_css().contains(".game-event-name")
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: FAIL because the rail still renders one text label per event.

- [ ] **Step 3: Implement DOM rendering**

Import `gameEventRailItem` from `player-core.js`. In `renderGameEventRail`, compute a view model per marker using `clip.markers.player_summary`, `presentation`, and `presentation.data_dragon`. Render duel rows as time, actor portrait/name, event icon, victim portrait/name. Render fallback rows as time plus text label. Keep the existing click/selection behavior and `data-game-event-index` attributes.

- [ ] **Step 4: Implement stable CSS**

Use fixed-size portrait and icon boxes, one-line clipped names, no nested cards, and blue/red/neutral accent classes:

```css
.game-event-row-friendly { border-color: rgba(59, 130, 246, 0.45); }
.game-event-row-enemy { border-color: rgba(248, 113, 113, 0.45); }
.game-event-duel { grid-template-columns: 38px minmax(0, 1fr) 28px minmax(0, 1fr); }
```

- [ ] **Step 5: Run UI contract to verify it passes**

Run the same UI contract command. Expected: PASS.

### Task 4: Manifest Hook And Package Refresh

**Files:**
- Modify: `apps/clipline-app/plugin-seeds/league_of_legends/clipline-plugin.json`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `C:\Users\dain\Projects\clipline-plugin-league-of-legends\package\clipline-plugin.json`
- Modify: `handoff.md`

- [ ] **Step 1: Write failing manifest assertion**

Extend the League seed manifest test so it asserts:

```rust
presentation["event_rail"]["layout"] == "kill_feed"
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app league_seed_manifest_declares_data_dragon_portrait_provider -- --nocapture
```

Expected: FAIL because `event_rail.layout` is missing.

- [ ] **Step 3: Update manifests and package version**

Set `presentation.event_rail.layout` to `"kill_feed"` in both bundled and standalone package manifests. Bump the standalone package to `1.3.2`, rebuild `dist/clipline-plugin-league-of-legends-1.3.2.zip`, update the known first-party release URL/hash/version, and update `handoff.md`.

- [ ] **Step 4: Run package tests**

Run:

```powershell
cargo test -p clipline-app game_plugins::tests:: -- --nocapture
```

Expected: PASS.

### Task 5: Verification

**Files:**
- All changed files.

- [ ] **Step 1: Format and syntax check**

Run:

```powershell
cargo fmt --all
node --check apps/clipline-app/ui/player-core.js
node --check apps/clipline-app/ui/main.js
```

Expected: all exit 0.

- [ ] **Step 2: Run focused tests**

Run:

```powershell
cargo test -p clipline-events player_summary -- --nocapture
cargo test -p clipline-lol player_summary -- --nocapture
cargo test -p clipline-app game_event_rail_item -- --nocapture
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: PASS.

- [ ] **Step 3: Run full gates**

Run:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Launch app**

Stop any running `clipline-app.exe`, then run:

```powershell
cargo run -p clipline-app
```

Expected: the review rail shows champion-vs-champion rows for League kills/deaths when participant data exists, falls back cleanly for old clips, and still scrolls/syncs/collapses as before.
