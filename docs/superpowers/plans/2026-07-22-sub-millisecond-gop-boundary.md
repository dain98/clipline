# Sub-Millisecond GOP-Boundary Finalization Fix

**Goal:** Finalize full-session recordings when two encoded video frames arrive less than 100 us
apart, without weakening the MP4 writer's protection against real timestamp regressions.

## Root cause

The recorder derives each non-terminal sample duration from adjacent encoded PTS values, but floors
every interval at 100 us. A real interval between one 90 kHz tick and 100 us is therefore lengthened
inside its GOP. At the next keyframe, the following GOP still starts from its absolute timestamp, so
the accumulated frontier can be several ticks later than that start. The existing one-tick rounding
tolerance cannot cover this non-rounding inflation; the observed failure moved video track 0 from
4,051,257 back to 4,051,255.

## Implementation

- [ ] Add a deterministic full-session regression with a seven-tick (about 78 us) adjacent-frame
      interval that the old 100 us floor inflates by exactly two ticks.
- [ ] Floor positive video sample durations at one tick in the configured video timescale, which is
      the actual minimum the MP4 writer can represent.
- [ ] Keep the existing one-tick boundary-rounding tolerance and rejection of larger real timestamp
      regressions unchanged.
- [ ] Run focused capture tests, workspace tests, warning-denied workspace Clippy, and clean-cache
      capture Clippy.
- [ ] Update the handoff with the reported symptom, root cause, and verification.

## Manual retest

1. Record and stop several full-session recordings, including a variable-refresh-rate game.
2. Confirm no `decode time cannot move backward` warning appears at stop.
3. Confirm each session is published as `.mp4`, appears in Library, and plays through GOP boundaries.
4. Confirm the preserved `session_1784705074.mp4.recording` is retained until explicitly recovered.
