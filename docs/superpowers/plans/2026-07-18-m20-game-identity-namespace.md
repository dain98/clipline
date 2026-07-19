# M-20 Game Identity Namespace and Migration Plan

> **Finding:** M-20 — custom game IDs can impersonate built-in plugins.

## Goal

Make built-in plugin identity impossible to obtain from a custom game's persisted string ID while
migrating existing collisions without dropping custom-game icons or historical clip associations.

## Design

- Add one Rust-owned game identity module containing the built-in IDs, custom-ID namespace/slug
  rules, and a typed `GameIdentity::{BuiltInPlugin, Custom}` runtime identity.
- Require custom IDs to use the canonical `custom-` namespace and reject built-in/reserved IDs at
  validation boundaries.
- During settings normalization, deterministically migrate legacy, empty, malformed, or built-in-
  colliding custom IDs to unique `custom-…` IDs. Preserve the old ID as a bounded legacy alias on
  the same custom-game record, retaining its embedded icon and allowing old session metadata to
  resolve by alias plus game name.
- Keep frontend generation in the same namespace and reserve the live built-in catalog IDs when
  allocating IDs.
- Replace runtime built-in inference from `DetectedGame.id`/`ActiveGame.id` strings with the typed
  identity. Event sources, osu! title tracking, active-rule continuity, session metadata, and the
  osu! short-session policy must branch on the enum variant before considering an ID.

## TDD sequence

1. Add settings tests proving `osu`, `league_of_legends`, malformed, and colliding legacy custom
   IDs migrate deterministically, preserve icon/name/legacy aliases, and serialize safely.
2. Add validation tests proving newly constructed custom records cannot use a built-in or leave the
   `custom-` namespace.
3. Add game/app/service tests proving a custom identity carrying an old built-in alias cannot set a
   plugin event source, collect osu! titles, or inherit osu!'s minimum full-session duration.
4. Add UI contracts for namespace-aware ID generation and alias-plus-name historical icon lookup.
5. Implement the shared namespace, migration, typed runtime plumbing, and frontend compatibility.
6. Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, workspace Clippy, then launch
   Clipline and smoke Settings > Games plus normal library rendering.
7. Record commits/evidence in the master audit ledger and `handoff.md`. Add a manual test only if an
   installed legacy collision fixture cannot be represented deterministically in tests.

## Expected commits

- `docs(plan): define M-20 game identity remediation`
- `fix(games): namespace custom game identities`
- `docs(audit): close custom game identity finding`
