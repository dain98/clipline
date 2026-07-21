# Late WASAPI Audio Recovery

**Goal:** Prevent system-output and microphone capture from becoming permanently silent when real
WASAPI buffers arrive after Clipline has already advanced that track with synthesized silence.

## Evidence and root cause

The reported 989-second League session contains structurally complete video plus two continuous
49,458-packet Opus tracks. Output contains real audio only around 7.40--13.74 seconds and microphone
only during the opening seconds; every later packet decodes as digital silence. Clipline logged no
device-loss error.

Finite audio polls currently synthesize silence through one Opus frame before the current video
timestamp. If a real device buffer is delivered later than that allowance, the neutral PCM
assembler trims the interval already represented by silence and discards the buffer when it is
fully overlapped. The next video poll advances silence again, so a consistently delayed source can
remain behind forever. Partial overlap produces the reported stutter before full lockout.

## Implementation

- [ ] Add a deterministic neutral regression that advances and emits silence, then delivers a
      fully overlapped real chunk followed by its next sequential chunk. Prove both real chunks are
      currently discarded and the track stays silent.
- [ ] Track whether the assembler has advanced synthetically since the last real chunk.
- [ ] When a real finite-timestamp chunk is late specifically because of that synthetic advance,
      preserve the complete chunk at the assembler's current monotonic position and retain the
      resulting timestamp correction for subsequent source chunks.
- [ ] Keep existing overlap trimming for late chunks that were not overtaken by synthesized
      silence, and keep forward-gap, discontinuity, invalid-timestamp, frame-grid, and bounded
      allocation behavior unchanged.
- [ ] Return the applied correction to the Windows capture layer and emit a typed, rate-limited
      diagnostic containing the correction in milliseconds.
- [ ] Run neutral PCM tests, Windows audio tests, the real shared-clock capture test, all capture
      tests, workspace tests, warning-denied workspace Clippy, and clean-cache capture Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Record a full-session game for at least five minutes with output and microphone enabled.
2. Speak and keep game audio active near the beginning, middle, and end.
3. Confirm neither track stutters into permanent silence and both remain synchronized.
4. Inspect `clipline.log`; if `wasapi_late_audio_reanchored` appears, confirm it is rate-limited and
   recording remains audible after the event.
5. Save a replay during the same run and confirm its beginning and tail both contain synchronized
   audio.
