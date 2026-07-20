# WASAPI Delivery Headroom

**Goal:** Eliminate recurring encoded crackles caused by normal WASAPI delivery latency without
shortening the audio tail or relaxing synchronization guarantees.

## Evidence and root cause

The reported 30-second replay has continuous Opus packet timestamps, but decoded Output Audio has a
40 dB, 10 ms hole near 28.27 seconds and Microphone has a 21 dB hole near 23.64 seconds. Recorder
diagnostics show each source still invokes `wasapi_late_audio_reanchored` roughly every 2--3 seconds,
including suppressed events at those positions.

Clipline originally synthesized silence only 20 ms behind the current video timestamp. The first
implementation raised that allowance to 30 ms, and a second live experiment raised it to 60 ms.
Neither changed the recovery cadence: the same endpoints still reported a 10--12 ms correction
about every three seconds (`suppressed_since_last=9` to `11` per 30-second diagnostic window).
The endpoints can therefore remain quiescent longer than a sensible fixed allowance. When they
resume, preserving the complete late chunk is correct, but joining its arbitrary first sample
directly to committed digital silence creates the audible edge.

## Implementation

- [ ] Keep 30 ms of normal active-delivery headroom; do not add unbounded latency in an attempt to
      outwait quiescent endpoints.
- [ ] Add a deterministic assembler regression requiring a five-millisecond fade only at the
      synthesized-silence-to-live recovery boundary, while retaining every live sample and normal
      amplitude after the fade.
- [ ] Add an `AudioSource` finish-drain contract and a recorder regression proving terminal audio
      available only during that drain is retained through the final video boundary.
- [ ] Have WASAPI terminal drain wait three Opus frames for outstanding device buffers, drain
      without synthesizing new silence, and return only complete packets ending within the video
      timeline.
- [ ] Keep monitor drains non-synthesizing and preserve timestamp, Opus framing, bounded queue, and
      replay/full-session mux behavior.
- [ ] Confirm normal hardware capture remains inside the 45 ms shared-clock drift contract. Late
      re-anchor diagnostics may remain for quiescent endpoints, but their recovery boundaries must
      be smoothed rather than hard-edged.
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
5. Inspect `clipline.log`; `wasapi_late_audio_reanchored` may occur when an endpoint resumes, but it
   must not correspond to an audible click or crackle.
