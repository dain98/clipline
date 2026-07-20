# Replay Audio-Origin Save Fix

**Goal:** Save a replay whose selected first GOP begins inside an indivisible Opus packet, without
moving later audio or weakening the MP4 writer's negative-timestamp validation.

## Root cause

Replay saving rebases the MP4 timeline to the first selected video GOP. Audio packets are assigned
to GOPs by their end timestamp, so a packet that begins before a GOP keyframe and ends after it is
correctly retained for continuity during full-session recording. When that later GOP becomes the
first GOP of a replay, however, its straddling packet precedes the new replay origin and the MP4
writer rejects the negative relative timestamp.

The full-session startup fix only filters against the recording's first video timestamp. It cannot
filter every GOP boundary because full-session audio must remain continuous across those boundaries.

## Implementation

- [ ] Add a deterministic replay test whose selected first keyframe is 10 ms into a 20 ms Opus
      packet and prove the unmodified save fails with the negative-origin error.
- [ ] Before replay muxing, remove complete audio samples from only the first selected segment when
      their start timestamp precedes the replay video origin, preserving the existing 1 ns tolerance.
- [ ] Advance each affected track's start timestamp by the exact durations of removed samples and
      keep delayed, gapped, and later-segment audio unchanged.
- [ ] Keep the MP4 writer's negative-timestamp validation intact.
- [ ] Run focused capture tests, workspace tests, warning-denied workspace Clippy, and clean-cache
      capture Clippy.
- [ ] Update the combined audit ledger and handoff with commit evidence and the manual replay retest.

## Manual retest

1. Leave replay capture running with system or microphone audio enabled for longer than one GOP.
2. Save a replay whose window starts after the recording's initial GOP.
3. Confirm no `media sample timestamp precedes recording origin` warning appears.
4. Confirm the replay appears in Library, begins on video, and its audio starts cleanly and remains
   synchronized.
