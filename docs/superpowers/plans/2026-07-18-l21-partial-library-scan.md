# L-21 Partial Library Scan Plan

**Goal:** Keep readable clips visible when one child session cannot be scanned, while making the
partial result explicit to the user and preserving fatal behavior for an unreadable media root.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-21.

## Design boundary

- [ ] Return a typed local-library scan result containing both readable clips and scoped warnings.
- [ ] Keep root clip enumeration and root directory opening fatal because neither permits a
      trustworthy library result.
- [ ] Treat a failed child-session scan as partial: record the session name and diagnostic, log it,
      continue scanning siblings, and preserve normal sorting and asset authorization.
- [ ] Apply warnings only after the frontend request-generation gate accepts the result, preventing
      a stale refresh from overwriting newer UI state.
- [ ] Show partial-scan warnings in the existing app notice/error surface and clear only the prior
      library warning after a later complete scan, without erasing unrelated errors.

## TDD sequence

- [ ] Add a deterministic injected child-reader test proving one denied session is skipped while a
      readable sibling remains in the result with a named warning.
- [ ] Add a root-failure test proving an unavailable media root still fails the command boundary.
- [ ] Add a UI contract for typed result parsing, request-gated warning application, and safe
      previous-warning clearing.
- [ ] Implement the backend scan result and frontend partial-warning handling.

## Verification

- [ ] Run focused library and UI-contract tests.
- [ ] Clean `clipline-app`, then run warning-denied app Clippy.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild and open the exact workspace app; verify the ordinary complete nine-clip Library.
- [ ] Update `handoff.md` and the combined remediation ledger. Keep a manual item only if a real
      permission/disconnected-volume scenario adds coverage that deterministic injection cannot.
