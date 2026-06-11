# Clipline Media Foundation H.264 Encoder (Windows Platform Layer, Part 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A real `Encoder` implementation — hardware H.264 via a Media Foundation transform
(`IMFTransform`, async hardware MFT; AMF on this machine's RX 6700 XT) — and a real
`probe::enumerate()` listing available MFT backends. **Milestone exit criterion (handoff):**
`Recorder` with WGC + MFT on the dev machine → `save_replay` → the file plays in a real player
and ffprobe shows a sane stream.

**Architecture:** Platform-neutral first: `annexb.rs` (Annex B → 4-byte length-prefixed AVCC
conversion, NAL inspection, SPS/PPS extraction — clipline-mp4 requires length-prefixed NALs,
MFTs emit Annex B start codes), `Encoder::finish()` default method so encoders can drain
buffered tail packets at end of stream (Recorder calls it), `LimitedCapture` adapter (bounds an
endless capture source to N frames so `run_to_end` terminates — WGC never ends on its own), and
an `EncoderBackend::MfSoftware` probe variant (last-resort tier below X264). Windows side:
`windows/mft.rs` hosts the async-MFT state machine (event-driven NeedInput/HaveOutput,
`MFT_MESSAGE_SET_D3D_MANAGER` for GPU input, `MFT_MESSAGE_COMMAND_DRAIN` on finish);
`windows/nv12.rs` converts WGC's BGRA textures to NV12 (encoder input format) on the GPU via
the D3D11 video processor, scaling to the (even-rounded) encode size. Capture and encoder must
share one `ID3D11Device` (cross-device textures don't flow), so `WgcCapture` gains `*_on(device)`
constructors and `d3d11::create_device` turns on `ID3D10Multithread` protection (required when
the MFT's DXGI device manager shares the device).

**Tech Stack:** `windows` 0.62, added features: `Win32_Media_MediaFoundation`,
`Win32_Media_DirectShow` (ICodecAPI), `Win32_Graphics_Direct3D10` (ID3D10Multithread),
`Win32_System_Com_StructuredStorage`/`Win32_System_Variant` (VARIANT for ICodecAPI) — adjust
to wherever 0.62 actually puts these. No FFmpeg (ddoc §4's FFmpeg matrix is milestone +1).

**Environment notes:** Dev machine: AMD RX 6700 XT (AMF H.264/HEVC hardware MFTs, no AV1 —
RDNA2). Primary monitor is 5120x1440 — **wider than common H.264 encoder limits (4096)**, so
the e2e smoke records a window; the converter scales/even-rounds anyway. MFT runtime tests are
CI-skipped like the WGC device test (same rationale; windows-2025 runners proved hostile).
ffmpeg/ffprobe gets installed locally in Task 7 (winget) — required for the exit criterion.
Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

> **windows-rs drift note:** signatures below target `windows = "0.62"`; adjust field/wrapper
> details at compile time (PROPVARIANT/VARIANT helpers, attribute setter shapes, GetEvent flag
> types). The structure, state machine, and safety boundaries are the contract.

---

### Task 1: Annex B / AVCC utilities (platform-neutral)

clipline-mp4 expects 4-byte length-prefixed NALs (`avcC lengthSizeMinusOne=3`); MFTs emit
Annex B (`00 00 01` / `00 00 00 01` start codes), with SPS/PPS either on
`MF_MT_MPEG_SEQUENCE_HEADER` or in-band before the first IDR. Pure byte math — neutral, tested
on both OSes (handoff: unit-testable logic stays off the device layer).

**Files:**
- Create: `crates/clipline-capture/src/annexb.rs`
- Modify: `crates/clipline-capture/src/lib.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Tiny but structurally-real NALs: type byte (nal_ref_idc<<5 | type) + payload.
    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x0A, 0xAC];
    const PPS: &[u8] = &[0x68, 0xEE, 0x38, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x33];
    const NON_IDR: &[u8] = &[0x41, 0x9A, 0x02];
    const SEI: &[u8] = &[0x06, 0x05, 0x04];
    const AUD: &[u8] = &[0x09, 0x10];

    fn annexb(units: &[&[u8]]) -> Vec<u8> {
        // Alternate 4-byte and 3-byte start codes to exercise both.
        let mut out = Vec::new();
        for (i, u) in units.iter().enumerate() {
            out.extend_from_slice(if i % 2 == 0 { &[0, 0, 0, 1][..] } else { &[0, 0, 1][..] });
            out.extend_from_slice(u);
        }
        out
    }

    #[test]
    fn splits_mixed_start_codes() {
        let data = annexb(&[SPS, PPS, IDR]);
        let units = split_annexb(&data);
        assert_eq!(units, vec![SPS, PPS, IDR]);
    }

    #[test]
    fn split_handles_no_leading_code_gracefully() {
        assert!(split_annexb(b"junk without start codes").is_empty());
        assert!(split_annexb(&[]).is_empty());
    }

    #[test]
    fn nal_types_decode() {
        assert_eq!(nal_type(SPS), 7);
        assert_eq!(nal_type(PPS), 8);
        assert_eq!(nal_type(IDR), 5);
        assert_eq!(nal_type(NON_IDR), 1);
        assert_eq!(nal_type(AUD), 9);
    }

    #[test]
    fn extracts_sps_pps_from_sequence_header() {
        let hdr = annexb(&[SPS, PPS]);
        let (sps, pps) = extract_sps_pps(&hdr).expect("both present");
        assert_eq!(sps, SPS);
        assert_eq!(pps, PPS);
        // Also from a full access unit (in-band parameter sets).
        let au = annexb(&[AUD, SPS, PPS, SEI, IDR]);
        let (sps2, pps2) = extract_sps_pps(&au).expect("in-band");
        assert_eq!((sps2.as_slice(), pps2.as_slice()), (SPS, PPS));
        assert!(extract_sps_pps(&annexb(&[IDR])).is_none(), "no parameter sets");
    }

    #[test]
    fn avcc_conversion_length_prefixes_and_strips_metadata_nals() {
        // AUD/SPS/PPS are carried out-of-band (avcC); slices + SEI stay.
        let au = annexb(&[AUD, SPS, PPS, SEI, IDR]);
        let avcc = annexb_to_avcc(&au);
        let mut expected = Vec::new();
        for u in [SEI, IDR] {
            expected.extend_from_slice(&(u.len() as u32).to_be_bytes());
            expected.extend_from_slice(u);
        }
        assert_eq!(avcc, expected);
    }

    #[test]
    fn avcc_conversion_of_plain_frame() {
        let au = annexb(&[NON_IDR]);
        let avcc = annexb_to_avcc(&au);
        assert_eq!(&avcc[..4], &(NON_IDR.len() as u32).to_be_bytes());
        assert_eq!(&avcc[4..], NON_IDR);
    }

    #[test]
    fn even_rounds_dimensions_down() {
        assert_eq!(even_dimensions(2290, 1288), (2290, 1288));
        assert_eq!(even_dimensions(2291, 1289), (2290, 1288));
        assert_eq!(even_dimensions(1, 1), (2, 2), "minimum sane size");
    }
}
```

`lib.rs` gains:
```rust
pub mod annexb;

pub use annexb::{annexb_to_avcc, even_dimensions, extract_sps_pps, nal_type, split_annexb};
```

- [ ] **Step 2: Run tests to verify they fail** — `cargo test -p clipline-capture` → COMPILE ERROR.

- [ ] **Step 3: Write the implementation** (top of `annexb.rs`)

```rust
//! H.264 Annex B ↔ MP4 (AVCC) stream-format conversion. clipline-mp4
//! writes avcC with 4-byte length prefixes; MFTs emit Annex B start codes
//! (handoff "sharp edges"). Pure byte math — platform-neutral and unit
//! tested on every OS.

/// Split an Annex B stream into NAL units (start codes removed). Handles
/// 3- and 4-byte start codes; bytes before the first start code are
/// ignored (there is no NAL to attribute them to).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new(); // (payload_start, code_start)
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let code_start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((i + 3, code_start));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (idx, &(payload, _)) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).map(|&(_, code)| code).unwrap_or(data.len());
        if payload < end {
            units.push(&data[payload..end]);
        }
    }
    units
}

/// H.264 nal_unit_type (low 5 bits of the first byte).
/// 1 = non-IDR slice, 5 = IDR, 6 = SEI, 7 = SPS, 8 = PPS, 9 = AUD.
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1F).unwrap_or(0)
}

/// Pull (SPS, PPS) out of an Annex B blob — works on
/// `MF_MT_MPEG_SEQUENCE_HEADER` and on full access units with in-band
/// parameter sets.
pub fn extract_sps_pps(annexb: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let units = split_annexb(annexb);
    let sps = units.iter().find(|u| nal_type(u) == 7)?;
    let pps = units.iter().find(|u| nal_type(u) == 8)?;
    Some((sps.to_vec(), pps.to_vec()))
}

/// Convert one Annex B access unit to 4-byte-length-prefixed AVCC sample
/// data. AUD/SPS/PPS are dropped: parameter sets travel in avcC, and MP4
/// needs no access-unit delimiters.
pub fn annexb_to_avcc(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for unit in split_annexb(annexb) {
        if matches!(nal_type(unit), 7 | 8 | 9) {
            continue;
        }
        out.extend_from_slice(&(unit.len() as u32).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

/// Encoders (and NV12 itself) need even dimensions; round down, floor 2.
pub fn even_dimensions(width: u32, height: u32) -> (u32, u32) {
    ((width & !1).max(2), (height & !1).max(2))
}
```

- [ ] **Step 4: Run tests to verify they pass** — 7 new tests green, everything else untouched.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(capture): Annex B to AVCC conversion + NAL utilities

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: neutral groundwork — `Encoder::finish()`, `LimitedCapture`, `MfSoftware`

Three small neutral-side changes the Windows code needs (handoff rule: trait changes happen
with tests on the neutral side first):

1. **`Encoder::finish()`** — encoders buffer frames; without an end-of-stream drain the tail
   is silently lost. Default impl returns no packets (mocks unaffected); `Recorder::run_to_end`
   calls it when capture ends, before the final seal.
2. **`LimitedCapture<C>`** — WGC never returns `Ok(None)`; bounding to N frames lets
   `run_to_end` terminate for smokes and tests.
3. **`EncoderBackend::MfSoftware`** — `enumerate()` will see the Microsoft software H.264 MFT;
   it ranks *below* X264 (when FFmpeg lands, x264 is the preferred software tier per ddoc §3).

**Files:**
- Modify: `crates/clipline-capture/src/traits.rs`, `src/pipeline.rs`, `src/probe.rs`,
  `src/mock.rs` (add `LimitedCapture` there — it's a test/composition utility), `src/lib.rs`

- [ ] **Step 1: Write the failing tests**

`probe.rs` tests gain:
```rust
    #[test]
    fn mf_software_ranks_below_x264() {
        let caps = vec![
            EncoderCapability { backend: EncoderBackend::MfSoftware, codecs: vec![Codec::H264] },
            EncoderCapability { backend: EncoderBackend::X264, codecs: vec![Codec::H264] },
        ];
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::X264, Codec::H264)));
    }
```

`mock.rs` tests gain:
```rust
    #[test]
    fn limited_capture_truncates_an_endless_source() {
        use crate::mock::LimitedCapture;
        // MockCapture would produce 100; the limiter ends the stream at 3.
        let mut cap = LimitedCapture::new(MockCapture::new(100, 30), 3);
        let mut n = 0;
        while let Some(_) = cap.next_frame().unwrap() {
            n += 1;
        }
        assert_eq!(n, 3);
    }
```

`pipeline.rs` tests gain (plus a tiny test encoder that buffers one packet):
```rust
    /// Wraps MockEncoder but holds back the latest packet until finish() —
    /// models real encoders' internal buffering.
    struct OneFrameLatency {
        inner: MockEncoder,
        held: Option<crate::traits::EncodedPacket>,
    }

    impl Encoder for OneFrameLatency {
        fn encode(&mut self, frame: &crate::traits::Frame) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut out = self.inner.encode(frame)?;
            let newly = out.pop();
            let released = self.held.take();
            self.held = newly;
            Ok(released.into_iter().collect())
        }
        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }
        fn finish(&mut self) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            Ok(self.held.take().into_iter().collect())
        }
    }

    #[test]
    fn run_to_end_drains_encoder_via_finish() {
        let enc = OneFrameLatency { inner: MockEncoder::new(30, 30), held: None };
        let mut rec = Recorder::new(MockCapture::new(30, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        // All 30 frames present despite the encoder's one-frame latency.
        let total: usize = rec.ring().segments().map(|s| s.samples.len()).sum();
        assert_eq!(total, 30);
    }
```

- [ ] **Step 2: Run tests to verify they fail** — compile error (`finish` not on trait, etc.).

- [ ] **Step 3: Implement**

`traits.rs` — add to `trait Encoder`:
```rust
    /// Drain any internally buffered packets at end of stream. Called by
    /// the pipeline after the capture source ends.
    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        Ok(Vec::new())
    }
```

`pipeline.rs` — in `run_to_end`, after the capture loop and before the final-seal block:
```rust
        for pkt in self.encoder.finish()? {
            if pkt.is_keyframe && !self.pending.is_empty() {
                self.seal_pending(pkt.pts_s);
            }
            self.pending.push(pkt);
        }
```

`probe.rs` — `EncoderBackend` gains `MfSoftware` *after* `X264` (derived `Ord` = priority):
```rust
pub enum EncoderBackend {
    Nvenc,
    Amf,
    QuickSync,
    X264,
    /// Microsoft software H.264 MFT — last resort until FFmpeg/x264 lands.
    MfSoftware,
}
```

`mock.rs` — add:
```rust
/// Bounds an endless capture source to `remaining` frames so
/// `Recorder::run_to_end` terminates (WGC never ends on its own).
pub struct LimitedCapture<C: CaptureEngine> {
    inner: C,
    remaining: u64,
}

impl<C: CaptureEngine> LimitedCapture<C> {
    pub fn new(inner: C, frames: u64) -> Self {
        Self { inner, remaining: frames }
    }
}

impl<C: CaptureEngine> CaptureEngine for LimitedCapture<C> {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let frame = self.inner.next_frame()?;
        if frame.is_some() {
            self.remaining -= 1;
        }
        Ok(frame)
    }
}
```

`lib.rs`: export `LimitedCapture`.

- [ ] **Step 4: Run the full suite** — `cargo test --workspace` green.

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(capture): Encoder::finish drain, LimitedCapture, MfSoftware tier

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: real `probe::enumerate()` via MFTEnumEx

**Files:**
- Modify: `crates/clipline-capture/Cargo.toml` (new `windows` features)
- Create: `crates/clipline-capture/src/windows/mft_probe.rs`
- Modify: `crates/clipline-capture/src/windows/mod.rs`

- [ ] **Step 1: Add features** to the `windows` dependency:
`"Win32_Media_MediaFoundation"`, `"Win32_Media_DirectShow"`, `"Win32_Graphics_Direct3D10"`,
`"Win32_System_Com_StructuredStorage"`, `"Win32_System_Variant"`, `"Win32_System_Com"`.

- [ ] **Step 2: Write the failing test** (`mft_probe.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Enumeration itself must work anywhere MF exists; result contents
    /// depend on hardware. On CI this may legitimately be empty.
    #[test]
    fn enumerate_returns_without_error() {
        let caps = enumerate().expect("MFTEnumEx works");
        for c in &caps {
            assert!(!c.codecs.is_empty(), "empty-codec entries are filtered");
        }
        eprintln!("encoders found: {caps:?}");
    }

    #[test]
    fn vendor_ids_map_to_backends() {
        assert_eq!(backend_for_vendor("VEN_10DE"), Some(EncoderBackend::Nvenc));
        assert_eq!(backend_for_vendor("VEN_1002"), Some(EncoderBackend::Amf));
        assert_eq!(backend_for_vendor("VEN_8086"), Some(EncoderBackend::QuickSync));
        assert_eq!(backend_for_vendor("VEN_FFFF"), None);
    }
}
```

`windows/mod.rs`: `pub mod mft_probe;`

- [ ] **Step 3: Verify failure**, then **Step 4: Implement**

```rust
//! Real encoder probing (ddoc §3) via MFTEnumEx: which hardware vendors
//! offer encoder MFTs for which codecs, plus the Microsoft software H.264
//! MFT as the last-resort tier.

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    MFShutdown, MFStartup, MFTEnumEx, IMFActivate, MFSTARTUP_FULL, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, MFT_REGISTER_TYPE_INFO, MFMediaType_Video,
    MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC,
};

use crate::probe::{Codec, EncoderBackend, EncoderCapability};

pub fn backend_for_vendor(vendor: &str) -> Option<EncoderBackend> {
    match vendor {
        "VEN_10DE" => Some(EncoderBackend::Nvenc),
        "VEN_1002" => Some(EncoderBackend::Amf),
        "VEN_8086" => Some(EncoderBackend::QuickSync),
        _ => None,
    }
}

/// One MFTEnumEx pass for a codec subtype; returns the vendor strings of
/// hardware encoders (or whether a software one exists, for H.264).
fn hardware_vendors_for(subtype: GUID) -> windows::core::Result<Vec<String>> { /* MFTEnumEx(
    MFT_CATEGORY_VIDEO_ENCODER, HARDWARE|SORTANDFILTER, None,
    Some(&MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype }))
    → for each IMFActivate: GetString(MFT_ENUM_HARDWARE_VENDOR_ID_Attribute) (ignore failures)
    → collect; CoTaskMemFree the activate array per MFTEnumEx contract (windows-rs manages the
    out slice as *mut *mut IMFActivate + count — release each activate). */ }

/// MF-backed implementation of the ddoc §3 probe.
pub fn enumerate() -> windows::core::Result<Vec<EncoderCapability>> {
    // SAFETY: MFStartup/MFShutdown pair; FULL gives the standard platform.
    unsafe { MFStartup(crate::windows::mft::MF_VERSION, MFSTARTUP_FULL)? };
    let mut by_backend: Vec<(EncoderBackend, Vec<Codec>)> = Vec::new();
    for (subtype, codec) in [
        (MFVideoFormat_AV1, Codec::Av1),
        (MFVideoFormat_HEVC, Codec::Hevc),
        (MFVideoFormat_H264, Codec::H264),
    ] {
        for vendor in hardware_vendors_for(subtype)? {
            if let Some(backend) = backend_for_vendor(&vendor) {
                match by_backend.iter_mut().find(|(b, _)| *b == backend) {
                    Some((_, codecs)) => codecs.push(codec),
                    None => by_backend.push((backend, vec![codec])),
                }
            }
        }
    }
    // Software H.264 (sync MFT enumeration, Microsoft's encoder).
    if software_h264_exists()? {
        by_backend.push((EncoderBackend::MfSoftware, vec![Codec::H264]));
    }
    Ok(by_backend
        .into_iter()
        .map(|(backend, codecs)| EncoderCapability { backend, codecs })
        .collect())
}
```

(Exact MFTEnumEx out-param handling and the `MF_VERSION` constant land in `windows/mft.rs` —
created in Task 6; for this task put `pub const MF_VERSION: u32 = 0x0002_0070;` in a small
`windows/mft.rs` stub with just the constant, or temporarily local. Don't call `MFShutdown`
per call — leak the startup; MF refcounts startups and the process uses MF for its lifetime.)

- [ ] **Step 5: Test** (on dev machine expect AMF H264+HEVC and MfSoftware H264 in stderr),
**Step 6: Commit** `feat(capture): MFTEnumEx-backed encoder probe`.

---

### Task 4: shared D3D device

The MFT consumes the capture textures, so capture and encode must run on one device, and the
DXGI device manager requires multithread protection on it.

**Files:**
- Modify: `crates/clipline-capture/src/windows/d3d11.rs`, `src/windows/wgc.rs`

- [ ] **Step 1: failing test** (`wgc.rs` tests; CI-skipped like the existing one)

```rust
    #[test]
    fn capture_runs_on_a_caller_provided_device() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: WGC device test needs a real interactive desktop");
            return;
        }
        let (device, _ctx) = crate::windows::d3d11::create_device().expect("device");
        let mut cap = match WgcCapture::primary_monitor_on(device.clone()) {
            Ok(cap) => cap,
            Err(e) => { eprintln!("SKIP: WGC unavailable: {e}"); return; }
        };
        let frame = cap.next_frame_timeout(std::time::Duration::from_secs(5))
            .expect("frame").expect("open");
        let crate::traits::FrameData::Gpu(tex) = &frame.data else { panic!("gpu") };
        // The texture lives on the device we provided.
        use windows::core::Interface;
        let mut owner = None;
        unsafe { tex.GetDevice(&mut owner) };
        assert_eq!(owner.unwrap().as_raw(), device.as_raw());
    }
```

- [ ] **Step 2: verify failure → Step 3: implement**

`d3d11.rs` — in `create_device_with`, after creation, enable multithread protection:
```rust
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
// MF's DXGI device manager shares this device across threads (MSDN:
// required for D3D-aware MFTs).
let mt: ID3D10Multithread = device.cast()?;
// SAFETY: trivial setter on a valid interface.
unsafe { mt.SetMultithreadProtected(true) };
```

`wgc.rs` — split constructors:
```rust
    pub fn primary_monitor() -> Result<Self, CaptureError> {
        let (device, _) = d3d11::create_device().map_err(|e| CaptureError::Init(e.to_string()))?;
        Self::primary_monitor_on(device)
    }

    pub fn primary_monitor_on(device: ID3D11Device) -> Result<Self, CaptureError> { /* hmon checks → create_item → Self::new(item, device) */ }

    pub fn for_window(hwnd: HWND) -> Result<Self, CaptureError> { /* same shape */ }
    pub fn for_window_on(device: ID3D11Device, hwnd: HWND) -> Result<Self, CaptureError> { ... }

    fn new(item: GraphicsCaptureItem, device: ID3D11Device) -> Result<Self, CaptureError> {
        // unchanged, except: derive the context from the device
        // (device.GetImmediateContext) instead of creating the device here.
    }
```

- [ ] **Step 4: run tests** (both WGC device tests capture for real locally), **Step 5: commit**
`refactor(capture): WGC on a caller-provided D3D device + MT protection`.

---

### Task 5: BGRA→NV12 `VideoConverter` (D3D11 video processor)

**Files:**
- Create: `crates/clipline-capture/src/windows/nv12.rs`
- Modify: `crates/clipline-capture/src/windows/mod.rs`, `src/windows/d3d11.rs` (NV12 texture helper)

- [ ] **Step 1: failing test** (WARP — video processor works on WARP; self-skip if not)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgra_texture_to_nv12_with_scaling() {
        let (device, _ctx) = crate::windows::d3d11::create_device_for_tests().expect("WARP");
        let mut conv = match VideoConverter::new(&device, 64, 64, 32, 32) {
            Ok(c) => c,
            Err(e) => { eprintln!("SKIP: video processor unavailable: {e}"); return; }
        };
        let src = crate::windows::d3d11::create_bgra_texture(&device, 64, 64).expect("src");
        let nv12 = conv.convert(&src).expect("convert");
        let desc = crate::windows::d3d11::texture_desc(&nv12);
        assert_eq!((desc.Width, desc.Height), (32, 32));
        assert_eq!(desc.Format, windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12);
    }
}
```

- [ ] **Step 2: verify failure → Step 3: implement**

`d3d11.rs` gains:
```rust
/// NV12 render-target texture (video processor output / encoder input).
pub fn create_nv12_texture(device: &ID3D11Device, width: u32, height: u32) -> WinResult<ID3D11Texture2D> {
    // As create_bgra_texture but Format: DXGI_FORMAT_NV12,
    // BindFlags: RENDER_TARGET | SHADER_RESOURCE.
}

pub fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC { ... }
```

`nv12.rs`:
```rust
//! GPU BGRA→NV12 conversion + scaling via the D3D11 video processor —
//! WGC delivers BGRA, H.264 encoder MFTs consume NV12; ddoc §3/§7 require
//! the path to stay on the GPU.

pub struct VideoConverter {
    device: ID3D11Device,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    out_width: u32,
    out_height: u32,
}

impl VideoConverter {
    /// in_* = capture size, out_* = encode size (already even-rounded).
    pub fn new(device: &ID3D11Device, in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> WinResult<Self> {
        // device.cast::<ID3D11VideoDevice>()
        // context (GetImmediateContext).cast::<ID3D11VideoContext>()
        // CreateVideoProcessorEnumerator(D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        //   InputFrameFormat: PROGRESSIVE, Input/OutputWidth/Height, frame rates 60/1,
        //   Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL })
        // CreateVideoProcessor(&enumerator, 0)
    }

    /// Convert one BGRA texture into a freshly allocated NV12 texture.
    pub fn convert(&mut self, bgra: &ID3D11Texture2D) -> WinResult<ID3D11Texture2D> {
        // out = d3d11::create_nv12_texture(&self.device, self.out_width, self.out_height)
        // input view: CreateVideoProcessorInputView(bgra, &enumerator,
        //   &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC { ViewDimension: TEXTURE2D, .. })
        // output view: CreateVideoProcessorOutputView(out, &enumerator, TEXTURE2D desc)
        // stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: TRUE, pInputSurface: view, .. }
        // VideoProcessorBlt(&processor, &out_view, 0, &[stream])
        // → Ok(out)
    }
}
```

Allocate the output per frame (the encoder holds textures asynchronously; pooling is a
follow-up). One `VideoConverter` per recording — input size is fixed by the capture item.

- [ ] **Step 4: run tests** (works on WARP locally; CI may skip), **Step 5: commit**
`feat(capture): GPU BGRA->NV12 converter via D3D11 video processor`.

---

### Task 6: `MftH264Encoder`

The core of the milestone. Async hardware MFT (AMF here), D3D-aware, implementing `Encoder`.

**Files:**
- Create: `crates/clipline-capture/src/windows/mft.rs` (replacing the Task 3 stub if any)
- Modify: `crates/clipline-capture/src/windows/mod.rs`

**Contract recap (handoff + sharp edges):**
- Output `EncodedPacket.data` = **AVCC** (Task 1 converter), `is_keyframe` from
  `MFSampleExtension_CleanPoint`, pts/duration from sample time/duration (100 ns → s).
- `track_config()` = `VideoTrackConfig { width, height, timescale: 90_000, sps, pps }` — SPS/PPS
  from `MF_MT_MPEG_SEQUENCE_HEADER` on the negotiated output type, else from the first
  keyframe's in-band sets (cache once found).
- B-frames **off** (CODECAPI_AVEncMPVDefaultBPictureCount=0): pts==dts; the muxer has no ctts.
- GOP size via CODECAPI_AVEncMPVGOPSize (default: 2 s of frames); `MF_LOW_LATENCY` on.
- `finish()` sends `MFT_MESSAGE_COMMAND_DRAIN` and collects remaining outputs.

```rust
pub struct MftConfig {
    pub width: u32,       // already even (annexb::even_dimensions)
    pub height: u32,
    pub fps: u32,         // nominal, for media types + GOP sizing
    pub bitrate_bps: u32, // e.g. 12_000_000
}

pub struct MftH264Encoder {
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    converter: VideoConverter,        // owns BGRA→NV12
    clockless_input_id: u32,          // from GetStreamIDs (E_NOTIMPL → 0)
    output_id: u32,
    need_input_credits: u32,
    cached_sps_pps: Option<(Vec<u8>, Vec<u8>)>,
    cfg: MftConfig,
    prev_pts_s: Option<f64>,
}
```

Construction sequence (all `map_err` → `EncodeError::Backend(..)`):
1. `MFStartup(MF_VERSION, MFSTARTUP_FULL)`.
2. MFTEnumEx hardware H.264 (as Task 3) → first activate → `ActivateObject::<IMFTransform>()`.
3. Async unlock: `transform.GetAttributes()` → `SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, 1)`.
4. `MFCreateDXGIDeviceManager(&mut token, &mut manager)`; `manager.ResetDevice(device, token)`;
   `ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager as usize)`.
5. Stream IDs via `GetStreamIDs` (E_NOTIMPL ⇒ 0/0).
6. **Output type first** (encoder MFTs require it): `MFCreateMediaType` with
   MF_MT_MAJOR_TYPE=Video, MF_MT_SUBTYPE=H264, MF_MT_AVG_BITRATE, MF_MT_FRAME_SIZE
   (u64 (w<<32)|h), MF_MT_FRAME_RATE ((fps<<32)|1), MF_MT_INTERLACE_MODE=Progressive,
   MF_MT_MPEG2_PROFILE = eAVEncH264VProfile_High → `SetOutputType(output_id, ty, 0)`.
7. **Input type**: iterate `GetInputAvailableType` for NV12 subtype, set size/fps, `SetInputType`.
8. ICodecAPI (`transform.cast()`): AVEncMPVGOPSize = 2*fps, AVEncMPVDefaultBPictureCount = 0,
   (best-effort `let _ =` — software/odd MFTs may not implement all), CODECAPI_AVLowLatencyMode on.
9. SPS/PPS attempt #1: `GetOutputCurrentType` → blob `MF_MT_MPEG_SEQUENCE_HEADER` →
   `annexb::extract_sps_pps` → cache.
10. `ProcessMessage(NOTIFY_BEGIN_STREAMING)` + `(NOTIFY_START_OF_STREAM)`.

`encode(&mut self, frame)`:
```text
packets = []
nv12 = converter.convert(gpu_texture_of(frame))      // FrameData::Gpu required
sample = MFCreateSample; buffer = MFCreateDXGISurfaceBuffer(ID3D11Texture2D, nv12, 0, FALSE)
sample.AddBuffer; SetSampleTime(frame.pts_s * 1e7); SetSampleDuration(delta_or_nominal * 1e7)
fed = false
while !fed:
    event = events.GetEvent(0)                       // blocking — encoder always asks
    match event type:
        METransformNeedInput → if !fed { ProcessInput(input_id, sample, 0); fed = true }
                                else   { need_input_credits += 1 }
        METransformHaveOutput → packets.push(drain_one())
// opportunistic non-blocking drain:
loop:
    event = events.GetEvent(MF_EVENT_FLAG_NO_WAIT)   // MF_E_NO_EVENTS_AVAILABLE → break
    NeedInput → need_input_credits += 1
    HaveOutput → packets.push(drain_one())
return packets
```
(If `need_input_credits > 0` on entry, skip the blocking wait and feed immediately.)

`drain_one()`:
```text
MFT_OUTPUT_DATA_BUFFER { dwStreamID: output_id, pSample: None, .. }   // HW MFT provides samples
ProcessOutput(0, &mut [buf], &mut status)
sample = buf.pSample; media_buf = sample.ConvertToContiguousBuffer()
lock → annexb bytes; if cached_sps_pps.is_none() → try extract_sps_pps(bytes) → cache
EncodedPacket {
    data: annexb_to_avcc(bytes),
    pts_s: sample.GetSampleTime()? as f64 / 1e7,
    duration_s: sample.GetSampleDuration().unwrap_or(nominal) / 1e7,
    is_keyframe: sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1,
}
(release MFT_OUTPUT_DATA_BUFFER.pEvents if set)
```
On `MF_E_TRANSFORM_STREAM_CHANGE`: re-`GetOutputAvailableType(0)` + `SetOutputType`, retry —
and re-attempt the sequence-header SPS/PPS fetch.

`finish()`: `ProcessMessage(NOTIFY_END_OF_STREAM)`, `ProcessMessage(COMMAND_DRAIN)`, then pump
events until `METransformDrainComplete`, collecting HaveOutput packets.

`track_config()`: cached SPS/PPS (empty vecs + log if never seen — save_replay runs after
encoding, so in practice the first keyframe filled it).

- [ ] **Step 1: failing test** (CI-skipped; runs for real on the RX 6700 XT):

```rust
    #[test]
    fn encodes_synthetic_frames_to_keyframed_avcc() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: hardware MFT test");
            return;
        }
        let (device, _ctx) = crate::windows::d3d11::create_device().expect("device");
        let cfg = MftConfig { width: 640, height: 360, fps: 30, bitrate_bps: 2_000_000 };
        let mut enc = match MftH264Encoder::new(&device, 640, 360, cfg) {
            Ok(e) => e,
            Err(e) => { eprintln!("SKIP: no hardware H.264 MFT: {e}"); return; }
        };
        let mut packets = Vec::new();
        for i in 0..30 {
            let tex = crate::windows::d3d11::create_bgra_texture(&device, 640, 360).unwrap();
            let frame = Frame { pts_s: i as f64 / 30.0, data: FrameData::Gpu(tex) };
            packets.extend(enc.encode(&frame).unwrap());
        }
        packets.extend(enc.finish().unwrap());
        assert!(packets.len() >= 25, "most frames came back (got {})", packets.len());
        assert!(packets[0].is_keyframe, "stream starts with IDR");
        // AVCC: first 4 bytes are a plausible NAL length, not a start code.
        let first = &packets[0].data;
        assert_ne!(&first[..4], &[0, 0, 0, 1], "no Annex B start codes");
        let cfg = enc.track_config();
        assert!(!cfg.sps.is_empty() && !cfg.pps.is_empty(), "SPS/PPS extracted");
        let mono = packets.windows(2).all(|w| w[1].pts_s >= w[0].pts_s);
        assert!(mono, "pts monotonic (B-frames disabled)");
    }
```

(`MftH264Encoder::new(&device, in_w, in_h, cfg)` builds the `VideoConverter` from
`in_w`/`in_h` (capture size) to `cfg.width`/`cfg.height`.)

- [ ] **Step 2: verify failure → Step 3: implement → Step 4: iterate on real hardware until
green** (this step is where AMF quirks surface — stream-change events, sample attributes;
debug on the machine, the structure above is the contract).

- [ ] **Step 5: workspace tests + commit**
`feat(capture): Media Foundation H.264 encoder behind Encoder trait`.

---

### Task 7: end-to-end `record_smoke` — the milestone exit criterion

**Files:**
- Create: `crates/clipline-capture/examples/record_smoke.rs`

- [ ] **Step 0: install ffmpeg locally** (`winget install Gyan.FFmpeg`, new shell for PATH) —
the handoff machine-setup recommendation; needed to verify the artifact.

- [ ] **Step 1: write the example**

```rust
//! End-to-end smoke: WGC → NV12 → hardware H.264 MFT → ReplayRing →
//! save_replay → finalized hybrid MP4 on disk. Run manually:
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --window "league"
//!   cargo run -p clipline-capture --example record_smoke -- --seconds 5 --out replay.mp4
```
Flow: parse `--seconds N` (default 5), `--window SUBSTR` (else primary monitor — warn that
5120-wide may exceed H.264 limits and scale to ≤2560 via `even_dimensions` + cap), `--out PATH`
(default `replay_smoke.mp4`). One shared D3D device → `WgcCapture::*_on` + first frame gives
capture size → `MftH264Encoder::new(&device, in_w, in_h, MftConfig { even-rounded size, fps 60,
bitrate 12_000_000 })` → `Recorder::new(LimitedCapture::new(cap, seconds*60), enc, usize::MAX)`
→ `run_to_end()` → `save_replay(File::create(out), seconds as f64 + 1.0, None)` → print packet
count, file size, and run ffprobe on the result if available (reuse the repo's skip-if-absent
pattern: print SKIP if not found).

Wait — `save_replay` needs `Write + Seek`: `std::fs::File` implements both. Good.

- [ ] **Step 2: run it for real** — `--window` against an open window first, then monitor.
Then:
- `ffprobe -v error -show_entries stream=codec_name,width,height,nb_frames,avg_frame_rate -show_entries format=duration <out>` — h264, expected dims, ~seconds duration, sane frame count.
- **Open the file in a real player** (`Start-Process replay_smoke.mp4`) and confirm it plays.
  Ask the human at the keyboard to confirm visually if the agent can't.

- [ ] **Step 3: commit** with the observed ffprobe output in the body:
`feat(capture): record_smoke e2e - WGC + MFT to playable MP4`.

---

### Task 8: quality gates

- [ ] `cargo test --workspace` green (device tests real locally, skipped on CI).
- [ ] `cargo clippy --workspace --all-targets` zero warnings.
- [ ] Push; CI green on ubuntu-latest + windows-latest.
- [ ] Update `handoff.md`: milestone 2 done (with observed e2e numbers), milestone 3 (WASAPI) next.

---

## Out of scope (follow-ups)

- Software encode path (CPU NV12 staging readback + sync MFT) — the probe lists `MfSoftware`,
  but encoding via it waits for the FFmpeg milestone to decide the software tier.
- HEVC/AV1 output, CBR/CQP rate-control knobs, encoder texture pooling, sequence-header refresh
  on dynamic resolution change (`MF_E_TRANSFORM_STREAM_CHANGE` mid-stream beyond retry).
- WASAPI loopback `AudioSource` (milestone 3); A/V sync hardening (milestone 4).
- Real-time pacing (the smoke records as fast as WGC delivers, which is real-time by nature of
  the frame pool — fine; a production recorder thread is Tauri-shell work).
