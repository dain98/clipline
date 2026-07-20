# Continuous WASAPI Clock Servo

**Goal:** Keep WASAPI content aligned to WGC's shared QPC timeline for the lifetime of the recorder
without packet-edge crackle, progressive lead, or repeated discontinuous gap correction.

## Evidence

The nominal-cadence build produced `clip_1784581736.mp4`. Its video is exactly 30.000 seconds, but
Output Audio ends at 29.618 seconds and Microphone ends at 29.598 seconds. The user hears the same
lead in Clipline and VLC, so the offset is encoded.

The source displayed in the capture is a synchronized YouTube osu! video. Cross-correlating its
captured audio spectral onsets with temporal changes in the captured gameplay region independently
places audio about 350--400 ms before the matching frames. The strongest whole-section correlation
is -366.7 ms, and separate 6--14 s and 12--20 s windows both peak near -367 ms. That agrees with the
missing audio tail and rules out an endpoint-only or player-only problem.

Microsoft documents `Direct3D11CaptureFrame.SystemRelativeTime` as the QPC time when the compositor
rendered the frame and WASAPI `IAudioCaptureClient::GetBuffer` QPC position as the 100 ns QPC time of
the packet's first audio frame. Those timestamps are intended to synchronize media. Appending every
device packet at nominal 48 kHz after only one QPC anchor lets the device sample clock diverge from
the compositor clock over the recorder's full uptime; a 30-second replay then selects earlier audio
content and omits the corresponding tail.

## Implementation

- [ ] Add a neutral regression where fixed-size packets span a different QPC interval and require
      cumulative output duration to follow QPC rather than input sample count.
- [ ] Add waveform-boundary coverage proving clock correction interpolates continuously instead of
      inserting packet-sized silence, trimming whole packets, or forcing discontinuous endpoints.
- [ ] Replace nominal-only continuous placement with a one-chunk-lookahead clock servo. Derive each
      chunk's cumulative target sample frontier from the following valid QPC timestamp and resample
      smoothly to that frontier.
- [ ] Retain the hard pending-real-audio synthesis frontier: normal polls cannot overtake a held
      packet, actual device-delivery idle flushes after 100 ms, and terminal drain flushes
      immediately at nominal duration.
- [ ] Preserve explicit timestamp-error fallback, data-discontinuity fade/re-anchor, bounded idle
      resume, Opus framing, and finite memory.
- [ ] Run focused capture tests, real WGC/WASAPI shared-clock tests, workspace tests, warning-denied
      workspace Clippy, and clean-cache capture/app Clippy.
- [ ] Update `handoff.md`, rebuild, and launch Clipline.

## Manual retest

1. Restart Clipline, let the replay buffer run for at least two minutes, then capture the same kind
   of synchronized audiovisual source for a 30-second replay.
2. Compare simultaneous sound/frame events near both ends in Clipline and VLC; neither fixed lead
   nor growing drift should remain.
3. Confirm both audio tracks reach the video endpoint within normal Opus/polling headroom, with no
   periodic crackle, stutter, startup transient, or opening-audio repeat at EOF.
