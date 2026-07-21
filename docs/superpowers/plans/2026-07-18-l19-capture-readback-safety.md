# L-19 Capture Readback Safety Plan

> **Finding:** L-19 — safe WASAPI/NV12 helpers depend on unchecked raw alignment, pointer, pitch,
> and arithmetic assumptions.

## Goal

Make every raw Windows capture buffer boundary explicit and safe: decode audio from alignment-one
bytes, validate mapped NV12 layout before pointer arithmetic, and guarantee D3D unmapping and
WASAPI buffer release across validation failures without changing sample values or packed output.

## TDD sequence

- [ ] Add pure WASAPI decoder tests with deliberately misaligned float32/PCM16/PCM32 byte slices,
  PCM24 sign-extension edges, and truncated buffers; run red while typed raw slices remain.
- [ ] Add pure NV12 layout tests for valid padding, short pitch, odd/zero dimensions, packed-size
  overflow, mapped-span overflow, and exact Y/UV offsets; run red before validation exists.
- [ ] Decode every audio format with fixed-size little-endian byte chunks; checked frame/channel/
  byte arithmetic and a non-null check are the only raw-pointer boundary.
- [ ] Ensure a non-silent WASAPI packet is released even when pointer/size/decoding validation
  fails, while silent packets continue to accept the API's null data pointer.
- [ ] Validate NV12 width/height, row pitch, packed length, UV offset, and maximum pointer span
  before allocation or pointer arithmetic.
- [ ] Add one scoped D3D map guard so every successful `Map` is paired with `Unmap` on success,
  validation error, or unwind; use it for both NV12 and BGRA readback.
- [ ] Run focused capture tests, real device readback where available, fresh-cache capture Clippy,
  CI-mode workspace tests, and warning-denied workspace Clippy.
- [ ] Rebuild/open Clipline and verify normal Library startup; retain the existing real Windows
  capture lifecycle scenario for device-level acceptance.
- [ ] Update `handoff.md`, the master ledger, and the cumulative checklist.

## Invariants

- [ ] No `u8` device pointer is cast into a typed Rust slice.
- [ ] A null non-silent WASAPI buffer returns a capture error without skipping `ReleaseBuffer`.
- [ ] Float/PCM output is numerically identical to little-endian Windows sample semantics.
- [ ] No NV12 allocation, pointer `add`, or slice construction occurs before checked layout
  validation.
- [ ] Row pitch covers one visible row; dimensions are nonzero/even; every multiplication/addition
  and the maximum mapped span is representable by Rust pointer offsets.
- [ ] Every successful D3D `Map` is unconditionally paired with exactly one `Unmap`.

## Commits

- `docs(plan): define capture readback safety boundary`
- `fix(capture): validate raw audio and texture reads`
- `docs(audit): close capture readback safety finding`
