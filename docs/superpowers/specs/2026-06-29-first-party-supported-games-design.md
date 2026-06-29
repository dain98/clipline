# First-Party Supported Games Design

## Goal

Replace the installable game plugin direction with first-party supported game profiles built into Clipline. Community contributions happen as normal open-source pull requests, while Clipline ships the polished integrations directly.

## Decision

Clipline will not ship a plugin marketplace, manual package installer, receipt format, seed/reseed flow, external League package repository, or downloadable game adapters for now. Those pieces add security, release, and maintenance cost that users are unlikely to value if the built-in integrations are good.

The useful part of the plugin work stays: game-specific detection, event source selection, marker presentation, right-side event rails, bottom metadata, and gallery card customization are still data-driven enough to keep League from sprawling through the UI. They are not user-installed packages.

## Architecture

Supported games are first-party profiles compiled into the app. A profile describes:

- id, name, summary, default enabled state, and default recording mode.
- anti-cheat-safe window matching rules.
- an optional built-in event source capability such as `league_live_client`.
- optional presentation data for marker styling, event rail layout, bottom metadata, and gallery cards.
- bundled asset paths resolved by Clipline to data URLs.

The event ingestion boundary stays core-owned. A profile can name a capability that the binary already implements, but it cannot ship code, arbitrary JavaScript, native libraries, or arbitrary third-party URLs.

## Compatibility

The existing settings key `games.plugins.<id>` remains as a compatibility key for now. The UI should call these "Supported games", and internal code can gradually move away from plugin naming as part of the cleanup. Existing League clips, marker sidecars, KDA display, champion portraits, item/summoner metadata, event rail behavior, and Live Client polling must keep working.

## Removed Scope

This pivot removes:

- `%APPDATA%\Clipline\plugins\<id>` installs.
- `clipline-plugin.receipt.json`.
- seeded/manual/unknown provenance handling.
- package version comparison and SemVer-based reseeding.
- GitHub release zip downloads for game packages.
- package SHA-256 verification and zip extraction.
- Settings buttons for Check, Update, Reinstall, and Reset-to-seed.
- the standalone `clipline-plugin-league-of-legends` release flow.

The app updater still uses the existing Tauri/minisign flow. Game integration updates ship with normal Clipline builds.

## Data Flow

1. Rust builds a list of supported game profiles from first-party profile definitions.
2. Settings > Games renders the supported game rows from that backend list.
3. Game detection uses the profile window matcher and the existing per-game settings.
4. Recording stores the supported game id on clips as it does today.
5. `main.js` looks up the profile presentation for the current clip.
6. `player-core.js` receives presentation data as explicit arguments and remains DOM-free and Tauri-free.
7. UI rendering uses the profile presentation for marker ticks, the right event rail, bottom metadata, and gallery cards.

## Future Games

New games are normal first-party implementation work:

- CS2: likely a built-in Valve Game State Integration capability plus a CS2 profile.
- Apex: a built-in LiveAPI capability if local testing confirms it works for normal users.
- TFT: likely OCR/synthetic round markers plus Riot postgame data, if that feels good enough.
- Valorant and Fortnite: only when an official, safe data source supports a worthwhile integration.

Community members can still contribute assets, styling, event mapping, and integrations through pull requests.

## Tests

The cleanup should preserve the current coverage bars:

- backend-driven Settings > Games rows.
- no hardcoded League-only gallery branch.
- League title/KDA/card icon behavior.
- digest-in-subtitle fallback.
- marker tick/category styling.
- right-side event rail behavior, including collapse tab behavior.
- bottom metadata rendering with champion portrait, summoner spells, KDA ratio, and item build.
- `player-core.js` marker/gallery/metadata behavior under injected presentation config.
- shared `GameId -> supported_game_id` helper behavior.

## Non-Goals

This design does not add CS2, TFT, Apex, Valorant, or Fortnite. It only changes the extension model so the League work becomes a clean first-party foundation instead of an installable plugin system.
