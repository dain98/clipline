# First-Party Supported Games Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the installable plugin package system with built-in first-party supported game profiles while preserving the League presentation work.

**Architecture:** Keep the useful data-driven presentation layer, but remove package install/update infrastructure. Supported game profiles are compiled into Clipline, assets resolve to data URLs from first-party resources, and event ingestion remains behind built-in capability names.

**Tech Stack:** Rust/Tauri backend, vanilla JavaScript UI, `player-core.js` pure helpers tested through Boa, `ui_contract.rs` source contract tests, bundled first-party assets.

---

## File Structure

- Modify `apps/clipline-app/src/game_plugins.rs`: shrink the registry into first-party supported game profiles, remove install receipts, zip install, package releases, and seed/reseed logic.
- Modify `apps/clipline-app/src/games.rs`: keep detection behavior while consuming the simplified profile catalog.
- Modify `apps/clipline-app/src/app.rs`: remove package-management commands and startup seeding/scope work.
- Modify `apps/clipline-app/ui/main.js`: keep backend-driven supported game rows, remove package action UI, and keep presentation lookups.
- Modify `apps/clipline-app/ui/styles.css`: remove package-action styling and keep supported game row styling.
- Modify `apps/clipline-app/tests/ui_contract.rs`: replace package-install assertions with supported-game-profile assertions.
- Modify `apps/clipline-app/tests/player_core.rs`: keep injected presentation tests for marker, event rail, metadata, and gallery card behavior.
- Modify `apps/clipline-app/Cargo.toml` and `Cargo.lock`: remove dependencies used only by package install/update.
- Modify `apps/clipline-app/tauri.conf.json`: remove `plugin-seeds/**/*` bundle resources if profile assets are embedded into the binary.
- Modify `handoff.md`: mark the plugin package direction as replaced by first-party supported games.
- Keep `apps/clipline-app/plugin-seeds/league_of_legends/assets/` as the first-party League asset source for `include_bytes!` in this milestone. A later cleanup may rename this directory, but this plan only removes runtime package seeding/install behavior.

### Task 1: Backend Profile Model Without Install State

**Files:**
- Modify: `apps/clipline-app/src/game_plugins.rs`

- [ ] **Step 1: Write the failing tests**

Replace package-install tests with profile-catalog tests:

```rust
#[test]
fn league_profile_has_no_install_state_but_keeps_presentation() {
    let profile = all()
        .into_iter()
        .find(|profile| profile.id() == LEAGUE_OF_LEGENDS_ID)
        .expect("league profile");
    let info = profile.info();

    assert_eq!(info.id, LEAGUE_OF_LEGENDS_ID);
    assert_eq!(info.name, "League of Legends");
    assert!(info.default_enabled);
    assert_eq!(info.default_recording_mode, GameRecordingMode::FullSession);
    assert!(info.event_markers);
    assert!(info.icon.as_deref().is_some_and(|icon| icon.starts_with("data:image/png;base64,")));

    let presentation = info.presentation.expect("league presentation");
    assert_eq!(
        presentation.pointer("/event_rail/title").and_then(serde_json::Value::as_str),
        Some("Match events")
    );
    assert_eq!(
        presentation.pointer("/metadata_panel/fields/0/asset_provider").and_then(serde_json::Value::as_str),
        Some("riot_data_dragon_champion_square")
    );
}

#[test]
fn unsupported_event_source_names_are_rejected() {
    let json = r#"{
      "schema_version": 1,
      "id": "future_game",
      "name": "Future Game",
      "summary": "Future game profile.",
      "default_enabled": true,
      "default_recording_mode": "full_session",
      "window_match": { "exe_name": "Future.exe", "selection": "longest_title" },
      "event_source": "future_live_client"
    }"#;

    let err = GameProfileManifest::from_json(json).unwrap_err();

    assert!(err.contains("unsupported game event source"), "{err}");
}

#[test]
fn declarative_league_matcher_preserves_longest_title_behavior() {
    let manifest = league_profile_manifest();
    let windows = vec![
        window(1, "League of Legends", "LeagueClientUx.exe", None),
        window(2, "League", "League of Legends.exe", None),
        window(3, "League of Legends (TM) Client", "League of Legends.exe", None),
    ];

    let matched = manifest.match_window(&windows).expect("game window");

    assert_eq!(matched.handle, 3);
    assert_eq!(matched.exe_name, "League of Legends.exe");
}
```

Remove tests whose only purpose is package install behavior:

```rust
seed_does_not_clobber_manual_or_unknown_installs
seed_updates_only_seeded_older_installs
corrupt_receipt_loads_valid_manifest_as_unknown
zip_package_rejects_bad_digest_without_changing_active_install
zip_package_rejects_corrupt_zip_without_changing_active_install
zip_package_rejects_zip_slip_paths_without_changing_active_install
zip_package_rejects_unknown_event_source_without_changing_active_install
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app game_plugins::tests::league_profile_has_no_install_state_but_keeps_presentation -- --nocapture
```

Expected: FAIL because `GamePluginInfo` still exposes install/package state and assets currently depend on installed package roots.

- [ ] **Step 3: Implement the simplified profile model**

In `apps/clipline-app/src/game_plugins.rs`, remove `semver`, `sha2`, `InstalledPluginRecord`, `InstallProvenance`, `KnownFirstPartyPackageRelease`, `SeedOutcome`, `ManualInstallOutcome`, zip extraction, receipt helpers, package release helpers, `plugin_install_root`, and seed install functions.

Replace the manifest and info shapes with first-party profile shapes:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct GamePluginInfo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub event_markers: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<serde_json::Value>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameProfileManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    #[serde(default)]
    pub icon: Option<GameProfileIcon>,
    pub window_match: WindowMatchRule,
    #[serde(default)]
    pub event_source: Option<String>,
    #[serde(default)]
    pub presentation: Option<serde_json::Value>,
}
```

Update validation copy to say "game profile" instead of "plugin manifest":

```rust
if self.schema_version != GAME_PROFILE_SCHEMA_VERSION {
    return Err(format!(
        "unsupported game profile schema {}; expected {}",
        self.schema_version, GAME_PROFILE_SCHEMA_VERSION
    ));
}
```

Change `GamePlugin` to hold only the manifest:

```rust
#[derive(Debug, Clone)]
pub struct GamePlugin {
    pub manifest: GameProfileManifest,
}
```

Change `all()` to return built-ins directly:

```rust
pub fn all() -> Vec<GamePlugin> {
    vec![GamePlugin {
        manifest: league_profile_manifest(),
    }]
}
```

Resolve profile assets from embedded first-party bytes:

```rust
fn first_party_asset_data_url(path: &str) -> Option<String> {
    let bytes: &[u8] = match path {
        "assets/games/league-of-legends.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/games/league-of-legends.png"),
        "assets/markers/kill.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/markers/kill.png"),
        "assets/markers/death.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/markers/death.png"),
        "assets/markers/dragon.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/markers/dragon.png"),
        "assets/markers/baron.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/markers/baron.png"),
        "assets/markers/turret.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/markers/turret.png"),
        "assets/event-rail/kill.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/event-rail/kill.png"),
        "assets/event-rail/death.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/event-rail/death.png"),
        "assets/event-rail/dragon.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/event-rail/dragon.png"),
        "assets/event-rail/baron.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/event-rail/baron.png"),
        "assets/event-rail/turret.png" => include_bytes!("../plugin-seeds/league_of_legends/assets/event-rail/turret.png"),
        _ => return None,
    };
    Some(crate::game_icon::png_data_url(bytes))
}
```

Update `GameProfileIcon::File` and presentation icon resolution to call `first_party_asset_data_url`.

- [ ] **Step 4: Run focused tests**

Run:

```powershell
cargo test -p clipline-app game_plugins::tests:: -- --nocapture
```

Expected: PASS for the remaining profile, matcher, event source, and asset tests.

### Task 2: Remove Package Commands, Startup Seeding, and Install Dependencies

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/Cargo.toml`
- Modify: `apps/clipline-app/tauri.conf.json`
- Modify: `Cargo.lock`

- [ ] **Step 1: Write the failing contract test**

In `apps/clipline-app/tests/ui_contract.rs`, update the Settings > Games contract so it requires no package commands:

```rust
assert!(
    main_js().contains("await invoke(\"list_game_plugins\")")
        && main_js().contains("renderGamePlugins")
        && main_js().contains("gamePluginSettings")
        && main_js().contains("plugin.presentation")
        && main_js().contains("games.plugins")
        && main_js().contains("dataset.gamePluginEnabled")
        && main_js().contains("game-plugin-mode-")
        && main_js().contains("normalizeGamePluginId")
        && main_js().contains("Takes priority over matching custom games.")
        && styles_css().contains(".game-profile-mode")
        && !main_js().contains("check_game_plugin_package")
        && !main_js().contains("update_game_plugin_package")
        && !main_js().contains("reinstall_game_plugin_package")
        && !main_js().contains("reset_game_plugin_to_seed")
        && !main_js().contains("dataset.gamePluginAction")
        && !styles_css().contains(".game-plugin-actions"),
    "supported games must render from backend profiles without package install/update actions"
);
```

Add an app-source contract assertion near the invoke-handler checks:

```rust
assert!(
    !app_rs().contains("check_game_plugin_package")
        && !app_rs().contains("update_game_plugin_package")
        && !app_rs().contains("reinstall_game_plugin_package")
        && !app_rs().contains("reset_game_plugin_to_seed")
        && !app_rs().contains("seed_bundled_plugins")
        && !app_rs().contains("plugin_install_root"),
    "Clipline should not expose installable game package commands"
);
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: FAIL because package commands/actions are still present.

- [ ] **Step 3: Remove app package management**

Delete these backend items from `apps/clipline-app/src/app.rs`:

```rust
GamePluginPackageStatus
game_plugin_package_status
plugin_seed_root
reset_plugin_to_seed
download_first_party_plugin_package
install_first_party_plugin_release
check_game_plugin_package
update_game_plugin_package
reinstall_game_plugin_package
reset_game_plugin_to_seed
```

Remove these invoke handler entries:

```rust
check_game_plugin_package,
update_game_plugin_package,
reinstall_game_plugin_package,
reset_game_plugin_to_seed,
```

Remove this setup block:

```rust
match app.path().resource_dir() {
    Ok(resource_dir) => {
        let seed_root = resource_dir.join("plugin-seeds");
        if let Err(e) = crate::game_plugins::seed_bundled_plugins(&seed_root) {
            eprintln!("could not seed bundled game plugins from {seed_root:?}: {e}");
        }
    }
    Err(e) => eprintln!("could not resolve resource dir for plugin seeds: {e}"),
}
let plugin_install_root = crate::game_plugins::plugin_install_root();
if let Err(e) = std::fs::create_dir_all(&plugin_install_root) {
    eprintln!("could not create plugin install root {plugin_install_root:?}: {e}");
} else if let Err(e) = app
    .asset_protocol_scope()
    .allow_directory(&plugin_install_root, true)
{
    eprintln!(
        "could not scope plugin install root {plugin_install_root:?} for assets: {e}"
    );
}
```

In `apps/clipline-app/Cargo.toml`, remove app dependencies that were only used for game package install:

```toml
semver = { version = "1", features = ["serde"] }
sha2 = "0.10"
zip = { version = "2", default-features = false, features = ["deflate"] }
```

In `apps/clipline-app/tauri.conf.json`, remove bundled game package resources:

```json
"resources": [
  "plugin-seeds/**/*"
],
```

Then let Cargo refresh dependency usage:

```powershell
cargo check -p clipline-app
```

If `Cargo.lock` still contains transitive `semver`, `sha2`, or `zip` entries used by other dependencies, leave them alone. The success criterion is that `clipline-app` no longer declares direct dependencies for installable game package handling.

Run a metadata sanity check after the lockfile settles:

```powershell
cargo metadata --format-version 1 --locked
```

- [ ] **Step 4: Run focused tests**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
cargo check -p clipline-app
```

Expected: both commands exit 0.

### Task 3: Frontend Supported Game Rows Without Package Actions

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write/adjust the UI contract**

Keep the backend-driven supported-games contract from Task 2 and add these source assertions:

```rust
assert!(
    main_js().contains("empty.textContent = \"no supported games available\"")
        && !main_js().contains("not installed")
        && !main_js().contains("repair available")
        && !main_js().contains("Package is current"),
    "Settings > Games copy should describe built-in supported games, not installable packages"
);
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: FAIL because package status text and action helpers still exist.

- [ ] **Step 3: Remove package UI helpers**

Delete these functions from `apps/clipline-app/ui/main.js`:

```javascript
function gamePluginPackageStatusText(plugin) { ... }
async function checkGamePluginPackage(plugin) { ... }
async function runGamePluginPackageAction(plugin, command, busyText) { ... }
function renderGamePluginPackageActions(plugin) { ... }
function updateGamePluginPackageStatus(pluginId, message) { ... }
```

Change the empty-state copy in `renderGamePlugins`:

```javascript
empty.textContent = "no supported games available";
```

Change the row append call from:

```javascript
row.append(enabled, meta, renderGamePluginModeControl(plugin, settings), renderGamePluginPackageActions(plugin));
```

to:

```javascript
row.append(enabled, meta, renderGamePluginModeControl(plugin, settings));
```

Remove `.game-plugin-actions` styles from `apps/clipline-app/ui/styles.css`.

- [ ] **Step 4: Run focused checks**

Run:

```powershell
node --check apps/clipline-app/ui/main.js
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: both commands exit 0.

### Task 4: Preserve League Presentation Behavior

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Verify: `apps/clipline-app/ui/player-core.js`
- Verify: `apps/clipline-app/ui/main.js`

- [ ] **Step 1: Keep focused player-core tests**

Keep or add tests that exercise the surviving first-party profile presentation:

```rust
#[test]
fn gallery_card_preview_uses_profile_title_stats_and_portrait_icon() {
    let mut ctx = js_context_with_player_core();
    eval_ok(
        &mut ctx,
        r#"
        const CARD_CLIP = {
          markers: {
            player_summary: {
              champion_name: "Vel'Koz",
              kills: 11,
              deaths: 19,
              assists: 34,
              creep_score: 177,
              duration_s: 1345
            }
          }
        };
        const CARD_PRESENTATION = {
          data_dragon: { version: '16.13.1' },
          gallery: {
            card: {
              title: 'summary_for_full_session',
              title_format: {
                type: 'player_summary_stats',
                separator: ' | ',
                stats: [{ type: 'kda' }, { type: 'cs_per_min', label: 'CS/min' }]
              },
              icon: {
                type: 'portrait',
                source: 'player_summary.champion_name',
                label: 'Champion',
                asset_provider: 'riot_data_dragon_champion_square',
                asset_key_format: 'data_dragon_champion',
                asset_aliases: { "vel'koz": 'Velkoz' }
              }
            }
          }
        };
        "#,
    );

    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.galleryCardPreview(CARD_CLIP, 'session', 'Jun 28 - 12:15 PM', CARD_PRESENTATION, { data_dragon: CARD_PRESENTATION.data_dragon })"
        ),
        r#"{"title":"11/19/34 | 7.9 CS/min","titleSource":"summary","summary":"","icon":{"type":"portrait","label":"Champion","value":"Vel'Koz","assetKey":"Velkoz","asset":"https://ddragon.leagueoflegends.com/cdn/16.13.1/img/champion/Velkoz.png"}}"#
    );
}
```

Keep existing tests for:

```rust
marker_styles_accept_injected_plugin_presentation
player_summary_fields_resolve_data_dragon_portraits
player_summary_fields_resolve_summoner_spells_and_items
game_event_rail_item_formats_duel_with_data_dragon_portraits
```

- [ ] **Step 2: Run tests**

Run:

```powershell
cargo test -p clipline-app marker_styles_accept_injected_plugin_presentation -- --nocapture
cargo test -p clipline-app player_summary_fields_resolve_data_dragon_portraits -- --nocapture
cargo test -p clipline-app player_summary_fields_resolve_summoner_spells_and_items -- --nocapture
cargo test -p clipline-app game_event_rail_item_formats_duel_with_data_dragon_portraits -- --nocapture
```

Expected: PASS. If a test still says "plugin" in the name, keep it passing first; rename tests only after behavior is green.

- [ ] **Step 3: Ensure UI contract keeps the current UX guards**

In `apps/clipline-app/tests/ui_contract.rs`, ensure the major presentation assertion still checks:

```rust
main_js().contains("function pluginPresentationForClip(clip)")
main_js().contains("function clipGalleryCardPreview(clip, kind, fallbackTitle)")
main_js().contains("function renderGameEventRail")
main_js().contains("function renderGameMetadataPanel")
main_js().contains("data_dragon: presentation && presentation.data_dragon")
main_js().contains("cardPreview.titleSource === \"summary\"")
styles_css().contains(".game-event-rail-tab")
styles_css().contains(".game-event-row-friendly")
styles_css().contains(".game-event-row-enemy")
styles_css().contains(".game-event-rail ol button.marker-kill .game-event-kind-icon img")
styles_css().contains(".game-event-rail ol button.marker-death .game-event-kind-icon img")
styles_css().contains(".clip .game-meta")
!main_js().contains("LEAGUE_OF_LEGENDS_ID")
!main_js().contains("isLeagueClip")
```

- [ ] **Step 4: Run UI contract**

Run:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: PASS.

### Task 5: Documentation and Obsolete Plan Cleanup

**Files:**
- Modify: `handoff.md`
- Modify: `docs/superpowers/plans/2026-06-28-plugin-manifest-catalog.md`
- Modify: `docs/superpowers/plans/2026-06-28-plugin-declarative-presentation.md`
- Modify: `docs/superpowers/plans/2026-06-28-plugin-manual-install-update.md`
- Modify: `docs/superpowers/plans/2026-06-28-plugin-review-layout.md`

- [ ] **Step 1: Mark old plugin plans as superseded**

At the top of each old plugin plan, add:

```markdown
> Superseded by `docs/superpowers/plans/2026-06-29-first-party-supported-games.md`.
> Clipline is no longer pursuing installable game presentation plugins in this branch.
```

- [ ] **Step 2: Update handoff**

Replace the "League presentation plugin extraction" handoff section with:

```markdown
33. **First-party supported game presentation** - the installable plugin direction was replaced with built-in supported game profiles. League remains the first profile, with declarative presentation data for marker styling, gallery cards, a playback-synced right-side event rail, and bottom metadata. Event ingestion stays core-owned behind the built-in `league_live_client` capability; game updates ship with normal Clipline releases instead of external plugin zips.
```

Remove or rewrite the "Signed plugin metadata" follow-up so it does not imply an external League package release remains planned.

- [ ] **Step 3: Run docs/source checks**

Run:

```powershell
rg -n "clipline-plugin-league-of-legends|Signed plugin metadata|manual install|reset-to-seed|receipt|seeded|provenance" handoff.md docs/superpowers/plans apps/clipline-app/src apps/clipline-app/ui apps/clipline-app/tests
```

Expected: only superseded old plan notes or intentional compatibility comments remain.

### Task 6: Full Verification

**Files:**
- All changed files.

- [ ] **Step 1: Format**

Run:

```powershell
cargo fmt --all
```

Expected: exit 0.

- [ ] **Step 2: Run focused tests**

Run:

```powershell
node --check apps/clipline-app/ui/main.js
cargo test -p clipline-app game_plugins::tests:: -- --nocapture
cargo test -p clipline-app player_summary_fields -- --nocapture
cargo test -p clipline-app game_event_rail_item -- --nocapture
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

Expected: all commands exit 0.

- [ ] **Step 3: Run workspace tests and lint**

Run:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: both commands exit 0.

- [ ] **Step 4: Launch app for manual check**

Stop any existing `clipline-app.exe`, then run:

```powershell
cargo run -p clipline-app
```

Expected manual checks:

- Settings > Games shows League as a supported game with enabled and recording mode controls, without Check/Update/Reinstall/Reset package buttons.
- League clips still show champion portrait, summoner spells, KDA ratio, and item build in the bottom metadata area.
- Gallery cards still use champion portrait icons and KDA/CS-per-minute titles.
- The right-side match events rail still renders portraits/icons, colors friendly/enemy rows, follows playback, and collapses via the pull tab.
- Existing League recording and Live Client marker polling still work.
