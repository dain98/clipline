# M-15 FFmpeg Process Lifecycle Plan

**Goal:** Make every FFmpeg probe and encoder shutdown bounded without pipe deadlocks, while preserving valid encoded tail packets during a generous normal flush window.

## Shared child deadline primitive

- [ ] Add a child-process regression that exceeds a pipe's capacity and another that ignores stdin/does not exit within a short test deadline.
- [ ] Introduce one polling wait primitive that returns the real exit status, kills and reaps on deadline, and also cleans up after `try_wait` errors.
- [ ] Keep production probe and encoder grace periods explicit and documented; use short injected durations only in tests.

## Probe stdout drainage

- [ ] Start a stdout reader immediately after probe spawn so verbose `-encoders` output cannot block the child before exit.
- [ ] Retain only a fixed maximum stdout size while continuing to drain excess bytes through EOF.
- [ ] On normal exit, join the reader and construct the probe output; on timeout/error, kill/reap first and then join so no reader thread or child remains.

## Encoder flush and drop

- [ ] Close stdin, wait for FFmpeg concurrently with the already-running stdout reader, and give normal H.264/HEVC/AV1 flush a documented grace period.
- [ ] If the grace period expires, kill/reap the child before joining the reader and return an explicit error that the encoded tail was lost.
- [ ] Apply the same bounded cleanup in `Drop` without surfacing errors, and avoid waiting again after `finish` already completed cleanup.
- [ ] Preserve normal nonzero-exit, reader-error, packet-drain, and input/output-cardinality reporting.

## Verification and handoff

- [ ] Run focused subprocess tests, the real available FFmpeg integration, fresh-cache capture Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild and open Clipline, verify normal startup, and record finding/commit evidence in the master ledger and `handoff.md`.
- [ ] Add a manual acceptance item only if forced termination of a genuinely wedged hardware encoder cannot be represented by the helper subprocess regression.
