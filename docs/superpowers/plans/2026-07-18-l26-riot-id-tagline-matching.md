# L-26 Riot ID Tagline Matching Plan

**Goal:** Prevent same-game-name players with different Riot ID taglines from being confused while
retaining compatibility with Live Client payloads that omit taglines.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-26.

## Design boundary

- [ ] Parse player names into a case-insensitive, whitespace-normalized game-name key plus an
      optional normalized full Riot ID.
- [ ] When both compared values contain taglines, require their full Riot IDs to match.
- [ ] Permit game-name fallback when either side lacks a usable tagline so older/partial payloads
      remain compatible.
- [ ] For player-summary lookup, scan all full Riot IDs before using any name-only fallback so an
      earlier same-name participant cannot shadow the exact local player.

## TDD sequence

- [ ] Add normalization fixtures proving distinct taglines do not mark a foreign kill, while an
      untagged event value still matches a tagged local identity.
- [ ] Add a player-list collision fixture with two identical game names and different taglines,
      placing the wrong player first and asserting the exact Riot ID wins.
- [ ] Retain existing case, surrounding-whitespace, summoner-name, and missing-tagline coverage.
- [ ] Implement one shared identity parser/matcher and the exact-first summary lookup.

## Verification

- [ ] Run focused `clipline-lol` tests.
- [ ] Clean `clipline-lol`, then run warning-denied Clippy for all of its targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild and open the native app because the League integration is linked into it.
- [ ] Update `handoff.md` and the combined remediation ledger; no manual item is expected because
      the payload variants and collision ordering are deterministic fixtures.
