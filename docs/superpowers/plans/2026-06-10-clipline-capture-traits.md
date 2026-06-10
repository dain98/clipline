# Clipline Capture/Encode Trait Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define the platform abstraction from ddoc §3 — `CaptureEngine` / `Encoder` traits, deterministic encoder probing (NVENC → AMF → QuickSync → x264; AV1 → HEVC → H.264) — and prove the full recording architecture composes by driving mock capture → mock encoder → GOP-aligned segments → `ReplayRing` → Save Replay → `HybridMp4Writer` → ffprobe-validated MP4, all on Linux before any Windows code exists.

**Architecture:** New `clipline-capture` crate. `traits.rs` defines `Frame`, `EncodedPacket`, `CaptureEngine`, `Encoder` (synchronous pull model — production runs this on a dedicated capture thread). `probe.rs` implements encoder selection per ddoc §3's deterministic priority. `mock.rs` provides `MockCapture`/`MockEncoder` for tests and CI. `pipeline.rs` is `Recorder`: pulls frames, encodes, groups packets into GOP-aligned `Segment`s pushed to the `ReplayRing`, and `save_replay()` re-muxes a trailing window into a Hybrid MP4. Prerequisite: `clipline-buffer::Segment` gains a per-sample index (`SampleInfo`) so saved segments can be sliced back into muxer samples — its `data` stays an opaque concatenation.

**Tech Stack:** Rust std + thiserror; depends on `clipline-buffer` and `clipline-mp4`. ffprobe e2e reuses the skip-if-absent pattern from clipline-mp4.

**Environment notes:** `cargo` at `~/.cargo/bin/cargo`. Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Encoder probe/selection (`clipline-capture`)

**Files:**
- Modify: `Cargo.toml` (workspace members)
- Create: `crates/clipline-capture/Cargo.toml`, `crates/clipline-capture/src/lib.rs`, `crates/clipline-capture/src/probe.rs`
- Test: inline `#[cfg(test)]` in `probe.rs`

- [ ] **Step 1: Scaffold the crate**

Root `Cargo.toml` members gains `"crates/clipline-capture"`.

`crates/clipline-capture/Cargo.toml`:
```toml
[package]
name = "clipline-capture"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
clipline-buffer = { path = "../clipline-buffer" }
clipline-mp4 = { path = "../clipline-mp4" }
thiserror = { workspace = true }
```

`src/lib.rs`:
```rust
pub mod probe;

pub use probe::{select_encoder, Codec, EncoderBackend, EncoderCapability};
```

- [ ] **Step 2: Write the failing tests** (`probe.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_nvenc_over_other_backends() {
        let caps = vec![
            EncoderCapability { backend: EncoderBackend::Amf, codecs: vec![Codec::Av1] },
            EncoderCapability {
                backend: EncoderBackend::Nvenc,
                codecs: vec![Codec::H264, Codec::Hevc],
            },
        ];
        // Backend priority wins even when a lower backend has a better codec.
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::Nvenc, Codec::Hevc)));
    }

    #[test]
    fn prefers_av1_within_a_backend() {
        let caps = vec![EncoderCapability {
            backend: EncoderBackend::QuickSync,
            codecs: vec![Codec::H264, Codec::Av1, Codec::Hevc],
        }];
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::QuickSync, Codec::Av1)));
    }

    #[test]
    fn falls_back_to_software_x264() {
        let caps = vec![EncoderCapability {
            backend: EncoderBackend::X264,
            codecs: vec![Codec::H264],
        }];
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::X264, Codec::H264)));
    }

    #[test]
    fn no_encoders_means_none() {
        assert_eq!(select_encoder(&[]), None);
        let empty_codecs = vec![EncoderCapability {
            backend: EncoderBackend::Nvenc,
            codecs: vec![],
        }];
        assert_eq!(select_encoder(&empty_codecs), None);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: COMPILE ERROR.

- [ ] **Step 4: Write the implementation** (top of `probe.rs`)

```rust
/// Hardware/software encoder backends in deterministic priority order
/// (ddoc §3: NVENC → AMF → QuickSync → x264 software fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncoderBackend {
    Nvenc,
    Amf,
    QuickSync,
    X264,
}

/// Codec preference order (ddoc §3: AV1 → HEVC → H.264).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Codec {
    Av1,
    Hevc,
    H264,
}

/// What one backend reported during startup probing.
#[derive(Debug, Clone)]
pub struct EncoderCapability {
    pub backend: EncoderBackend,
    pub codecs: Vec<Codec>,
}

/// Pick the encoder: highest-priority backend that offers any codec, then
/// the most-preferred codec within it. Derived `Ord` on the enums encodes
/// the ddoc §3 priority (declaration order).
pub fn select_encoder(available: &[EncoderCapability]) -> Option<(EncoderBackend, Codec)> {
    available
        .iter()
        .filter(|c| !c.codecs.is_empty())
        .min_by_key(|c| c.backend)
        .map(|c| (c.backend, *c.codecs.iter().min().expect("non-empty")))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): deterministic encoder probe/selection

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Capture/encoder traits + mocks

**Files:**
- Create: `crates/clipline-capture/src/traits.rs`, `crates/clipline-capture/src/mock.rs`
- Modify: `crates/clipline-capture/src/lib.rs`
- Test: inline `#[cfg(test)]` in `mock.rs`

- [ ] **Step 1: Write the failing tests** (`mock.rs`)

Add to `lib.rs`:
```rust
pub mod mock;
pub mod traits;

pub use mock::{MockCapture, MockEncoder};
pub use traits::{CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder, Frame, FrameData};
```

`mock.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{CaptureEngine, Encoder};

    #[test]
    fn mock_capture_produces_n_frames_then_ends() {
        let mut cap = MockCapture::new(3, 30);
        let f0 = cap.next_frame().unwrap().unwrap();
        assert_eq!(f0.pts_s, 0.0);
        let f1 = cap.next_frame().unwrap().unwrap();
        assert!((f1.pts_s - 1.0 / 30.0).abs() < 1e-9);
        cap.next_frame().unwrap().unwrap();
        assert!(cap.next_frame().unwrap().is_none(), "source ended");
    }

    #[test]
    fn mock_encoder_emits_keyframes_on_gop_boundaries() {
        let mut cap = MockCapture::new(5, 30);
        let mut enc = MockEncoder::new(2, 30); // GOP length 2
        let mut keys = Vec::new();
        while let Some(frame) = cap.next_frame().unwrap() {
            for pkt in enc.encode(&frame).unwrap() {
                keys.push(pkt.is_keyframe);
            }
        }
        assert_eq!(keys, vec![true, false, true, false, true]);
    }

    #[test]
    fn mock_encoder_packets_carry_pts_and_duration() {
        let mut enc = MockEncoder::new(30, 30);
        let frame = Frame { pts_s: 1.5, data: FrameData::Cpu(vec![0; 4]) };
        let pkts = enc.encode(&frame).unwrap();
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].pts_s, 1.5);
        assert!((pkts[0].duration_s - 1.0 / 30.0).abs() < 1e-9);
        assert!(!pkts[0].data.is_empty());
    }

    #[test]
    fn mock_encoder_provides_a_muxable_track_config() {
        let enc = MockEncoder::new(30, 30);
        let cfg = enc.track_config();
        assert!(cfg.timescale > 0);
        assert!(!cfg.sps.is_empty());
        assert!(!cfg.pps.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: COMPILE ERROR.

- [ ] **Step 3: Write the implementations**

`traits.rs`:
```rust
use clipline_mp4::VideoTrackConfig;

/// One captured video frame. Platform implementations keep pixels on the
/// GPU (ddoc §3: frames stay as GPU textures); the pipeline only needs
/// timing plus an opaque payload handle.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Seconds since capture start (monotonic, from the capture clock).
    pub pts_s: f64,
    pub data: FrameData,
}

/// Frame payload. `Cpu` serves mocks/tests/software paths; a GPU texture
/// variant arrives with the Windows WGC implementation.
#[derive(Debug, Clone)]
pub enum FrameData {
    Cpu(Vec<u8>),
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("capture device lost: {0}")]
    DeviceLost(String),
}

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("encoder failed: {0}")]
    Backend(String),
}

/// Pull-model capture source. `Ok(None)` means the source ended.
pub trait CaptureEngine {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError>;
}

/// One encoded sample out of the encoder.
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts_s: f64,
    pub duration_s: f64,
    pub is_keyframe: bool,
}

/// Video encoder. May buffer internally (B-frames later), hence Vec out.
pub trait Encoder {
    fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError>;
    /// Track parameters for muxing the produced stream.
    fn track_config(&self) -> VideoTrackConfig;
}
```

`mock.rs` implementation (above the tests):
```rust
use clipline_mp4::VideoTrackConfig;

use crate::traits::{
    CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder, Frame, FrameData,
};

/// Deterministic frame source: `total` frames at `fps`.
pub struct MockCapture {
    total: u64,
    fps: u32,
    produced: u64,
}

impl MockCapture {
    pub fn new(total: u64, fps: u32) -> Self {
        Self { total, fps, produced: 0 }
    }
}

impl CaptureEngine for MockCapture {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        if self.produced >= self.total {
            return Ok(None);
        }
        let pts_s = self.produced as f64 / self.fps as f64;
        self.produced += 1;
        Ok(Some(Frame { pts_s, data: FrameData::Cpu(vec![0u8; 16]) }))
    }
}

/// Deterministic "encoder": one packet per frame, keyframe every `gop_len`
/// frames, recognizable payload bytes for muxer round-trip assertions.
pub struct MockEncoder {
    gop_len: u64,
    fps: u32,
    count: u64,
}

impl MockEncoder {
    pub fn new(gop_len: u64, fps: u32) -> Self {
        Self { gop_len, fps, count: 0 }
    }
}

impl Encoder for MockEncoder {
    fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError> {
        let FrameData::Cpu(_) = &frame.data;
        let idx = self.count;
        self.count += 1;
        let mut data = format!("F{idx:06}").into_bytes();
        data.resize(64 + (idx % 7) as usize, 0xEE); // mildly varying sizes
        Ok(vec![EncodedPacket {
            data,
            pts_s: frame.pts_s,
            duration_s: 1.0 / self.fps as f64,
            is_keyframe: idx % self.gop_len == 0,
        }])
    }

    fn track_config(&self) -> VideoTrackConfig {
        VideoTrackConfig {
            width: 128,
            height: 128,
            timescale: 90_000,
            sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            pps: vec![0x68, 0xEE, 0x38, 0x80],
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): CaptureEngine/Encoder traits with deterministic mocks

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Per-sample index on `clipline-buffer::Segment`

Saved windows must be re-muxable: `Segment.data` stays an opaque concatenation, but a `samples` index records each sample's size/duration/sync so the pipeline can slice it back into muxer samples.

**Files:**
- Modify: `crates/clipline-buffer/src/segment.rs`
- Modify: `crates/clipline-buffer/src/ring.rs` (test helper gains the new field)
- Test: extend `#[cfg(test)]` in `segment.rs`

- [ ] **Step 1: Write the failing test** (new `#[cfg(test)]` module in `segment.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_index_slices_data_back_into_samples() {
        let seg = Segment {
            starts_with_keyframe: true,
            pts_start_s: 0.0,
            duration_s: 0.1,
            data: b"AAAABBBCC".to_vec(),
            samples: vec![
                SampleInfo { size: 4, duration_s: 0.04, is_sync: true },
                SampleInfo { size: 3, duration_s: 0.03, is_sync: false },
                SampleInfo { size: 2, duration_s: 0.03, is_sync: false },
            ],
        };
        let slices: Vec<&[u8]> = seg.sample_slices().collect();
        assert_eq!(slices, vec![b"AAAA".as_slice(), b"BBB".as_slice(), b"CC".as_slice()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p clipline-buffer`
Expected: COMPILE ERROR (`SampleInfo` not defined; `Segment` has no `samples`).

- [ ] **Step 3: Update the implementation**

`segment.rs` becomes:
```rust
/// Metadata for one encoded sample inside a segment's `data`.
#[derive(Debug, Clone, Copy)]
pub struct SampleInfo {
    /// Byte length within `Segment::data`.
    pub size: u32,
    pub duration_s: f64,
    pub is_sync: bool,
}

/// One encoded, GOP-aligned media segment (ddoc §6). `data` is the opaque
/// concatenation of encoded samples; `samples` indexes it so a saved
/// window can be sliced back into muxer samples.
#[derive(Debug, Clone)]
pub struct Segment {
    /// True when the segment begins with a keyframe (IDR). Saved clips must
    /// start at such a segment so they decode cleanly.
    pub starts_with_keyframe: bool,
    /// Presentation start, seconds since recording t0.
    pub pts_start_s: f64,
    pub duration_s: f64,
    pub data: Vec<u8>,
    pub samples: Vec<SampleInfo>,
}

impl Segment {
    pub fn pts_end_s(&self) -> f64 {
        self.pts_start_s + self.duration_s
    }

    /// Iterate `data` sliced per the sample index.
    pub fn sample_slices(&self) -> impl Iterator<Item = &[u8]> {
        let mut offset = 0usize;
        self.samples.iter().map(move |s| {
            let start = offset;
            offset += s.size as usize;
            &self.data[start..offset]
        })
    }
}
```

Also export `SampleInfo` from `crates/clipline-buffer/src/lib.rs`:
```rust
pub use segment::{SampleInfo, Segment};
```

And in `ring.rs` tests, the `seg()` helper gains the field:
```rust
    fn seg(pts: f64, dur: f64, bytes: usize, key: bool) -> Segment {
        Segment {
            starts_with_keyframe: key,
            pts_start_s: pts,
            duration_s: dur,
            data: vec![0u8; bytes],
            samples: Vec::new(),
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-buffer`
Expected: 9 tests pass (8 prior + 1 new).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(buffer): per-sample index on segments for re-muxing saved windows

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Recorder pipeline (capture → encode → GOP segments → ring)

**Files:**
- Create: `crates/clipline-capture/src/pipeline.rs`
- Modify: `crates/clipline-capture/src/lib.rs`
- Test: inline `#[cfg(test)]` in `pipeline.rs`

- [ ] **Step 1: Write the failing tests**

Add to `lib.rs`:
```rust
pub mod pipeline;

pub use pipeline::{PipelineError, Recorder};
```

`pipeline.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockCapture, MockEncoder};

    #[test]
    fn groups_packets_into_gop_aligned_segments() {
        // 90 frames at 30 fps, GOP 30 → exactly 3 keyframe-led segments.
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring();
        assert_eq!(ring.len(), 3);
        for seg in ring.segments() {
            assert!(seg.starts_with_keyframe);
            assert_eq!(seg.samples.len(), 30);
            assert!((seg.duration_s - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn byte_budget_evicts_oldest_gop() {
        // Each MockEncoder sample is 64–70 bytes → a GOP of 30 ≈ ~2 KB.
        // Budget for ~2 GOPs: the first of three must be evicted.
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            4 * 1024,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring();
        assert_eq!(ring.len(), 2);
        let first = ring.segments().next().unwrap();
        assert!((first.pts_start_s - 1.0).abs() < 1e-6, "GOP at t=0 evicted");
    }

    #[test]
    fn trailing_partial_gop_is_sealed_at_end() {
        // 45 frames, GOP 30 → one full GOP + one 15-frame partial.
        let mut rec = Recorder::new(
            MockCapture::new(45, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let counts: Vec<usize> =
            rec.ring().segments().map(|s| s.samples.len()).collect();
        assert_eq!(counts, vec![30, 15]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: COMPILE ERROR (`Recorder` not defined).

- [ ] **Step 3: Write the implementation** (top of `pipeline.rs`)

```rust
use clipline_buffer::{ReplayRing, SampleInfo, Segment};

use crate::traits::{CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Encode(#[from] EncodeError),
}

/// The recording pipeline (ddoc §3): capture → encode → GOP-aligned
/// segments → replay ring. Synchronous pull loop; production runs it on a
/// dedicated thread.
pub struct Recorder<C: CaptureEngine, E: Encoder> {
    capture: C,
    encoder: E,
    ring: ReplayRing,
    pending: Vec<EncodedPacket>,
}

impl<C: CaptureEngine, E: Encoder> Recorder<C, E> {
    pub fn new(capture: C, encoder: E, max_buffer_bytes: usize) -> Self {
        Self {
            capture,
            encoder,
            ring: ReplayRing::new(max_buffer_bytes),
            pending: Vec::new(),
        }
    }

    /// Drive the loop until the capture source ends, sealing a segment at
    /// every GOP boundary (a keyframe closes the previous GOP).
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while let Some(frame) = self.capture.next_frame()? {
            for pkt in self.encoder.encode(&frame)? {
                if pkt.is_keyframe && !self.pending.is_empty() {
                    self.seal_pending();
                }
                self.pending.push(pkt);
            }
        }
        if !self.pending.is_empty() {
            self.seal_pending();
        }
        Ok(())
    }

    pub fn ring(&self) -> &ReplayRing {
        &self.ring
    }

    pub fn encoder(&self) -> &E {
        &self.encoder
    }

    fn seal_pending(&mut self) {
        let packets = std::mem::take(&mut self.pending);
        let pts_start_s = packets[0].pts_s;
        let duration_s: f64 = packets.iter().map(|p| p.duration_s).sum();
        let starts_with_keyframe = packets[0].is_keyframe;
        let mut data = Vec::new();
        let mut samples = Vec::with_capacity(packets.len());
        for p in &packets {
            samples.push(SampleInfo {
                size: p.data.len() as u32,
                duration_s: p.duration_s,
                is_sync: p.is_keyframe,
            });
            data.extend_from_slice(&p.data);
        }
        self.ring.push(Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
        });
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-capture`
Expected: 11 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): Recorder pipeline grouping packets into GOP segments

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Save Replay → Hybrid MP4, end-to-end with ffprobe

**Files:**
- Modify: `crates/clipline-capture/src/pipeline.rs` (add `save_replay`)
- Test: `crates/clipline-capture/tests/end_to_end.rs`

- [ ] **Step 1: Write the failing test**

`tests/end_to_end.rs`:
```rust
use std::io::Cursor;
use std::process::Command;

use clipline_capture::{MockCapture, MockEncoder, Recorder};
use clipline_mp4::walker::{find, walk};

fn recorder_with_3s_of_footage() -> Recorder<MockCapture, MockEncoder> {
    let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), usize::MAX);
    rec.run_to_end().unwrap();
    rec
}

#[test]
fn save_replay_produces_a_standard_mp4_of_the_window() {
    let rec = recorder_with_3s_of_footage();
    // Save the trailing 2 s → GOPs at t=1.0 and t=2.0 → 60 samples.
    let (buf, end_pts) = rec
        .save_replay(Cursor::new(Vec::new()), 2.0, None)
        .map(|(w, end)| (w.into_inner(), end))
        .unwrap();

    assert!((end_pts - 3.0).abs() < 1e-6);
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);
    // First saved sample is frame 30 (the GOP at t=1.0): "F000030".
    assert!(buf.windows(7).any(|w| w == b"F000030"), "window starts at frame 30");
    assert!(!buf.windows(7).any(|w| w == b"F000029"), "frame 29 excluded");
}

#[test]
fn smart_mode_skips_already_saved_footage() {
    let rec = recorder_with_3s_of_footage();
    let (_, end) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    // Nothing new since the last save → empty result, no file content.
    let res = rec.save_replay(Cursor::new(Vec::new()), 2.0, Some(end));
    assert!(res.is_err(), "saving zero new footage is an error, not a silent empty file");
}

#[test]
fn ffprobe_accepts_the_saved_replay() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let rec = recorder_with_3s_of_footage();
    let (w, _) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    let buf = w.into_inner();

    let path = std::env::temp_dir().join("clipline_e2e_replay.mp4");
    std::fs::write(&path, &buf).unwrap();
    let out = Command::new(&ffprobe)
        .args([
            "-v", "error",
            "-show_entries", "stream=codec_name,nb_frames",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("codec_name=h264"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");
    std::fs::remove_file(&path).ok();
}

fn ffprobe_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let local = std::path::Path::new(&home).join("bin/ffprobe");
    if local.exists() {
        return Some(local);
    }
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| std::path::Path::new(d).join("ffprobe"))
        .find(|p| p.exists())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p clipline-capture --test end_to_end`
Expected: COMPILE ERROR (`save_replay` not defined).

- [ ] **Step 3: Write the implementation** (append to `pipeline.rs`)

Add imports at the top:
```rust
use std::io::{self, Seek, Write};

use clipline_mp4::{FragSample, HybridMp4Writer};
```

Add to `impl<C: CaptureEngine, E: Encoder> Recorder<C, E>`:
```rust
    /// Save the trailing `window_s` seconds as a finalized Hybrid MP4
    /// written to `w` (ddoc §6). `exclude_before_s` is the smart
    /// no-overlap mode. Returns the writer and the end pts of the saved
    /// footage — pass it back as `exclude_before_s` next time.
    ///
    /// Erroring (rather than writing an empty file) when no new footage
    /// exists lets the hotkey handler tell the user "nothing new to save".
    pub fn save_replay<W: Write + Seek>(
        &self,
        w: W,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> io::Result<(W, f64)> {
        let segments = self.ring.save_window(window_s, exclude_before_s);
        if segments.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "no new footage in window"));
        }
        let cfg = self.encoder.track_config();
        let timescale = cfg.timescale as f64;
        let mut writer = HybridMp4Writer::new(w, cfg)?;
        for seg in &segments {
            let samples: Vec<FragSample> = seg
                .sample_slices()
                .zip(&seg.samples)
                .map(|(slice, info)| FragSample {
                    data: slice.to_vec(),
                    duration: (info.duration_s * timescale).round() as u32,
                    is_sync: info.is_sync,
                })
                .collect();
            writer.write_fragment(&samples)?;
        }
        let end_pts = segments.last().expect("non-empty").pts_end_s();
        Ok((writer.finalize()?, end_pts))
    }
```

- [ ] **Step 4: Run all tests**

Run: `~/.cargo/bin/cargo test --workspace`
Expected: all green, including the three end-to-end tests (ffprobe one validates h264/60-frame/2.0s output for real).

- [ ] **Step 5: Clippy + commit**

Run: `~/.cargo/bin/cargo clippy --workspace --all-targets`

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): save_replay — ring window re-muxed to finalized hybrid MP4

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

## Out of scope (follow-ups)

- `#[cfg(windows)]` WGC capture + NVENC/AMF/QSV encoder implementations via windows-rs/FFmpeg (ddoc §3/§4) — needs Windows hardware/CI.
- Audio capture trait + WASAPI loopback (ddoc §10) and audio track muxing.
- Continuous-recording sink (manual full recording, ddoc Goal 2) alongside the ring — same encoded stream, second consumer.
- Tauri shell wiring hotkey → `save_replay`.
