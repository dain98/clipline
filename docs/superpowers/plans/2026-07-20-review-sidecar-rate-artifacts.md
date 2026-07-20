# Review Sidecar Rate Artifacts

**Goal:** Eliminate crackling introduced by continuous WebView playback-rate correction while
preserving bounded review audio/video synchronization.

## Evidence and root cause

The fresh 30-second replay `clip_1784527236.mp4` has continuous Opus packet timelines, no impulses
at its two-second GOP boundaries, and modest decoded sample transitions. The two generated review
sidecars are stream copies: their encoded-packet SHA-256 hashes exactly match the corresponding
source tracks (`1417a14a...` and `bf766953...`). Replay materialization and sidecar extraction
therefore do not introduce the reported crackle.

During playback, each hidden audio element is checked every 500 ms. More than 25 ms of ordinary
clock difference changes its playback rate to 0.95x or 1.05x, then back to the requested rate when
inside the deadband. WebView must repeatedly time-stretch two independent Opus decoders, which
introduces audible artifacts even though their source packets are intact.

## Implementation

- [ ] Change the pure sidecar sync decision so ordinary playing drift keeps the video's requested
      playback rate and does not seek.
- [ ] Preserve forced seeks, paused alignment, invalid sidecar recovery, and the existing 500 ms
      gross-discontinuity seek threshold.
- [ ] Update the deterministic player regression to prove ahead/behind sidecars remain at the
      requested rate while gross drift still seeks.
- [ ] Keep sidecar preparation, selected-track routing, volume/mute behavior, playback lifecycle,
      and direct single-track playback unchanged.
- [ ] Run focused player tests, workspace tests, warning-denied workspace Clippy, and clean-cache
      app Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Play `clip_1784527236.mp4` from beginning to end with Output Audio and Microphone selected.
2. Confirm the throughout-the-clip crackle is gone and the two tracks remain acceptably synced.
3. Repeat with Output only and Microphone only.
4. Seek while playing and paused, then change playback speed; audio must follow the requested
   position and speed without a repeated fragment or time-stretch crackle.
