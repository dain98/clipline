# Audio Cadence and MFT Keyframe Classification

**Goal:** Preserve real-time audio duration during continuous WASAPI delivery and prevent valid
H.264 IDR pictures from being rejected when a hardware MFT omits its keyframe metadata flag.

## Evidence

The user confirmed that sound leads picture. The latest successful replay,
`clip_1784530928.mp4`, contains 30.100 seconds of video but only 29.700 seconds of Output Audio and
29.680 seconds of Microphone audio. The current timestamp aligner deliberately converts each
512-pair MOTU packet to the roughly 510-pair interval reported by the next QPC timestamp. That
shortens continuous real PCM by about 0.4 seconds over a 30-second clip, matching both the measured
track deficit and the perceived lead.

The packet-idleness fix already prevents poll-time silence from overtaking continuously arriving
PCM. Continuous device packets therefore need to retain their sample-count cadence. QPC remains
necessary to establish the initial shared-clock anchor and to re-anchor after a genuine idle gap,
but it must not resample every packet.

After the recorder had run for roughly 53 minutes, it reported that no keyframe had arrived for
10 seconds. The active Automatic path on this AMD system can use the H.264 hardware MFT. That MFT
currently trusts only `MFSampleExtension_CleanPoint`; FFmpeg paths instead inspect the encoded NAL
units. A valid H.264 IDR without the optional CleanPoint flag is therefore misclassified as a
non-keyframe until the pending-GOP safety limit stops recording. MFT output should recognize either
signal while retaining the bounded safety check for a genuinely stalled encoder.

## Implementation

- [ ] Add a neutral PCM regression proving that 512-pair packets remain 512 pairs when their QPC
      timestamps advance by about 510 pairs, with no accumulated timeline shortening.
- [ ] Replace one-packet timestamp interpolation with a delivery timeline that uses QPC for the
      first packet and the first packet after genuine idle synthesis, while appending continuously
      arriving packets at nominal sample cadence.
- [ ] Preserve explicit quiet-endpoint silence synthesis, discontinuity fades, bounded late-resume
      correction, terminal draining, and device-level metering.
- [ ] Add a neutral H.264 regression that classifies an IDR access unit as a keyframe independently
      of container/MFT metadata.
- [ ] Make the MFT packet path accept either CleanPoint metadata or an encoded IDR NAL as the
      keyframe signal; leave the 10-second pending-GOP bound intact for real stalls.
- [ ] Run focused tests, real Windows capture/encode tests, workspace tests, warning-denied
      workspace Clippy, and clean-cache capture/app Clippy.
- [ ] Update `handoff.md`, rebuild, and open Clipline for a sync and long-running recorder retest.

## Manual retest

1. Record at least 30 seconds with an easy-to-see action that makes a simultaneous sound, then
   save a replay and compare sync near both its beginning and end in Clipline and VLC.
2. Confirm the audio tracks reach approximately the same endpoint as the video and do not crackle,
   stutter, or replay their opening at EOF.
3. Leave the replay buffer running for at least 15 minutes, save more than once, and confirm no
   pending-GOP/no-keyframe error appears.
