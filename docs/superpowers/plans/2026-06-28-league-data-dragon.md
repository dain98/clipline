# League Data Dragon Portraits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Riot Data Dragon champion portrait support to the League plugin's declarative bottom metadata strip.

**Architecture:** Keep Data Dragon as declarative presentation data on the League package. `player-core.js` remains pure and resolves known Data Dragon champion square URLs from field config, while `main.js` only threads `presentation.data_dragon` into the existing metadata formatter and renders the returned `asset`.

**Tech Stack:** Vanilla JavaScript, Boa-backed Rust UI tests, manifest JSON in `apps/clipline-app/plugin-seeds`, standalone plugin package JSON in `C:\Users\dain\Projects\clipline-plugin-league-of-legends`.

---

### Task 1: Pure Data Dragon Field Formatting

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/player-core.js`

- [ ] **Step 1: Write the failing test**

Add a `player_summary_fields_resolve_data_dragon_portraits` test that calls:

```javascript
PlayerCore.playerSummaryFields(
  { champion_name: 'Nautilus', kills: 3, deaths: 4, assists: 23 },
  [{
    type: 'portrait',
    source: 'player_summary.champion_name',
    label: 'Champion',
    asset_provider: 'riot_data_dragon_champion_square',
    asset_key_format: 'data_dragon_champion',
    asset_aliases: { wukong: 'MonkeyKing' }
  }],
  { data_dragon: { version: '16.13.1' } }
)
```

Expected first field:

```json
{
  "type": "portrait",
  "label": "Champion",
  "value": "Nautilus",
  "assetKey": "Nautilus",
  "asset": "https://ddragon.leagueoflegends.com/cdn/16.13.1/img/champion/Nautilus.png"
}
```

Also assert that `champion_name: 'Wukong'` resolves `assetKey: "MonkeyKing"`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-app player_summary_fields_resolve_data_dragon_portraits -- --nocapture`

Expected: FAIL because `playerSummaryFields` does not accept the third options argument and does not resolve the Data Dragon provider.

- [ ] **Step 3: Implement minimal pure helper support**

Update `player-core.js` so `playerSummaryFields(summary, fields, options = {})`:

- Preserves existing `asset_template` behavior.
- Supports `asset_provider: "riot_data_dragon_champion_square"`.
- Resolves `options.data_dragon.version`.
- Normalizes alias keys by lowercasing and removing non-alphanumeric characters.
- Builds `https://ddragon.leagueoflegends.com/cdn/{version}/img/champion/{assetKey}.png`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-app player_summary_fields_resolve_data_dragon_portraits -- --nocapture`

Expected: PASS.

### Task 2: Thread Plugin Data Dragon Config

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing UI contract assertion**

Extend the plugin-driven review UI contract to assert that `main.js` passes `presentation.data_dragon` into `playerSummaryFields` and still contains no League-specific gallery branch.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture`

Expected: FAIL because `renderGameMetadataPanel` currently calls `playerSummaryFields(summary, metadataPanel.fields)`.

- [ ] **Step 3: Implement config threading**

Change `renderGameMetadataPanel` to call:

```javascript
const presentation = pluginPresentationForClip(clip);
const fields = metadataPanel && metadataPanel.fields
  ? playerSummaryFields(summary, metadataPanel.fields, {
      data_dragon: presentation && presentation.data_dragon,
    })
  : [];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture`

Expected: PASS.

### Task 3: League Package Manifest Config

**Files:**
- Modify: `apps/clipline-app/plugin-seeds/league_of_legends/clipline-plugin.json`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `C:\Users\dain\Projects\clipline-plugin-league-of-legends\package\clipline-plugin.json`
- Test: `apps/clipline-app/src/game_plugins.rs`

- [ ] **Step 1: Write the failing manifest test**

Add a test that `league_seed_manifest_declares_data_dragon_portrait_provider` parses `LEAGUE_SEED_MANIFEST_JSON` and asserts:

- `presentation.data_dragon.version == "16.13.1"`
- first `metadata_panel.fields` portrait has `asset_provider == "riot_data_dragon_champion_square"`
- first portrait includes `asset_aliases.wukong == "MonkeyKing"`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-app league_seed_manifest_declares_data_dragon_portrait_provider -- --nocapture`

Expected: FAIL because the manifest does not declare Data Dragon yet.

- [ ] **Step 3: Update manifests**

Add `presentation.data_dragon.version` and the portrait field provider/key/alias config to both the bundled seed JSON and the standalone package JSON. Bump the seed package version and external package version only after tests are green and the zip is rebuilt.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-app league_seed_manifest_declares_data_dragon_portrait_provider -- --nocapture`

Expected: PASS.

### Task 4: External Package Release Refresh

**Files:**
- Modify: `C:\Users\dain\Projects\clipline-plugin-league-of-legends\package\clipline-plugin.json`
- Modify: `apps/clipline-app/src/game_plugins.rs`
- Modify: `handoff.md`

- [ ] **Step 1: Rebuild the standalone package zip**

Create `dist/clipline-plugin-league-of-legends-1.3.1.zip` from the package contents and compute its SHA-256.

- [ ] **Step 2: Update Clipline's known first-party release**

Set the League known release URL to the `v1.3.1` zip and set `sha256` to the computed digest.

- [ ] **Step 3: Verify package install tests**

Run: `cargo test -p clipline-app game_plugins::tests:: -- --nocapture`

Expected: PASS.

- [ ] **Step 4: Update handoff**

Mention that League presentation now uses the Riot Data Dragon champion-square provider for the bottom metadata portrait and that signed metadata remains the next hardening step.

### Task 5: Full Verification

**Files:**
- All changed files.

- [ ] **Step 1: Format**

Run: `cargo fmt --all`

Expected: exit 0.

- [ ] **Step 2: Run focused UI tests**

Run:

```powershell
cargo test -p clipline-app player_summary_fields -- --nocapture
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: PASS.

- [ ] **Step 3: Run workspace tests and lint**

Run:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: both commands exit 0.

- [ ] **Step 4: Launch app for manual check**

Stop any existing `clipline-app.exe`, then run `cargo run -p clipline-app`.

Expected: Settings > Games still renders plugin rows, and League clips with player summaries show a Data Dragon champion portrait in the bottom metadata strip, falling back to initials if the image load fails.
