# Plugin Manifest Catalog

## Goal

Move the existing built-in game plugin seam to manifest-backed records without changing user-visible League behavior.

## Scope

- [ ] Add schema-versioned `PluginManifest` and `InstalledPluginRecord` types.
- [ ] Seed the first-party League package from bundled resources into `%APPDATA%\Clipline\plugins`.
- [ ] Preserve `games.plugins.<id>` settings and `list_game_plugins` frontend shape.
- [ ] Replace `GamePlugin.match_window` function pointers with declarative window rules.
- [ ] Keep event ingestion behind built-in capability names, starting with `league_live_client`.
- [ ] Treat missing/corrupt receipts as unknown provenance: load valid manifests, never auto-overwrite.

## TDD Steps

- [ ] Test unsupported manifest schema rejection.
- [ ] Test SemVer package ordering for seed decisions.
- [ ] Test seeded/manual/unknown provenance clobber rules.
- [ ] Test corrupt receipt loads valid manifest as unknown and requests repair.
- [ ] Test declarative League window matching selects the longest `League of Legends.exe` title and ignores launcher/client-only windows.
- [ ] Test `list_game_plugins` still exposes League defaults and event marker support.

## Verification

- [ ] `cargo test -p clipline-app`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`

