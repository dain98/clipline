# Clipline Audio Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Audio flows end-to-end through the recording architecture (ddoc §3/§6/§10): an `AudioSource` trait (encoded-packet pull model; the Windows WASAPI+Opus implementation will live behind it), audio attached to GOP-aligned segments in the replay ring, and `save_replay` emitting a two-track (h264 + opus) Hybrid MP4 that ffprobe validates.

**Architecture:** (1) `clipline-buffer::Segment` gains `audio: Vec<TrackSamples>` — per-audio-track `(data, samples)` pairs riding alongside the video payload; ring byte-accounting switches to `byte_len()` (video + audio). Video alone still drives GOP alignment and save-window selection. (2) `clipline-capture` gains `AudioPacket` + `AudioSource` (a `poll_packets(until_pts_s)` drain model matching how WASAPI buffers are consumed) and a deterministic `MockAudioSource`. (3) `Recorder` accepts N boxed audio sources via a `with_audio` builder; the run loop drains each source per video frame into pending buffers, and sealing a GOP at boundary `t` takes every packet ending at or before `t`. (4) `save_replay` muxes `[video, audio…]` via `new_multi`/`write_fragment_multi`.

**Tech Stack:** unchanged. Boxed trait objects for audio sources (heterogeneous, low rate).

**Environment notes:** `cargo` at `~/.cargo/bin/cargo`. Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Multi-track segments in `clipline-buffer`

**Files:**
- Modify: `crates/clipline-buffer/src/segment.rs`, `crates/clipline-buffer/src/ring.rs`, `crates/clipline-buffer/src/lib.rs`
- Test: extend `#[cfg(test)]` in `segment.rs` and `ring.rs`

- [ ] **Step 1: Write the failing tests**

Append to `segment.rs` tests:
```rust
    #[test]
    fn byte_len_counts_video_and_audio() {
        let seg = Segment {
            starts_with_keyframe: true,
            pts_start_s: 0.0,
            duration_s: 1.0,
            data: vec![0; 100],
            samples: vec![],
            audio: vec![
                TrackSamples { data: vec![0; 30], samples: vec![] },
                TrackSamples { data: vec![0; 20], samples: vec![] },
            ],
        };
        assert_eq!(seg.byte_len(), 150);
    }

    #[test]
    fn track_samples_slice_like_segments() {
        let t = TrackSamples {
            data: b"XXYYY".to_vec(),
            samples: vec![
                SampleInfo { size: 2, duration_s: 0.02, is_sync: true },
                SampleInfo { size: 3, duration_s: 0.02, is_sync: true },
            ],
        };
        let slices: Vec<&[u8]> = t.sample_slices().collect();
        assert_eq!(slices, vec![b"XX".as_slice(), b"YYY".as_slice()]);
    }
```

Append to `ring.rs` tests:
```rust
    #[test]
    fn eviction_counts_audio_bytes() {
        let mut ring = ReplayRing::new(250);
        let mut s1 = seg(0.0, 2.0, 50, true);
        s1.audio.push(crate::segment::TrackSamples { data: vec![0; 60], samples: vec![] });
        let mut s2 = seg(2.0, 2.0, 50, true);
        s2.audio.push(crate::segment::TrackSamples { data: vec![0; 60], samples: vec![] });
        let mut s3 = seg(4.0, 2.0, 50, true);
        s3.audio.push(crate::segment::TrackSamples { data: vec![0; 60], samples: vec![] });
        ring.push(s1);
        ring.push(s2);
        ring.push(s3); // 330 bytes total > 250 → evict front
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.bytes(), 220);
    }
```

- [ ] **Step 2: Run to verify failure** (`audio` field / `TrackSamples` / `byte_len` missing)

- [ ] **Step 3: Implement**

`segment.rs`: add `TrackSamples`, the `audio` field, `byte_len`, and share the slicing logic:
```rust
/// One track's worth of encoded samples: opaque concatenated `data`
/// indexed by `samples`.
#[derive(Debug, Clone, Default)]
pub struct TrackSamples {
    pub data: Vec<u8>,
    pub samples: Vec<SampleInfo>,
}

impl TrackSamples {
    /// Iterate `data` sliced per the sample index.
    pub fn sample_slices(&self) -> impl Iterator<Item = &[u8]> {
        slice_samples(&self.data, &self.samples)
    }
}

fn slice_samples<'a>(
    data: &'a [u8],
    samples: &'a [SampleInfo],
) -> impl Iterator<Item = &'a [u8]> {
    let mut offset = 0usize;
    samples.iter().map(move |s| {
        let start = offset;
        offset += s.size as usize;
        &data[start..offset]
    })
}
```
`Segment` gains the field (after `samples`):
```rust
    /// Audio tracks riding alongside this video GOP (ddoc §10 multi-track).
    pub audio: Vec<TrackSamples>,
```
`Segment::byte_len`:
```rust
    /// Total payload bytes across video and all audio tracks — the unit of
    /// ring byte-accounting.
    pub fn byte_len(&self) -> usize {
        self.data.len() + self.audio.iter().map(|t| t.data.len()).sum::<usize>()
    }
```
`Segment::sample_slices` body becomes `slice_samples(&self.data, &self.samples)`.

`ring.rs`: replace the three `seg.data.len()` / `front.data.len()` accounting sites with `byte_len()` (`push` adds `seg.byte_len()`, eviction subtracts `front.byte_len()`), and the test helper `seg()` gains `audio: Vec::new()`.

`lib.rs`: `pub use segment::{SampleInfo, Segment, TrackSamples};`

Update `clipline-capture/src/pipeline.rs::seal_pending` to include `audio: Vec::new()` in the `Segment` literal (compile fix; real audio arrives in Task 3).

- [ ] **Step 4: Run the workspace** — all pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(buffer): multi-track segments with audio byte accounting

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: AudioSource trait + MockAudioSource

**Files:**
- Modify: `crates/clipline-capture/src/traits.rs`, `crates/clipline-capture/src/mock.rs`, `crates/clipline-capture/src/lib.rs`
- Test: extend `#[cfg(test)]` in `mock.rs`

- [ ] **Step 1: Write the failing tests** (append to `mock.rs` tests)

```rust
    #[test]
    fn mock_audio_source_drains_packets_up_to_pts() {
        let mut src = MockAudioSource::new(48_000, 20);
        // Packets are 20 ms; "up to 0.05 s" = the two ending at 0.02/0.04.
        let batch = src.poll_packets(0.05).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].pts_s, 0.0);
        assert!((batch[1].pts_s - 0.02).abs() < 1e-9);
        assert!((batch[0].duration_s - 0.02).abs() < 1e-9);
        // Next drain continues where the last left off; packet at 0.04
        // (ending 0.06) arrives once 0.06 is reachable.
        let batch2 = src.poll_packets(0.06).unwrap();
        assert_eq!(batch2.len(), 1);
        assert!((batch2[0].pts_s - 0.04).abs() < 1e-9);
    }

    #[test]
    fn mock_audio_source_has_a_muxable_config() {
        let src = MockAudioSource::new(48_000, 20);
        let cfg = src.track_config();
        assert_eq!(cfg.sample_rate, 48_000);
        assert_eq!(cfg.channels, 2);
    }
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

`traits.rs` additions:
```rust
use clipline_mp4::AudioTrackConfig;

/// One encoded audio packet (e.g. a 20 ms Opus frame).
#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub data: Vec<u8>,
    /// Seconds since capture start, same timebase as video frames.
    pub pts_s: f64,
    pub duration_s: f64,
}

/// An encoded-audio producer (ddoc §10: WASAPI loopback / per-process /
/// mic, each composed with an Opus encoder behind this trait). Drain
/// model: return every packet that ends at or before `until_pts_s`.
pub trait AudioSource {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError>;
    /// Track parameters for muxing this source's stream.
    fn track_config(&self) -> AudioTrackConfig;
}
```

`mock.rs` additions:
```rust
/// Deterministic audio source: fixed-size packets every `packet_ms`.
pub struct MockAudioSource {
    sample_rate: u32,
    packet_ms: u32,
    next_index: u64,
}

impl MockAudioSource {
    pub fn new(sample_rate: u32, packet_ms: u32) -> Self {
        Self { sample_rate, packet_ms, next_index: 0 }
    }
}

impl AudioSource for MockAudioSource {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        let dur = self.packet_ms as f64 / 1000.0;
        let mut out = Vec::new();
        loop {
            let pts = self.next_index as f64 * dur;
            if pts + dur > until_pts_s + 1e-9 {
                break;
            }
            let mut data = format!("P{:05}", self.next_index).into_bytes();
            data.resize(40, 0xAA);
            out.push(AudioPacket { data, pts_s: pts, duration_s: dur });
            self.next_index += 1;
        }
        Ok(out)
    }

    fn track_config(&self) -> AudioTrackConfig {
        AudioTrackConfig { channels: 2, sample_rate: self.sample_rate, pre_skip: 312 }
    }
}
```
(plus `AudioSource`/`AudioPacket`/`AudioTrackConfig` imports and `lib.rs` re-exports: `MockAudioSource`, `AudioPacket`, `AudioSource`.)

- [ ] **Step 4: Run tests** — pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): AudioSource trait with deterministic mock

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Audio through the Recorder

**Files:**
- Modify: `crates/clipline-capture/src/pipeline.rs`
- Test: extend `#[cfg(test)]` in `pipeline.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn audio_packets_land_in_their_gop_segments() {
        use crate::mock::MockAudioSource;
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        let ring = rec.ring();
        assert_eq!(ring.len(), 3);
        for (i, seg) in ring.segments().enumerate() {
            assert_eq!(seg.audio.len(), 1, "one audio track");
            // 1 s GOP at 20 ms packets = 50 packets per segment.
            assert_eq!(seg.audio[0].samples.len(), 50, "segment {i}");
        }
        // First packet of the second segment starts at its GOP boundary.
        let seg2 = ring.segments().nth(1).unwrap();
        assert_eq!(&seg2.audio[0].data[..6], b"P00050");
    }
```

- [ ] **Step 2: Run to verify failure** (`with_audio` missing)

- [ ] **Step 3: Implement** (in `pipeline.rs`)

`Recorder` gains fields:
```rust
    audio_sources: Vec<Box<dyn AudioSource>>,
    pending_audio: Vec<Vec<AudioPacket>>,
```
(`new` initializes both to empty; import `AudioPacket`, `AudioSource`, and `clipline_buffer::TrackSamples`.)

Builder:
```rust
    /// Attach an audio source as the next audio track (ddoc §10:
    /// game / mic / system).
    pub fn with_audio(mut self, source: Box<dyn AudioSource>) -> Self {
        self.audio_sources.push(source);
        self.pending_audio.push(Vec::new());
        self
    }
```

`run_to_end` drains audio per frame and passes seal boundaries:
```rust
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while let Some(frame) = self.capture.next_frame()? {
            for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
                pending.extend(src.poll_packets(frame.pts_s)?);
            }
            for pkt in self.encoder.encode(&frame)? {
                if pkt.is_keyframe && !self.pending.is_empty() {
                    self.seal_pending(pkt.pts_s);
                }
                self.pending.push(pkt);
            }
        }
        if !self.pending.is_empty() {
            // Drain any audio still buffered in the sources to the end of
            // the final GOP, then seal everything.
            let end = self.pending.last().map(|p| p.pts_s + p.duration_s).unwrap_or(0.0);
            for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
                pending.extend(src.poll_packets(end)?);
            }
            self.seal_pending(f64::INFINITY);
        }
        Ok(())
    }
```

`seal_pending(boundary_pts_s: f64)` additionally splits each pending-audio buffer (packets ending at or before the boundary belong to this segment) and builds `TrackSamples`:
```rust
    fn seal_pending(&mut self, boundary_pts_s: f64) {
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

        let mut audio = Vec::with_capacity(self.pending_audio.len());
        for pending in &mut self.pending_audio {
            let split = pending
                .iter()
                .position(|p| p.pts_s + p.duration_s > boundary_pts_s + 1e-9)
                .unwrap_or(pending.len());
            let mut track = TrackSamples::default();
            for p in pending.drain(..split) {
                track.samples.push(SampleInfo {
                    size: p.data.len() as u32,
                    duration_s: p.duration_s,
                    is_sync: true, // every Opus packet is independently decodable
                });
                track.data.extend_from_slice(&p.data);
            }
            audio.push(track);
        }

        self.ring.push(Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
            audio,
        });
    }
```

- [ ] **Step 4: Run tests** — all pipeline tests pass (audio-less tests get empty `audio` vecs).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): audio sources drained into GOP segments

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Two-track save_replay + ffprobe e2e

**Files:**
- Modify: `crates/clipline-capture/src/pipeline.rs` (`save_replay`)
- Test: extend `crates/clipline-capture/tests/end_to_end.rs`

- [ ] **Step 1: Write the failing test** (append to `end_to_end.rs`)

```rust
#[test]
fn ffprobe_accepts_a_video_plus_audio_replay() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), usize::MAX)
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
    rec.run_to_end().unwrap();
    let (w, _) = rec.save_replay(Cursor::new(Vec::new()), 2.0, None).unwrap();
    let buf = w.into_inner();

    let path = std::env::temp_dir().join("clipline_e2e_av.mp4");
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
    assert!(stdout.contains("codec_name=opus"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=100"), "got: {stdout}");
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");
    std::fs::remove_file(&path).ok();
}
```
(plus `MockAudioSource` in the imports at the top.)

- [ ] **Step 2: Run to verify failure** (single-track writer in save_replay → opus/nb_frames=100 assertions fail or compile error on imports)

- [ ] **Step 3: Update `save_replay`**

Replace the writer construction and fragment loop:
```rust
        let video_cfg = self.encoder.track_config();
        let video_ts = video_cfg.timescale as f64;
        let mut track_cfgs = vec![TrackConfig::Video(video_cfg)];
        let audio_cfgs: Vec<_> = self.audio_sources.iter().map(|s| s.track_config()).collect();
        for cfg in &audio_cfgs {
            track_cfgs.push(TrackConfig::Audio(cfg.clone()));
        }
        let mut writer = HybridMp4Writer::new_multi(w, track_cfgs)?;
        for seg in &segments {
            let video: Vec<FragSample> = seg
                .sample_slices()
                .zip(&seg.samples)
                .map(|(slice, info)| FragSample {
                    data: slice.to_vec(),
                    duration: (info.duration_s * video_ts).round() as u32,
                    is_sync: info.is_sync,
                })
                .collect();
            let mut per_track: Vec<Vec<FragSample>> = vec![video];
            for (track, cfg) in seg.audio.iter().zip(&audio_cfgs) {
                let ts = cfg.sample_rate as f64;
                per_track.push(
                    track
                        .sample_slices()
                        .zip(&track.samples)
                        .map(|(slice, info)| FragSample {
                            data: slice.to_vec(),
                            duration: (info.duration_s * ts).round() as u32,
                            is_sync: info.is_sync,
                        })
                        .collect(),
                );
            }
            let slices: Vec<&[FragSample]> = per_track.iter().map(|v| v.as_slice()).collect();
            writer.write_fragment_multi(&slices)?;
        }
```
(import `TrackConfig`; keep the empty-window error and end-pts return unchanged. Note: when a segment predates `with_audio` it has fewer audio tracks than sources — `zip` simply emits nothing for the missing ones, and `write_fragment_multi` then needs the right slice count, so pad `per_track` with empty vecs up to `1 + audio_cfgs.len()` after the loop:)
```rust
            per_track.resize_with(1 + audio_cfgs.len(), Vec::new);
```

- [ ] **Step 4: Run the full workspace + clippy** — everything green; the original video-only e2e tests still pass (no audio sources → single-track path through `new_multi` with one track).

- [ ] **Step 5: Commit and push**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(capture): save_replay emits video+audio hybrid MP4

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
git push
```

---

## Out of scope (follow-ups)

- WASAPI loopback / per-process loopback / mic `AudioSource` implementations + a real Opus encoder (Windows milestone).
- A/V sync against a shared QPC timebase on real capture clocks (ddoc §6 "Clocking & A/V sync") — mocks share an ideal clock.
- Tauri shell and editor.
