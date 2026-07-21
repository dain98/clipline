# Review Audio Sidecar Drift Correction

**Goal:** Keep multi-track review audio synchronized without audible periodic stutters during
ordinary playback.

## Evidence and root cause

The reported 74-second MP4 has continuous 20 ms Opus packets and continuous decoded samples on
both Output Audio and Microphone at the reported 2.000-second stutter. Its default review selection
uses extracted audio sidecars because both tracks are enabled.

During playback, Clipline compares each sidecar clock with the video every 500 ms and hard-seeks
the audio element whenever drift exceeds 100 ms. Independent WebView media clocks can naturally
cross that threshold shortly after playback begins, turning harmless clock drift into an audible
skip or repeated fragment even though the source MP4 is intact.

## Implementation

- [ ] Add pure player regression coverage distinguishing ordinary playing drift from explicit
      seeks, invalid clocks, paused transport, and gross discontinuities.
- [ ] Keep the video clock authoritative, but correct ordinary drift with a bounded proportional
      sidecar playback-rate adjustment instead of assigning `currentTime`.
- [ ] Retain hard sidecar seeks for explicit user/video seeks, unusable sidecar clocks, paused
      alignment, and drift too large to converge smoothly.
- [ ] Ensure each sidecar returns to the video's requested playback rate inside the drift deadband.
- [ ] Run focused player and UI-contract tests, workspace tests, and warning-denied workspace
      Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Open `session_1784523792.mp4` in Clipline with both Output Audio and Microphone selected.
2. Play from the beginning through at least ten seconds and confirm the stutter at exactly two
   seconds is gone.
3. Let the clip play for at least one minute and confirm there are no periodic skips or repeats.
4. Seek several times while playing and paused; confirm both tracks snap to the new position and
   remain synchronized.
5. Toggle between Output only, Microphone only, both tracks, and mute; confirm playback stays
   responsive and audible selections remain correct.
