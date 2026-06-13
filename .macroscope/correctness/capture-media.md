---
include:
  - "crates/clipline-capture/src/**"
  - "crates/clipline-mp4/src/**"
  - "crates/clipline-buffer/src/**"
---

Capture, replay, and MP4 code are correctness-critical.

Review for:

- One shared relative clock across capture, encode, audio, and muxing.
- Stamp-derived PTS rather than assumed fixed frame cadence.
- Audio gap fill, dropped-frame behavior, and A/V sync under irregular frame delivery.
- GOP-aligned replay saves, smart overlap handling, multi-track preservation, and clean shutdown.
- Correct H.264 Annex B to AVCC conversion, no-B-frame assumptions, Opus timing, sample tables, finalization, and keyframe-aligned trim/export.
- Safe COM/D3D11/MFT/WASAPI lifetimes, HRESULT propagation, thread ownership, and small, contained `unsafe` blocks behind safe wrappers.

Treat playable output that is corrupt, non-seekable, or desynchronized as a real bug.
