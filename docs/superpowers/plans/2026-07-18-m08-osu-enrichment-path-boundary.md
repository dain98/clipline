# M-08 osu! Enrichment Path Boundary Plan

**Goal:** Make a pending osu! enrichment sidecar authoritative only for its enrichment data, never for which filesystem object Clipline writes or deletes.

## Path binding

- [ ] Return a discovered job that retains the actual pending-sidecar path and an MP4 path derived from that sidecar's filename and directory.
- [ ] Canonicalize the media root, discovered sidecar, serialized `clip_path`, and expected MP4; require one exact existing MP4 at the root or one session directory below it.
- [ ] Reject missing/non-MP4 targets, serialized-path mismatches, symlink/reparse-point files, and linked/reparse session-directory traversal.
- [ ] Use only the bound job paths for marker publication, retry/failure rewrites, and completed-sidecar deletion.

## Compatibility and callers

- [ ] Keep `clip_path` in schema version 1 as a migration/rename consistency field, but never derive an I/O path from it after discovery.
- [ ] Preserve the library rename transaction that rewrites this consistency field when Clipline itself moves a clip and pending sidecar.
- [ ] Keep pure score-to-play mapping APIs independent of filesystem discovery.

## Tests and verification

- [ ] Add a crafted-record regression proving an outside `clip_path` cannot create/overwrite marker or pending files outside the media root.
- [ ] Add discovery tests for correctly bound jobs, missing expected clips, mismatched paths, root/session depth, and link/reparse refusal where the platform permits.
- [ ] Run focused app tests and fresh-cache app Clippy.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and update `handoff.md` and the master audit ledger; no manual-only acceptance should remain for this deterministic boundary.
