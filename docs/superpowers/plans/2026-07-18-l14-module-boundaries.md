# L-14 Module Boundaries Plan

**Goal:** Reduce the audit's remaining high-coupling Rust and renderer surfaces with named domain
owners, one shared presentation contract, and an incremental ES-module bootstrap that preserves
the DOM-free Boa harness.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-14.

## Design boundary

- [ ] Extract focused Rust owners from each cited application monolith: diagnostic logging from
      `app`, writable media-root policy from `service`, clip naming from `library`, and cache
      identity/path policy from `cloud`. Keep Tauri command shells in their current public modules.
- [ ] Give extracted modules narrow `pub(super)` APIs and keep platform effects behind existing
      safe wrappers; do not create new public crate API.
- [ ] Add one DOM-free presentation core for filename stems, marker labels, and calendar labels.
      Normalize the supported video suffix set (`mp4`, `webm`, `mkv`, `mov`) consistently.
- [ ] Preserve `PlayerCore`/`CloudCore` classic global adapters for Boa, while adding explicit ES
      module wrappers and a small module bootstrap/controller boundary for the live renderer.
- [ ] Keep the migration incremental: legacy DOM controllers may remain compatibility-owned, but
      startup and newly shared presentation behavior must not depend on implicit ordered bindings.
- [ ] Add repository contracts for extracted Rust owners, the single presentation implementation,
      module bootstrap, and the absence of the three duplicated helper definitions.

## TDD sequence

- [ ] Extend Boa fixtures with MP4 and non-MP4 filename cases, marker fallback/override cases, and
      month/day formatting from the shared core.
- [ ] Add UI contracts that require `type="module"` bootstrap and explicit core/presentation
      imports while retaining the compatibility adapters required by the existing harness.
- [ ] Add repository structure checks for each Rust domain module and production-file ownership.
- [ ] Extract one Rust domain at a time, running its focused app tests and warning-denied Clippy.
- [ ] Route library/cloud/player presentation call sites through the shared core and remove local
      copies.

## Verification

- [ ] Run app unit, Boa core, UI contract, and repository contract suites.
- [ ] Clean `clipline-app`, then run warning-denied Clippy for all targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open Clipline and verify Library, Settings, review playback, and Cloud disconnected
      state. Add only genuinely manual native/account follow-ups to the combined checklist.
- [ ] Update `handoff.md` and the combined remediation ledger, then reconcile all finding IDs.
