# M-12 Codec Framing and Keyframe Truth Plan

**Goal:** Build one MP4 video sample per encoded picture, mark sync samples from the encoded bitstream, and fail rather than invent timing when encoder input/output cardinality diverges.

## H.264 and HEVC picture boundaries

- [ ] Classify completed Annex-B NALs as AUD, other non-VCL, first slice of a picture, or continuation slice using the H.264 `first_mb_in_slice` and HEVC `first_slice_segment_in_pic_flag` fields.
- [ ] Keep all continuation slices in the current access unit; close an existing picture only at AUD or a later first-slice NAL.
- [ ] Carry parameter/SEI NALs after a picture forward to the next picture boundary without losing parameter extraction or three/four-byte start-code streaming.
- [ ] Treat a truncated slice header conservatively as a first slice so malformed output cannot merge an unbounded run of pictures.

## AV1 sync classification

- [ ] Extend the bounded low-overhead OBU walker to expose payloads and parse the frame/frame-header OBU's actual `frame_type` (including reduced-still-picture sequence headers).
- [ ] Mark only encoded AV1 key frames as sync; scene cuts and backend GOP variation must not depend on `frame_index % configured_gop`.
- [ ] Surface malformed/missing AV1 frame-header metadata as a reader error while continuing to drain child stdout to avoid a subprocess pipe deadlock.

## Timestamp cardinality

- [ ] Remove synthesized output timestamps: every emitted access unit must consume one queued input PTS or return an encoder error.
- [ ] After stdout/child completion, reject any remaining input PTS as a dropped-output mismatch.
- [ ] Preserve FIFO timing with B-frames disabled and keep normal encode/drain APIs unchanged to callers.

## Tests and verification

- [ ] Add H.264 and HEVC fixtures with multi-slice pictures, AUDs, and parameter NALs between pictures.
- [ ] Add AV1 key/inter/reduced-still/malformed frame-header fixtures and remove position-based expectations.
- [ ] Add encoder-state tests for extra-output and missing-output PTS mismatches.
- [ ] Run focused capture tests, fresh-cache capture Clippy, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild/open Clipline and update `handoff.md`, the master ledger, and the manual checklist only if a real-backend scenario remains unautomated.
