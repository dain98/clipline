# Replay Repeat Save and Review EOF

**Goal:** Keep short review-audio sidecars silent at video EOF and make every replay save reject
audio samples that precede its selected video origin.

## Evidence

The first clean capture, `clip_1784529665.mp4`, has a 30.000-second video track. Its Output Audio
ends at 29.655 seconds and Microphone ends at 29.635 seconds; the extracted audio-only sidecars
preserve those shorter endpoints. While video is still playing, the 500 ms review sync sees the
ended audio element paused and calls `play()`, which restarts an ended HTML media element from zero.
That explains why only Clipline plays a fraction of the clip's opening audio at the end and VLC does
not.

The following replay save fails with `media sample timestamp precedes recording origin`. Replay
materialization removes pre-origin samples only from the first selected segment. Continuously
delivered WASAPI audio can trail video and be sealed into a later GOP segment, so a later selected
segment may also contain samples older than the replay video origin.

## Implementation

- [ ] Add a pure player regression showing that a sidecar shorter than the still-playing video is
      exhausted and must not be restarted, while a seek back inside its duration can play it again.
- [ ] Pass sidecar duration/ended state into the pure synchronization policy and keep exhausted
      sidecars paused until video seeks back into their range.
- [ ] Add a capture regression with pre-origin audio in a later selected segment.
- [ ] Apply replay-origin audio filtering to every selected segment before MP4 fragments are built.
- [ ] Run focused tests, workspace tests, warning-denied workspace Clippy, and clean-cache Clippy for
      both changed crates.
- [ ] Update `handoff.md`, rebuild, and open Clipline.

## Manual retest

1. Play the first saved replay through 30.000 seconds in Clipline; no opening audio may repeat at
   the end. Seek back from EOF and confirm selected audio resumes normally.
2. Save at least two replays from the same running buffer. Both must finalize and appear in Library.
3. Play the second replay in Clipline and VLC and confirm clean audio from start through EOF.
