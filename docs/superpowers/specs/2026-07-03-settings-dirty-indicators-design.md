# Settings Dirty Indicators Design

## Goal

Show users exactly which Settings controls have unsaved changes by adding a glow around changed settings and a pip on any Settings tab that contains changed controls.

## User-Facing Behavior

- A setting row glows while its current draft value differs from the saved baseline shown when Settings was opened or last saved.
- If the user changes a value back to its saved value, that row's glow disappears.
- Any tab containing at least one changed setting row shows a small pip beside the tab label.
- Save, discard, and repainting settings from the saved state clear all row glows and tab pips.
- The existing unsaved-close behavior remains unchanged: dirty settings still change `Close` to `Discard Changes`, warn on first discard/backdrop attempt, and discard only through the action button path.

## Architecture

Reuse the existing settings draft flow in `ui/settings.js`. Each visible setting surface gets a `data-settings-key` attribute listing one or more settings paths that row represents. A normalized baseline is captured from `readSettings()` after `fillSettings()` finishes painting controls. Indicator helpers compare each row's keyed values in `settingsDraft` against that normalized baseline using the existing stable snapshot comparison.

Static rows declare keys directly in `index.html`. Dynamic game rows assign keys when rendered:

- supported game rows use `games.plugins.<plugin_id>`;
- custom game rows and the custom games panel use `games.custom_games`;
- settings rows with grouped controls use multiple space-separated paths.

The tab pip is derived from changed keyed rows inside each `.settings-section`, so it remains accurate when the user edits hidden tabs.

## Components

- `apps/clipline-app/ui/index.html`
  - Add `data-settings-key` to static `.setting-row`, `#capture-region-editor`, `.advanced-box`, and game panel surfaces.

- `apps/clipline-app/ui/settings.js`
  - Add `settingsIndicatorBaseline`.
  - Add helpers for path lookup, keyed value comparison, row indicator syncing, and tab pip syncing.
  - Call indicator syncing from `syncSettingsDirtyState()`, after `fillSettings()` establishes the normalized baseline, and after dynamic game lists render.

- `apps/clipline-app/ui/styles.css`
  - Add a glow style for `.setting-changed`.
  - Add a tab pip using `.settings-tab-changed::after`.
  - Keep layout dimensions stable so pips and glows do not shift controls.

- `apps/clipline-app/tests/ui_contract.rs`
  - Add contract coverage for keyed markup, indicator helper names, dynamic game row keys, and CSS classes.

## Testing

- Write a failing UI contract test first for dirty indicator markup/CSS/JS.
- Run the focused test and verify RED.
- Implement minimal code to pass.
- Run `cargo test -p clipline-app --test ui_contract`.
- Run `cargo test --workspace`.
- Run `cargo clean -p clipline-app && cargo clippy --workspace --all-targets -- -D warnings`.

## Out of Scope

- Persisted settings schema changes.
- Reorganizing settings tabs or controls.
- Adding per-field text labels that explain changed state.
- Changing game plugin settings dialog behavior outside the main Settings tab indicator.
