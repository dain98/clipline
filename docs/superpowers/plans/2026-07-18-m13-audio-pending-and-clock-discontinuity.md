# M-13 Audio Pending Budget and Clock Discontinuity Plan

**Goal:** Keep every unsealed encoded payload inside the replay budget, stop cleanly when an encoder fails to close a GOP, and preserve monotonic source-aligned audio timestamps across bounded large-gap recovery.

## Combined pending limits

- [ ] Add failing pipeline regressions proving encoded audio counts toward the unsealed GOP byte budget and an initial-keyframe wait cannot hide audio growth.
- [ ] Track pending audio payload bytes alongside video/pre-keyframe bytes, updating the reservation when lead-in or sealed packets are drained.
- [ ] Add a deterministic maximum pending GOP duration and fail the recording pipeline with an explicit encoder/keyframe error instead of retaining an arbitrarily long segment.
- [ ] Apply byte and duration validation after audio polling and encoder output so a keyframe produced for the current capture frame gets the opportunity to seal the prior GOP.

## Audio timestamp discontinuities

- [ ] Add a failing PCM regression in which a one-hour source timestamp jump resumes near that source time rather than remaining one hour minus five seconds behind.
- [ ] Represent output-grid discontinuities as absolute sample-position anchors while retaining the five-second silence-allocation cap.
- [ ] Select the latest reached anchor when emitting each 20 ms PCM frame, clamp re-anchors to the prior monotonic grid, and keep ordinary gaps/jitter/overlap behavior unchanged.
- [ ] Cover the first resumed frame and following packet cadence after a discontinuity, including chunks split around the anchor.

## Verification and handoff

- [ ] Run focused pipeline and PCM tests, fresh-cache capture Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild and open Clipline, verify normal library startup, and record the finding/commit evidence in the master ledger and `handoff.md`.
- [ ] Add a manual acceptance item only if a real audio-device clock discontinuity cannot be fully represented by the deterministic assembler fixtures.
