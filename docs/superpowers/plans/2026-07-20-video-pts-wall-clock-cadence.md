# Wall-Clock-Bounded Video Cadence

**Goal:** Prevent semi-static display capture from advancing video PTS faster than real time while
retaining fixed-cadence duplicate frames and responsive stop/save handling.

## Evidence

Two replay saves taken 737.28 wall-clock seconds apart showed the video frontier advancing 741.02
seconds (`1.00507x`) while audio advanced 737.44 seconds (`1.00021x`). A raw five-minute WASAPI
probe measured both MOTU endpoints at only +32 ppm, and the production audio sources stayed within
one Opus packet of wall time. The apparent audio lead and missing audio tail are therefore caused
by video PTS inflation, not audio clock drift.

`CadencedCapture` advances timeout duplicates on a synthetic `next_pts += 1/fps` grid. When the
inner source returns early, the timeout handler still emits a full cadence step and resets
`last_emit_wall` to the current instant. Stale-frame retry timeouts make this path repeat on
semi-static desktop content, paying out more PTS than elapsed wall time. Games generally mask the
bug because accepted real frames regularly re-anchor the cadence to their QPC timestamps.

## Implementation

- [ ] Add a regression with repeatedly premature timeout returns and require emitted video PTS to
      remain bounded by elapsed wall time rather than advancing once per call.
- [ ] Make premature timeouts remain timeouts until the current wall-clock cadence deadline; they
      must not emit a duplicate or reset the cadence anchor.
- [ ] Preserve catch-up across genuine missed wall-clock intervals, latest-texture reuse after a
      stale frame, monotonic PTS, real-frame re-anchoring, and prompt command-loop yields.
- [ ] Remove the rejected audio-clock diagnosis from the current handoff and document direct video
      and audio frontier measurements plus the corrected root cause.
- [ ] Run focused cadence tests, workspace tests, warning-denied workspace Clippy, rebuild, and
      launch Clipline.

## Manual retest

1. Leave Clipline recording an idle or semi-static desktop for at least ten minutes, then save a
   30-second replay.
2. Verify both audio tracks reach the video tail within roughly one 20 ms Opus frame and that a
   simultaneous audio/visual cue stays synchronized in VLC and Clipline.
3. Save multiple replays, then test a moving 60 fps game to confirm smooth video, no audio crackle,
   and no pending-GOP/keyframe regression.
