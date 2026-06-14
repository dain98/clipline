# Rec.709 Recording Color

## Goal

Fix dark or oversaturated Clipline recordings by removing implicit color-range and
color-space guessing from the capture, encode, and MP4 muxing pipeline.

## Scope

- Treat SDR WGC BGRA frames as full-range RGB Rec.709 input.
- Configure the D3D11 video processor to produce limited-range NV12 Rec.709 output.
- Attach matching Rec.709 limited-range metadata to Media Foundation H.264 input/output media
  types.
- Pass matching `ffmpeg` raw-input and output color flags for the FFmpeg subprocess tier.
- Emit an MP4 `colr`/`nclx` sample-entry box so finalized files advertise Rec.709 limited range.
- Add unit coverage for FFmpeg args and muxed color metadata, then verify a real smoke recording
  with `ffprobe`.

## Verification

- `cargo fmt`
- `cargo test -p clipline-capture -p clipline-mp4`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `record_smoke -- --seconds 3 --out target\codex-test\color_smoke.mp4`
- `ffprobe` reports `color_range=tv`, `color_space=bt709`, `color_transfer=bt709`, and
  `color_primaries=bt709`
