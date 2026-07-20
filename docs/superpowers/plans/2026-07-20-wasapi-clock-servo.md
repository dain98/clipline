# WASAPI Clock Servo

**Goal:** Remove periodic encoded crackle by replacing repeated packet-sized WASAPI timeline steps
with continuous, evidence-driven clock alignment.

## Evidence

The replay `clip_1784527236.mp4` crackles in VLC, confirming the artifact is encoded. Both tracks
contain sharp local level holes, and recorder diagnostics report recurring 10--11 ms late recovery
for each source. The configured output (`Out 1-2`) and microphone (`In 1-2`) are the same MOTU M
Series USB interface, both configured for 48 kHz float stereo. The existing five-millisecond fade
therefore masks, but does not remove, a repeated timeline correction close to one device packet.

Live telemetry confirmed the mechanism. Each MOTU chunk is 11 ms; microphone accumulated 128 ms
of correction over one 30-second diagnostic window, with ten suppressed events, and every individual
correction was one complete 10--11 ms chunk. Nominal frame-count duration is therefore accumulating
against the driver's QPC timestamps until the assembler takes a packet-sized step.

The first one-chunk-lookahead build exposed a second ordering requirement. PID 22688 still reached
162 ms of microphone correction and 158 ms of output correction in roughly 35 seconds because the
poll-time silence horizon advanced the assembler past the real chunk held for its following QPC
timestamp. The pending chunk must therefore be a hard synthesis frontier. Normal polling may not
synthesize beyond its start; if no following timestamp arrives, a separate bounded quiet-endpoint
grace flushes it at nominal length. Terminal drain flushes it immediately.

The hard-frontier build also proved that QPC timestamp age cannot identify a quiet endpoint. It
produced an exact 100 ms correction while both devices were still delivering packets, then resumed
the former step pattern. The MOTU's reported source timeline drifts behind the video clock even as
PCM arrives continuously. Quiet flushing must therefore use elapsed host time since the most recent
WASAPI packet, not the difference between a packet timestamp and the video/synthesis horizon.

## Implementation

- [ ] Extend the typed, rate-limited late-recovery diagnostic with device-chunk duration and total
      accumulated correction; cover formatting and assembler accounting deterministically.
- [ ] Run the real recorder long enough to determine whether corrections accumulate one complete
      packet at a time or reflect a stable sample-clock ratio. Confirmed: one complete packet at a
      time, accumulating continuously.
- [ ] Add a neutral failing fixture reproducing the observed sequence and requiring continuous PCM
      without packet-sized holes or repeated recovery fades.
- [ ] Hold one timestamped device chunk and interpolate it to the next chunk's QPC interval when
      that interval is close to nominal. Flush at nominal length after a bounded wait when no next
      chunk arrives, so quiet endpoints and stream finish remain finite.
- [ ] Cap poll-time silence synthesis at the start of a pending real chunk. Flush the pending chunk
      only after no WASAPI packet has arrived for the quiet-endpoint grace, then resume silence
      synthesis; cover both continuous-delivery protection and finite quiet flush with tests.
- [ ] Feed the timestamp-aligned samples to the existing assembler, eliminating packet-sized
      stepwise recovery while preserving monotonic timestamps, explicit gaps, Opus framing, and
      finite memory.
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
