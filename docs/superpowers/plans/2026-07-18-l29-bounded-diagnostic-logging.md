# L-29 Bounded Diagnostic Logging Plan

**Goal:** Keep Clipline's diagnostic logs bounded throughout a long-running process and avoid
turning interactive window movement into synchronous log traffic.

**Finding:** `CODEBASE_AUDIT_COMBINED.md` L-29.

## Design boundary

- [ ] Replace the process-lifetime bare file handle with a small locked writer that tracks its
      active path and byte count.
- [ ] Before a line would cross the 1 MiB active-file cap, close the Windows handle, replace the
      single old generation, and reopen a fresh active file.
- [ ] Bound any individual formatted line so one hostile or unexpectedly large diagnostic cannot
      defeat the size invariant.
- [ ] Filter high-frequency move and resize `WindowEvent`s while retaining close, focus, destroy,
      DPI, drag/drop, and theme diagnostics.
- [ ] Remove redundant per-line flushing; writes still go directly to the unbuffered `File` and
      the handle is flushed before rotation.

## TDD sequence

- [ ] Add a temporary-directory test that repeatedly writes across several generations and proves
      both active and old logs remain bounded while the newest message is retained.
- [ ] Add an oversized-message fixture proving line truncation cannot create an oversized active
      generation.
- [ ] Add window-event filtering fixtures for move/resize versus focus/destroy.
- [ ] Implement the stateful writer, rotation recovery, line bound, and generic-event filter.

## Verification

- [ ] Run focused app logging tests and the complete app test target.
- [ ] Clean `clipline-app`, then run warning-denied Clippy for all app targets.
- [ ] Run CI-mode workspace tests and warning-denied workspace Clippy.
- [ ] Rebuild/open the native app and move/resize it with Computer Use; confirm the active log stays
      quiet for those events and records a retained focus transition.
- [ ] Update `handoff.md` and the combined remediation ledger. No user manual item is expected
      because byte bounds and event filtering are deterministic and the runtime smoke is local.
