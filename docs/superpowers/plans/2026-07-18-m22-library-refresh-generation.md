# M-22 Local Library Refresh Generation Plan

> **Finding:** M-22 — concurrent local-library refreshes can apply stale snapshots.

## Goal

Make local Library snapshots latest-request-wins, prevent an in-flight snapshot from undoing an
optimistic rename/delete/export mutation, and ensure backend-event refresh failures are surfaced
without unhandled promise rejections.

## TDD sequence

- [ ] Add a UI contract that requires a dedicated local request gate, current-generation checks
  before cache/review mutation, invalidation at every direct local-cache mutation, and a caught
  fire-and-forget event refresh path.
- [ ] Run the focused UI contract and record the expected red failure.
- [ ] Start every `list_clips` request through the gate, suppress stale successes and failures, and
  preserve `preferredCurrentPath` only for the response that is still current.
- [ ] Invalidate pending snapshots before optimistic rename, delete, and exported-clip cache writes.
- [ ] Route saved and osu! enrichment events through one error-reporting refresh wrapper.
- [ ] Run focused UI/player tests, fresh-cache app Clippy, CI-mode workspace tests, and workspace
  Clippy with warnings denied.
- [ ] Rebuild and open Clipline for a native Library/review smoke check.
- [ ] Update `handoff.md` and the combined audit ledger. Add a manual acceptance item only if the
  remaining race cannot be covered deterministically.

## Invariants

- [ ] Only the most recently started local Library request may replace `clipsCache`, update the
  current review, close review, or render its snapshot.
- [ ] A completed local mutation invalidates every snapshot that began before it.
- [ ] Superseded request failures are silent; the current request's failure reaches awaited callers
  or the visible global error surface.
- [ ] Event bursts may issue overlapping reads, but cannot apply old results or starve the newest
  request once the burst ends.
- [ ] Cloud request arbitration and local/cloud source switching remain unchanged.

## Commits

- `docs(plan): define M-22 library refresh remediation`
- `fix(ui): reject stale local library snapshots`
- `docs(audit): close local library refresh finding`
