# Clipline A/V Sync Hardening (Windows Platform Layer, Part 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ddoc §6 "Clocking & A/V sync" — M0-core, not polish: *"the muxer derives PTS from
these stamps rather than assuming a fixed frame cadence."* Three gaps close: (1) the MP4 video
timeline is built from encoder-echoed durations, not capture stamps — under VRR/irregular
frame delivery the file timeline drifts from reality; (2) the shared QPC origin between video
and audio is hand-wired in the smoke instead of enforced by the engine API; (3) the
GOP-boundary discipline the mock tests pinned exactly has no real-clock counterpart
(tolerances, not exact equality — handoff milestone 4).

**Architecture:** All neutral-first. `pipeline.rs` re-derives every video sample's duration at
seal time from successive packet pts — `duration[i] = pts[i+1] − pts[i]`, the sealing
keyframe's pts closes the last sample of the previous GOP (the boundary is exact, no estimate),
and only the final seal (boundary = ∞) falls back to the encoder's own duration. `avsync.rs`
is a tolerance-based validator over sealed segments — keyframe-led, video pts/duration
continuity, per-segment audio coverage, cumulative cross-track drift — returning a
`SyncReport` (max gaps/drift) or a `SyncViolation`. Windows side: `WgcCapture` constructors
take the `RelativeClock` explicitly (`*_on(device, clock)`), so one origin is the API's
natural shape rather than a smoke-test convention; a real-clock device test runs WGC + WASAPI
on one clock and feeds the validator.

**Tech Stack:** no new dependencies.

**Environment notes:** device tests CI-skip as established. Commits end with
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: derive video sample durations from pts at seal

**Files:** `crates/clipline-capture/src/pipeline.rs`

- [ ] **Step 1: failing test** (`pipeline.rs` tests)

```rust
    /// Encoder echoing nominal durations while pts jitters (VRR-style):
    /// the sealed timeline must follow the STAMPS (ddoc §6), not the echo.
    struct JitteryEncoder {
        inner: MockEncoder,
    }

    impl Encoder for JitteryEncoder {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut pkts = self.inner.encode(frame)?;
            for p in &mut pkts {
                // Stamps: frames alternate 10 ms / 30 ms apart (avg 50 fps),
                // while the encoder still claims a flat 1/30 s duration.
                let idx = (p.pts_s * 30.0).round();
                p.pts_s = (idx / 2.0).floor() * 0.04 + if idx % 2.0 == 1.0 { 0.01 } else { 0.0 };
            }
            Ok(pkts)
        }
        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }
    }

    #[test]
    fn sealed_durations_come_from_pts_deltas_not_encoder_claims() {
        // GOP of 4 over 8 frames → two segments, boundary at frame 4.
        let enc = JitteryEncoder { inner: MockEncoder::new(4, 30) };
        let mut rec = Recorder::new(MockCapture::new(8, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        let segs: Vec<_> = rec.ring().segments().collect();
        assert_eq!(segs.len(), 2);
        // Within a GOP: 10/30/10 ms gaps, NOT the encoder's flat 33.3 ms.
        let d: Vec<f64> = segs[0].samples.iter().map(|s| s.duration_s).collect();
        assert!((d[0] - 0.01).abs() < 1e-9, "got {d:?}");
        assert!((d[1] - 0.03).abs() < 1e-9);
        assert!((d[2] - 0.01).abs() < 1e-9);
        // Boundary: last sample of GOP 1 closes exactly at GOP 2's keyframe.
        let gop2_start = segs[1].pts_start_s;
        assert!((segs[0].pts_end_s() - gop2_start).abs() < 1e-9, "no gap, no overlap");
        // Final seal falls back to the encoder duration for the last sample.
        let last = segs[1].samples.last().unwrap();
        assert!((last.duration_s - 1.0 / 30.0).abs() < 1e-9);
    }
```

- [ ] **Step 2: verify failure → Step 3: implement** — in `seal_pending`, before building
`SampleInfo`s, recompute durations:

```rust
        // ddoc §6: the timeline follows capture stamps, not encoder cadence
        // claims. Each sample lasts until the next pts; the sealing
        // keyframe's pts closes the GOP exactly; only the final seal
        // (boundary = ∞) trusts the encoder's own duration.
        let mut durations: Vec<f64> = Vec::with_capacity(packets.len());
        for i in 0..packets.len() {
            let next_pts = packets
                .get(i + 1)
                .map(|p| p.pts_s)
                .unwrap_or(if boundary_pts_s.is_finite() { boundary_pts_s } else { f64::NAN });
            let d = if next_pts.is_nan() {
                packets[i].duration_s
            } else {
                (next_pts - packets[i].pts_s).max(1e-4)
            };
            durations.push(d);
        }
```
…and use `durations[i]` for `SampleInfo.duration_s` plus `duration_s = durations.sum()` for
the segment. (Existing tests stay green: mock pts deltas equal the mock durations.)

- [ ] **Step 4: all tests pass → Step 5: commit**
`feat(capture): video timeline derives from capture stamps at seal (ddoc §6)`.

---

### Task 2: timeline sync validator

**Files:** Create `crates/clipline-capture/src/avsync.rs`; modify `lib.rs`.

API:
```rust
pub struct SyncTolerances {
    /// Max allowed |gap| between consecutive segments (s).
    pub max_video_gap_s: f64,        // default 0.005
    /// Max |audio coverage − segment duration| per segment (s).
    pub max_audio_segment_skew_s: f64, // default 0.045 (2 opus frames + jitter)
    /// Max |cumulative audio − cumulative video| at end (s).
    pub max_total_drift_s: f64,      // default 0.045
}

#[derive(Debug)]
pub struct SyncReport {
    pub segments: usize,
    pub video_duration_s: f64,
    pub audio_duration_s: Vec<f64>, // per track
    pub max_video_gap_s: f64,
    pub max_audio_segment_skew_s: f64,
    pub total_drift_s: Vec<f64>,    // per track
}

#[derive(Debug, thiserror::Error)]
pub enum SyncViolation { /* NotKeyframeLed{seg}, VideoGap{seg, gap_s},
    AudioSegmentSkew{seg, track, skew_s}, TotalDrift{track, drift_s} */ }

pub fn validate_timeline(
    segments: &[&Segment], tol: &SyncTolerances,
) -> Result<SyncReport, SyncViolation>
```

Checks per segment: `starts_with_keyframe`; video continuity
`|seg[k+1].pts_start − seg[k].pts_end| ≤ max_video_gap`; per audio track
`|Σ durations − seg.duration| ≤ max_audio_segment_skew` **except** the final segment’s tail
(audio may legitimately end up to one poll behind video — only check overshoot there).
Globally per track: `|Σ audio − Σ video| ≤ max_total_drift` (drift may be negative: audio
short). Report carries the maxima so smokes can print them.

- [ ] **Step 1: failing tests** — happy path via a real mock `Recorder` run
(`MockCapture(90,30)+MockEncoder(30,30)+MockAudioSource(48_000,20)` → validate Ok, report
numbers ≈ 0); violation paths via hand-built segments (non-keyframe-led; a 50 ms inter-segment
gap; an audio track 200 ms short mid-segment).
- [ ] **Step 2: verify failure → Step 3: implement → Step 4: pass → Step 5: commit**
`feat(capture): tolerance-based A/V timeline validator`.

---

### Task 3: shared-clock API + real-clock device test

**Files:** `crates/clipline-capture/src/windows/wgc.rs`, `examples/record_smoke.rs`,
`examples/wgc_smoke.rs` (signature ripple).

- [ ] `WgcCapture::primary_monitor_on(device, clock)` / `for_window_on(device, hwnd, clock)` —
the clock becomes a required parameter on the `_on` constructors (the convenience
constructors mint their own from `qpc_now_ticks_100ns()`). `clock()` getter stays. Update
callers; the existing caller-provided-device test now also asserts the engine uses the
provided clock (first frame pts ≈ small).
- [ ] **Real-clock sync test** (`wgc.rs` or a new `windows/avsync_test`, CI-skipped): one
clock → `WgcCapture::primary_monitor_on` + `WasapiLoopback::start` → `Recorder` +
`LimitedCapture` (~60 frames) → `validate_timeline` with default tolerances must pass; print
the report. This is the mock discipline reproduced on real hardware.
- [ ] Commit: `feat(capture): clock is an explicit shared parameter; real-clock sync test`.

---

### Task 4: smoke sync report, gates, handoff

- [ ] `record_smoke`: after saving, run `validate_timeline` over the saved window's segments
and print the `SyncReport` (max video gap, per-track drift). Run with `--audio` for real;
numbers must be within tolerances.
- [ ] `cargo test --workspace` (ffprobe on PATH), `cargo clippy --workspace --all-targets`,
push, CI green both OSes.
- [ ] `handoff.md`: milestone 4 done — **the M0 Windows platform layer is complete**; next
frontier is per ddoc §15 (FFmpeg encoder matrix, per-process audio, Tauri shell / hotkey →
`save_replay`).

---

## Out of scope (follow-ups)

- Audio-device-clock vs QPC drift *correction* (resample/skew) for hour-long sessions —
  measured by the report now, corrected later.
- Frame pacing for idle screens (repeat-last-frame), per-process loopback, mic track.
- An `avsync` check inside `save_replay` itself (fail-fast on desynced saves) — once the
  validator has soaked.
