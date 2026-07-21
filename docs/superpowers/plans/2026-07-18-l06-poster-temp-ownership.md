# L-06 Poster Temp Ownership Plan

> **Finding:** L-06 — concurrent poster generation shares one predictable temporary path.

## Goal

Give every FFmpeg poster attempt exclusive temporary-file ownership, publish only complete JPEGs
through an atomic replacement, and clean every losing or failed temporary artifact.

## TDD sequence

- [ ] Add deterministic tests proving concurrent reservations for one poster are distinct sibling
  paths, drop cleanup is scoped, and atomic publication replaces a stale poster with complete data.
- [ ] Run focused tests and record the expected compile/behavior failures.
- [ ] Reserve temporary files with `create_new` using process/counter uniqueness and an RAII cleanup
  guard.
- [ ] Point FFmpeg at the reserved path and atomically replace the poster only after successful exit.
- [ ] Preserve the existing fresh-cache fast path and error diagnostics.
- [ ] Run focused tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace Clippy with
  warnings denied.
- [ ] Rebuild/open Clipline and verify ordinary Library poster loading.
- [ ] Update `handoff.md` and the combined audit ledger; add no manual item unless real FFmpeg
  concurrency remains untestable.

## Invariants

- [ ] Two overlapping generations for one clip never remove, truncate, or write the same temp file.
- [ ] A failed spawn, failed encode, failed publication, or losing attempt removes only its own temp.
- [ ] The visible poster path is either the previous complete JPEG or a newly completed JPEG, never
  a partial FFmpeg output.
- [ ] Stale poster replacement works on Windows and is write-through at the publication boundary.
- [ ] Temp uniqueness is bounded and does not retain an in-memory key map.

## Commits

- `docs(plan): define L-06 poster temp ownership`
- `fix(library): isolate concurrent poster generation`
- `docs(audit): close poster temp collision finding`
