# WASAPI Delivery Headroom

**Goal:** Eliminate recurring encoded crackles caused by normal WASAPI delivery latency without
shortening the audio tail or relaxing synchronization guarantees.

## Evidence and root cause

The reported 30-second replay has continuous Opus packet timestamps, but decoded Output Audio has a
40 dB, 10 ms hole near 28.27 seconds and Microphone has a 21 dB hole near 23.64 seconds. Recorder
diagnostics show each source still invokes `wasapi_late_audio_reanchored` roughly every 2--3 seconds,
including suppressed events at those positions.

Clipline synthesizes silence only 20 ms behind the current video timestamp. Real WASAPI buffers on
this machine periodically arrive another 10--11 ms later, after that silence has already been
encoded. The recovery preserves the late live samples, preventing permanent lockout, but it cannot
replace the committed silence; the resulting short hole and hard boundary are audible as crackle.

## Implementation

- [ ] Add deterministic coverage requiring ordinary audio polls to leave 30 ms of device-delivery
      headroom rather than 20 ms.
- [ ] Add an `AudioSource` finish-drain contract and a recorder regression proving terminal audio
      available only during that drain is retained through the final video boundary.
- [ ] Have WASAPI terminal drain wait one Opus frame for outstanding device buffers, drain without
      synthesizing new silence, and return only complete packets ending within the video timeline.
- [ ] Keep monitor drains non-synthesizing and preserve timestamp, Opus framing, bounded queue, and
      replay/full-session mux behavior.
- [ ] Confirm normal hardware capture no longer emits recurring 10--11 ms late-reanchor events and
      remains inside the 45 ms shared-clock drift contract.
- [ ] Run focused capture tests, the real shared-clock test, workspace tests, warning-denied
      workspace Clippy, and clean-cache capture Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Record at least one minute with Output Audio and Microphone active, then save a 30-second replay.
2. Play the replay from beginning to end with both tracks selected and confirm no repeated crackles,
   holes, or stutters.
3. Play each track alone to confirm both are independently clean.
4. Stop a full-session recording while sound is active and confirm its final second remains audible
   and synchronized.
5. Inspect `clipline.log`; ordinary recording should not continuously accumulate suppressed
   `wasapi_late_audio_reanchored` events.
