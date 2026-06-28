# Plugin Declarative Presentation

## Goal

Move League marker/gallery/review presentation to optional manifest fields while keeping `clipline-events` as the closed event vocabulary.

## Scope

- [ ] Keep `EventKind`, `GameId`, and `is_timeline_marker()` core-owned.
- [ ] Add optional schema-v1 presentation fields for marker labels/categories/icons, gallery summary/title policy, and a review game panel.
- [ ] Accept M1 manifests without presentation and fall back to core defaults.
- [ ] Thread presentation config into `player-core.js` as explicit function arguments while keeping it DOM-free and Tauri-free.
- [ ] Replace hardcoded League UI names/classes with plugin-driven equivalents without changing the current UX.
- [ ] De-duplicate the `GameId -> plugin_id` bridge used by library and cloud.

## TDD Steps

- [ ] Test M1 manifest compatibility under M2 validation.
- [ ] Test whole-presentation fallback and per-kind fallback.
- [ ] Test dead presentation keys and missing styled kept markers.
- [ ] Test injected presentation config in `player-core` marker style/digest/summary behavior.
- [ ] Test the shared `GameId -> plugin_id` helper.
- [ ] Update `ui_contract` to preserve backend-driven Games rows, League title/KDA rule, digest fallback, marker styling, and no hardcoded League-only gallery branch.

## Verification

- [ ] `cargo test -p clipline-app`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`

