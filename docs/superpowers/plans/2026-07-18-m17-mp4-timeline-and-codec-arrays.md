# M-17 MP4 Timeline and Codec Array Plan

**Goal:** Preserve valid leading and internal per-track timing gaps through fragmented recording, finalization, trim, and remux, while retaining every H.264 and HEVC parameter set required by the copied samples.

## Explicit track timeline

- [ ] Add writer regressions proving a track may start after movie time zero and resume after an internal gap, with each fragmented `tfdt` carrying the requested decode time.
- [ ] Add a checked public writer API for advancing an individual track to an absolute decode time; reject unknown tracks, backward movement, arithmetic overflow, and non-empty writes with zero-duration samples.
- [ ] Record contiguous media runs separately from presentation time so empty runs do not silently collapse the track timeline.
- [ ] Emit versioned `elst` entries for leading/internal empty time and media runs, keep `mdhd` duration equal to encoded media duration, and make `tkhd`/`mvhd` cover the real presentation end.
- [ ] Use integer rescaling for all persisted timing boundaries and cover values above the version-zero signed/unsigned limits.

## Parse, trim, and remux gaps

- [ ] Parse supported version 0/1 edit lists and map contiguous sample-table decode times back to absolute presentation ticks without floating-point comparisons.
- [ ] Reject malformed, negative, overlapping, mid-sample, or rate-adjusted edit-list mappings rather than silently changing timing.
- [ ] Preserve every leading/internal gap when selecting audio tracks or copying full tracks, including file-backed and mixed-audio variants.
- [ ] Align trims on integer video ticks, select other-track samples on exact rational boundaries, rebase all retained starts to the aligned video origin, and preserve later gaps rather than stamping the first surviving packet at zero.
- [ ] Preserve Opus `dOps` pre-skip and keep the mid-stream replay pre-skip policy unchanged.

## Capture pipeline timing

- [ ] Retain each segment audio track's first packet PTS in memory and disk replay metadata.
- [ ] Set per-track fragment decode times from segment/video/audio presentation stamps for replay saves and full-session recording, including audio-empty early fragments and later discontinuities.
- [ ] Add pipeline regressions for delayed audio onset, an empty audio segment, and a later audio gap; assert both fragmented and finalized timing remain continuous with the intended silence.

## Complete H.264/HEVC parameter arrays

- [ ] Represent H.264 SPS/PPS and HEVC VPS/SPS/PPS as non-empty collections while keeping singleton constructors ergonomic for encoder call sites.
- [ ] Emit every collection member in `avcC`/`hvcC`, validate count and NAL-length field limits, and derive summary fields from the primary SPS.
- [ ] Parse every supported configuration-array member during trim/remux and reject missing required arrays or unsupported count/size boundaries.
- [ ] Add round-trip fixtures with multiple parameter-set IDs and prove trim/remux output retains all sets byte-for-byte.

## Verification and handoff

- [ ] Run focused MP4, buffer, and capture tests plus fresh-cache Clippy for all changed crates.
- [ ] Run CI-mode workspace tests and workspace Clippy with warnings denied.
- [ ] Rebuild and open Clipline, verify normal startup, and record finding/commit evidence in the master ledger and `handoff.md`.
- [ ] Add a manual playback item only for behavior that WebView2/FFprobe fixtures cannot exercise deterministically.
