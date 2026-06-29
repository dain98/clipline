# League Data Dragon Portraits Design

## Goal

Use Riot Data Dragon as the first external presentation data source for the League plugin, starting with champion portraits in the review player's bottom metadata strip.

## Scope

This first slice only changes presentation. Live Client polling, marker persistence, game detection, and the closed `EventKind` vocabulary stay core-owned and unchanged.

The League package declares that its portrait field uses the known `riot_data_dragon_champion_square` asset provider. Clipline builds the Data Dragon champion square URL from the field config and the existing `player_summary.champion_name`; it does not allow arbitrary manifest-provided third-party URLs.

## Manifest Shape

`presentation.data_dragon` may declare:

- `version`: Data Dragon CDN version used for package assets. This first pass pins a version in the package.

Metadata portrait fields may declare:

- `asset_provider: "riot_data_dragon_champion_square"`
- `asset_key_format: "data_dragon_champion"`
- `asset_aliases`: normalized champion-name overrides for Data Dragon's irregular ids.

The generic `asset_template` behavior remains supported for local or already-resolved assets.

## Data Flow

1. `list_game_plugins` sends the manifest presentation block to the UI.
2. `main.js` passes `presentation.metadata_panel.fields` plus `presentation.data_dragon` into `playerSummaryFields`.
3. `player-core.js` stays DOM-free and Tauri-free. It turns a champion name into a Data Dragon key, applies aliases, and returns a resolved portrait `asset` URL.
4. `main.js` renders the image. Browser image failure keeps the existing initials fallback.

## Fallbacks

If no summary exists, no fields render. If a champion name is missing, no portrait renders. If a Data Dragon key cannot be resolved or the CDN image fails to load, the current text fallback remains.

## Tests

Add focused `player_core` tests for:

- Data Dragon champion portrait URL generation.
- Alias handling for irregular ids such as Wukong.
- Existing M1/M2 manifest compatibility and local `asset_template` behavior.

Add UI/manifest contract coverage that the League seed package declares the Data Dragon provider rather than a hardcoded League-only renderer branch.
