# WASAPI Clock Servo

**Goal:** Remove periodic encoded crackle by replacing repeated packet-sized WASAPI timeline steps
with continuous, evidence-driven clock alignment.

## Evidence

The replay `clip_1784527236.mp4` crackles in VLC, confirming the artifact is encoded. Both tracks
contain sharp local level holes, and recorder diagnostics report recurring 10--11 ms late recovery
for each source. The configured output (`Out 1-2`) and microphone (`In 1-2`) are the same MOTU M
Series USB interface, both configured for 48 kHz float stereo. The existing five-millisecond fade
therefore masks, but does not remove, a repeated timeline correction close to one device packet.

## Implementation

- [ ] Extend the typed, rate-limited late-recovery diagnostic with device-chunk duration and total
      accumulated correction; cover formatting and assembler accounting deterministically.
- [ ] Run the real recorder long enough to determine whether corrections accumulate one complete
      packet at a time or reflect a stable sample-clock ratio.
- [ ] Add a neutral failing fixture reproducing the observed sequence and requiring continuous PCM
      without packet-sized holes or repeated recovery fades.
- [ ] Replace stepwise recovery with bounded continuous alignment appropriate to the measured
      failure mode. Preserve monotonic timestamps, every real source interval not already committed,
      gap bounding, Opus framing, and finite memory.
- [ ] Retain startup/data-discontinuity protection, but remove the late-recovery fade once the
      underlying repeated step is eliminated.
- [ ] Run focused capture tests, the real shared-clock hardware test, workspace tests,
      warning-denied workspace Clippy, and clean-cache capture Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Record at least one minute with continuous game output and microphone activity, then save a
   30-second replay.
2. Play both tracks and each track alone in Clipline and VLC; no periodic crackle or short hole
   should occur.
3. Stop a full session while sound is active and confirm its final second remains present and synced.
4. Inspect `clipline.log`; normal capture must not repeatedly accumulate packet-sized clock steps.
