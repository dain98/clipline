# Software VM Encoder Fallback Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Let Clipline record on Windows virtual machines and systems that support WGC but expose no D3D11 video processor or hardware video encoder.

**Architecture:** Preserve the existing zero-copy hardware pipeline. Add a last-resort path that reads WGC BGRA textures into CPU memory, converts/crops/scales them to limited-range Rec.709 NV12 in neutral Rust, and pipes those frames to FFmpeg's LGPL-compatible `h264_mf` software Media Foundation encoder. Probe the encoder before offering it and select this path only for the existing `MfSoftware/H264` candidate.

**Tech Stack:** Rust, windows-rs D3D11 staging textures, FFmpeg subprocess Media Foundation encoder, WGC, existing Hybrid MP4 pipeline.

## Global Constraints

- Hardware MFT and hardware FFmpeg paths must remain unchanged and preferred.
- Keep all D3D11 calls and unsafe readback in `crates/clipline-capture/src/windows/`.
- Keep color conversion platform-neutral and deterministic so Ubuntu CI tests it.
- Convert full-range BGRA Rec.709 to limited-range NV12 Rec.709, matching the existing GPU path and MP4 metadata.
- Preserve crop, fixed output dimensions, even dimensions, frame timestamps, fixed GOPs, and disabled B-frames.
- Use FFmpeg as a separate LGPL process; do not link it and do not add GPL encoders.
- `h264_mf` must be test-encoded before being reported as usable.
- Malformed dimensions, crop rectangles, strides, or buffers must return errors rather than panic.
- Follow strict TDD: observe each new test fail for the intended reason before implementation.

---

### Task 1: Neutral BGRA-to-NV12 conversion

**Files:**
- Create: `crates/clipline-capture/src/cpu_video.rs`
- Modify: `crates/clipline-capture/src/lib.rs`

**Interfaces:**
- Produces a reusable CPU converter configured with input size, optional crop, and fixed even output size.
- Consumes packed or row-pitched BGRA bytes and returns contiguous NV12 bytes.

- [ ] Add failing tests for black/white limited-range levels, Rec.709 chroma, row-pitch padding, crop/scale mapping, and invalid input rejection.
- [ ] Run the focused tests and verify RED because the module/API does not exist.
- [ ] Implement nearest-neighbor luma scaling plus 2x2 averaged chroma with integer Rec.709 limited-range coefficients.
- [ ] Run the focused tests and verify GREEN.

### Task 2: Windows BGRA texture readback

**Files:**
- Modify: `crates/clipline-capture/src/windows/d3d11.rs`
- Modify: `crates/clipline-capture/src/windows/nv12.rs`

**Interfaces:**
- Produces a CPU-readable BGRA staging texture helper and a safe wrapper returning packed BGRA bytes plus dimensions.

- [ ] Add a failing WARP-backed test proving a BGRA texture can be copied, mapped, and packed without `ID3D11VideoDevice`.
- [ ] Run the focused test and verify RED because BGRA readback is missing.
- [ ] Implement staging allocation, `CopyResource`, row-pitch-aware `Map`, and guaranteed `Unmap` behind the Windows safe wrapper.
- [ ] Run the focused test on the VM and verify GREEN without self-skipping.

### Task 3: Probe and configure FFmpeg software H.264

**Files:**
- Modify: `crates/clipline-capture/src/ffmpeg.rs`
- Modify: `crates/clipline-capture/src/ffmpeg_encoder.rs`
- Test: existing module test blocks

**Interfaces:**
- Maps `MfSoftware/H264` to `h264_mf`.
- Forces software mode and applies bitrate/GOP/no-B-frame arguments while retaining NV12 input and Rec.709 metadata.

- [ ] Add failing parser/name/probe-policy and argument tests for `h264_mf`.
- [ ] Run the focused tests and verify RED.
- [ ] Add `h264_mf`, require its one-frame usability probe, and configure `-hw_encoding 0` plus target bitrate.
- [ ] Run the focused tests and verify GREEN.

### Task 4: CPU conversion in the FFmpeg encoder and service selection

**Files:**
- Modify: `crates/clipline-capture/src/ffmpeg_encoder.rs`
- Modify: `apps/clipline-app/src/service.rs`
- Test: both existing module test blocks

**Interfaces:**
- Adds a Windows FFmpeg constructor for GPU BGRA input with CPU conversion.
- Selects it only for `EncoderApi::Ffmpeg + EncoderBackend::MfSoftware`; all other candidates retain GPU conversion.

- [ ] Add failing tests for conversion-path selection and software candidate construction policy.
- [ ] Run the focused tests and verify RED.
- [ ] Implement the CPU conversion variant and service branch.
- [ ] Preserve the existing MFT software candidate skip so the ranked walk falls through to FFmpeg's tested implementation.
- [ ] Run crate/app focused tests and verify GREEN.

### Task 5: Real FFmpeg and VM acceptance

**Files:**
- Modify only if required by verified behavior: FFmpeg probe/argument code and test fixtures.

- [ ] Obtain an LGPL shared Windows FFmpeg build in a local ignored/test location and point `CLIPLINE_FFMPEG` at it.
- [ ] Confirm `ffmpeg -h encoder=h264_mf` and a one-frame software encode succeed.
- [ ] Run `cargo test -p clipline-capture --test ffmpeg_encode -- --nocapture` and verify H.264 software encode/mux/ffprobe coverage.
- [ ] Launch Clipline on Microsoft Basic Display Adapter and verify the recorder reaches Running with a software H.264 status instead of `no video encoder could be opened`.
- [ ] Save a replay, validate it with ffprobe, and review it in WebView2.

### Task 6: Documentation and quality gates

**Files:**
- Modify: `docs/COMPATIBILITY.md`
- Modify: `handoff.md`

- [ ] Document the software VM fallback, its CPU tradeoff, and its dependency on the bundled FFmpeg runtime.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clean -p clipline-capture` followed by `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Review the diff for accidental behavior, dependency, license, and unsafe-surface changes.
- [ ] Relaunch the app with the validated FFmpeg bundle for user acceptance.
