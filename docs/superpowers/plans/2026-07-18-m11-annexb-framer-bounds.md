# M-11 Annex-B Framer Bounds Plan

**Goal:** Keep malformed or delimiter-free FFmpeg output strictly bounded and scan each byte only once without changing valid three/four-byte start-code framing across reader chunks.

## Incremental state machine

- [ ] Replace full-buffer `start_codes` rescans with an incremental cursor plus the current access-unit start and last incomplete NAL boundary.
- [ ] Process a NAL exactly when the next start code arrives, emit completed VCL-terminated units, and adjust all cursor/boundary offsets after draining emitted prefixes.
- [ ] Preserve three-byte and four-byte codes split at every reader boundary, including the optional leading zero of a four-byte code.

## Hard malformed-stream boundary

- [ ] Check `current + incoming` with overflow-safe arithmetic before extending the buffer; if it would exceed 32 MiB, discard the entire framing generation and all cursor/boundary state.
- [ ] Apply the same invariant on every return path, including streams that have never produced a start code.
- [ ] Do not retain suffix bytes across a malformed reset, so a later chunk cannot combine with discarded data into a synthetic start code or access unit.

## Tests and verification

- [ ] Add tests for delimiter-free cap/reset, incremental cursor advancement, split three/four-byte start codes, and post-reset non-merging.
- [ ] Preserve existing H.264/HEVC/IVF framing tests and encoder integrations.
- [ ] Run focused capture tests and fresh-cache capture Clippy.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and update `handoff.md` and the master audit ledger; no manual-only acceptance should remain.
