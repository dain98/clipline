# WASAPI Discontinuity Fade

**Goal:** Prevent a loud pop at the beginning of a recording, or after a device discontinuity,
without changing audio timing or masking sustained audio.

## Evidence and root cause

The reported 188-second session has two structurally continuous Opus tracks. Its Output Audio track
begins at 11.687 ms with a non-zero broadband transient: the first 20 ms peaks around -24.5 dBFS,
then decays by roughly 30 dB over the next 60 ms. The microphone track does not contain the same
strong onset. Recorder diagnostics place `wasapi_data_discontinuity` for both sources at the exact
05:17:48 recording start.

Clipline currently accepts a WASAPI packet marked `DATA_DISCONTINUITY` at full amplitude. When that
packet starts away from zero, the abrupt boundary is encoded into Opus and is audible in any player.
Timestamp gap handling keeps the track aligned but does not smooth its sample boundary.

## Implementation

- [ ] Add neutral deterministic coverage for a bounded stereo fade that starts at zero, reaches
      unity after 40 ms, continues across capture-buffer boundaries, and does not consume its ramp
      while the device reports digital silence.
- [ ] Arm the fade when WASAPI capture starts and re-arm it before processing every packet marked
      `DATA_DISCONTINUITY`.
- [ ] Apply the fade after channel conversion, resampling, and user gain so timing, sample count,
      audio levels, and Opus frame boundaries stay unchanged.
- [ ] Keep timestamp-error, silence, gap-fill, late-buffer recovery, and diagnostics behavior
      unchanged.
- [ ] Run focused PCM and Windows audio tests, the real shared-clock capture test, workspace tests,
      warning-denied workspace Clippy, and clean-cache capture Clippy.
- [ ] Update the combined audit ledger and handoff, then rebuild and open Clipline.

## Manual retest

1. Start a new full-session recording with Output Audio and Microphone enabled.
2. Stop after at least ten seconds and play it from exactly 0:00 with both tracks selected.
3. Confirm the loud startup pop is gone and ordinary audio reaches normal volume immediately after
   the brief 40 ms ramp.
4. Seek back to 0:00 and replay the opening several times to confirm the result is repeatable.
5. Keep recording for several minutes and confirm later audio remains continuous and synchronized.
