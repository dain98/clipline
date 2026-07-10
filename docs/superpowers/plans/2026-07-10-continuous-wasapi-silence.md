# Continuous WASAPI Silence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep enabled Clipline audio tracks continuous when WASAPI produces no buffers during completely quiet endpoint intervals.

**Architecture:** Extend the neutral `LoopbackAssembler` so it can advance its absolute shared-clock timeline with silence and trim real chunks that arrive after synthesized time. Then wire finite WASAPI polls to advance through one Opus frame before the requested video PTS, leaving a 20 ms delivery allowance while preserving `f64::MAX` monitor-drain behavior.

**Tech Stack:** Rust, WASAPI, shared QPC `RelativeClock`, 48 kHz stereo PCM, 20 ms Opus frames, Cargo tests and Clippy.

## Global Constraints

- Keep every enabled audio track continuous through quiet endpoint intervals.
- Preserve the shared QPC-derived timeline used by video and audio.
- Allow one 20 ms Opus frame for normal WASAPI delivery latency.
- Prevent late device buffers from duplicating or shifting time already represented by synthesized silence.
- Do not relax A/V validation tolerances or skip empty audio tracks.
- Do not add a synthetic render client merely to keep an endpoint active.
- Do not make the hardware test play a tone; it must remain valid on a quiet desktop.
- Do not change the shared clock, GOP duration, Opus frame size, or MP4 layout.
- Keep neutral timeline logic in `pcm.rs`; Windows-only code stays behind the existing Windows module boundary.
- Follow strict TDD: observe every new test fail for the intended reason before writing its implementation.

---

### Task 1: Neutral continuous PCM timeline

**Files:**
- Modify: `crates/clipline-capture/src/pcm.rs:19-75`
- Test: `crates/clipline-capture/src/pcm.rs:294-405`

**Interfaces:**
- Consumes: `LoopbackAssembler`, `SAMPLE_RATE`, `GAP_TOLERANCE_S`, and `MAX_GAP_FILL_S` already defined in `pcm.rs`.
- Produces: `pub fn advance_with_silence(&mut self, target_pts_s: f64)` and overlap-safe behavior in the existing `pub fn push_chunk(&mut self, pts_s: f64, interleaved: &[f32])`.

- [ ] **Step 1: Add failing tests for advancement through device silence**

Add these tests to the existing `pcm.rs` test module after `fills_gaps_with_silence`:

```rust
#[test]
fn anchored_assembler_advances_through_device_silence() {
    let mut asm = LoopbackAssembler::new();
    asm.push_chunk(0.0, &[]);

    asm.advance_with_silence(0.5);

    let mut frames = Vec::new();
    while let Some(frame) = asm.pop_frame() {
        frames.push(frame);
    }
    assert_eq!(frames.len(), 25);
    for (index, (pts_s, frame)) in frames.iter().enumerate() {
        assert!((*pts_s - index as f64 * FRAME_DURATION_S).abs() < 1e-9);
        assert!(frame.iter().all(|&sample| sample == 0.0));
    }
}

#[test]
fn silence_advancement_is_monotonic_and_idempotent() {
    let mut asm = LoopbackAssembler::new();
    asm.push_chunk(0.0, &[]);

    asm.advance_with_silence(0.04);
    asm.advance_with_silence(0.02);
    asm.advance_with_silence(0.04);

    assert_eq!(std::iter::from_fn(|| asm.pop_frame()).count(), 2);
}

#[test]
fn non_finite_silence_horizon_does_not_advance() {
    let mut asm = LoopbackAssembler::new();
    asm.push_chunk(0.0, &[]);

    asm.advance_with_silence(f64::INFINITY);
    asm.advance_with_silence(f64::NAN);

    assert!(asm.pop_frame().is_none());
}
```

- [ ] **Step 2: Run the advancement tests and verify RED**

Run:

```powershell
cargo test -p clipline-capture pcm::tests::anchored_assembler_advances_through_device_silence
cargo test -p clipline-capture pcm::tests::silence_advancement_is_monotonic_and_idempotent
cargo test -p clipline-capture pcm::tests::non_finite_silence_horizon_does_not_advance
```

Expected: each command fails to compile because `LoopbackAssembler::advance_with_silence` does not exist.

- [ ] **Step 3: Implement bounded monotonic silence advancement**

Add this method inside `impl LoopbackAssembler`, immediately before `push_contiguous_chunk`:

```rust
/// Extend an anchored timeline with stereo silence up to an absolute PTS.
/// Non-finite or non-forward targets are ignored. One call is bounded by
/// the same limit used for timestamp-discovered gaps.
pub fn advance_with_silence(&mut self, target_pts_s: f64) {
    if !target_pts_s.is_finite() {
        return;
    }
    let Some(expected_pts_s) = self.next_chunk_pts_s else {
        return;
    };
    let gap_s = target_pts_s - expected_pts_s;
    if gap_s <= 0.0 {
        return;
    }
    let missing_pairs = (gap_s.min(MAX_GAP_FILL_S) * SAMPLE_RATE).round() as usize;
    self.buffered
        .extend(std::iter::repeat_n(0.0, missing_pairs * 2));
    self.next_chunk_pts_s =
        Some(expected_pts_s + missing_pairs as f64 / SAMPLE_RATE);
}
```

- [ ] **Step 4: Run the advancement tests and verify GREEN**

Run:

```powershell
cargo test -p clipline-capture pcm::tests::anchored_assembler_advances_through_device_silence
cargo test -p clipline-capture pcm::tests::silence_advancement_is_monotonic_and_idempotent
cargo test -p clipline-capture pcm::tests::non_finite_silence_horizon_does_not_advance
```

Expected: all three tests pass.

- [ ] **Step 5: Add failing tests for late real-buffer overlap**

Add these tests after the advancement tests:

```rust
#[test]
fn late_chunk_keeps_only_suffix_after_synthesized_silence() {
    let mut asm = LoopbackAssembler::new();
    asm.push_chunk(0.0, &[]);
    asm.advance_with_silence(0.10);

    asm.push_chunk(0.08, &pairs(1_920, 0.75));

    let mut frames = Vec::new();
    while let Some(frame) = asm.pop_frame() {
        frames.push(frame);
    }
    assert_eq!(frames.len(), 6);
    assert!(frames[..5]
        .iter()
        .all(|(_, frame)| frame.iter().all(|&sample| sample == 0.0)));
    assert!((frames[5].0 - 0.10).abs() < 1e-9);
    assert!(frames[5].1.iter().all(|&sample| sample == 0.75));
}

#[test]
fn fully_overlapped_late_chunk_does_not_extend_timeline() {
    let mut asm = LoopbackAssembler::new();
    asm.push_chunk(0.0, &[]);
    asm.advance_with_silence(0.10);

    asm.push_chunk(0.04, &pairs(960, 0.75));

    assert_eq!(std::iter::from_fn(|| asm.pop_frame()).count(), 5);
}
```

- [ ] **Step 6: Run the overlap tests and verify RED**

Run:

```powershell
cargo test -p clipline-capture pcm::tests::late_chunk_keeps_only_suffix_after_synthesized_silence
cargo test -p clipline-capture pcm::tests::fully_overlapped_late_chunk_does_not_extend_timeline
```

Expected: the partial-overlap test reports seven frames or misplaced real samples, and the full-overlap test reports six frames because `push_chunk` currently appends late samples contiguously.

- [ ] **Step 7: Make `push_chunk` trim material overlaps**

Replace the existing `push_chunk` body with:

```rust
pub fn push_chunk(&mut self, pts_s: f64, interleaved: &[f32]) {
    if !pts_s.is_finite() {
        self.push_contiguous_chunk(interleaved);
        return;
    }
    self.base_pts_s.get_or_insert(pts_s);
    let expected = self.next_chunk_pts_s.unwrap_or(pts_s);
    let gap = pts_s - expected;
    let mut samples = interleaved;
    if gap > GAP_TOLERANCE_S {
        let missing_pairs = (gap.min(MAX_GAP_FILL_S) * SAMPLE_RATE).round() as usize;
        self.buffered
            .extend(std::iter::repeat_n(0.0, missing_pairs * 2));
    } else if gap < -GAP_TOLERANCE_S {
        let overlap_pairs = ((expected - pts_s) * SAMPLE_RATE).round() as usize;
        if overlap_pairs >= interleaved.len() / 2 {
            return;
        }
        samples = &interleaved[overlap_pairs * 2..];
    }
    self.buffered.extend_from_slice(samples);
    let chunk_duration_s = (samples.len() / 2) as f64 / SAMPLE_RATE;
    self.next_chunk_pts_s = Some(if gap > GAP_TOLERANCE_S {
        pts_s + chunk_duration_s
    } else {
        expected + chunk_duration_s
    });
}
```

- [ ] **Step 8: Run all neutral PCM tests**

Run:

```powershell
cargo test -p clipline-capture pcm::tests
```

Expected: all `pcm::tests` pass, including the pre-existing gap, jitter, resampling, and mixer coverage.

- [ ] **Step 9: Commit Task 1**

```powershell
git add crates/clipline-capture/src/pcm.rs
git commit -m "fix(capture): keep quiet PCM timeline continuous"
```

---

### Task 2: WASAPI finite poll integration

**Files:**
- Modify: `crates/clipline-capture/src/windows/wasapi.rs:457-468`
- Test: `crates/clipline-capture/src/windows/wasapi.rs:1310-1535`
- Verify unchanged integration test: `crates/clipline-capture/src/windows/wgc.rs:639-700`

**Interfaces:**
- Consumes: `LoopbackAssembler::advance_with_silence(target_pts_s: f64)` from Task 1 and `FRAME_DURATION_S` already imported from the Opus module.
- Produces: private pure helper `fn audio_poll_silence_horizon(until_pts_s: f64) -> Option<f64>` and finite-poll timeline advancement in `WasapiPcmCapture::poll_frames`.

- [ ] **Step 1: Add failing pure tests for the polling horizon**

Add these tests near the start of the existing `wasapi.rs` test module:

```rust
#[test]
fn audio_poll_horizon_leaves_one_opus_frame_for_delivery() {
    assert_eq!(audio_poll_silence_horizon(0.5), Some(0.48));
    assert_eq!(audio_poll_silence_horizon(0.01), Some(0.0));
}

#[test]
fn audio_poll_horizon_does_not_synthesize_for_monitor_drains() {
    assert_eq!(audio_poll_silence_horizon(f64::MAX), None);
    assert_eq!(audio_poll_silence_horizon(f64::INFINITY), None);
    assert_eq!(audio_poll_silence_horizon(f64::NAN), None);
}
```

- [ ] **Step 2: Run the horizon tests and verify RED**

Run:

```powershell
cargo test -p clipline-capture windows::wasapi::tests::audio_poll_horizon_leaves_one_opus_frame_for_delivery
cargo test -p clipline-capture windows::wasapi::tests::audio_poll_horizon_does_not_synthesize_for_monitor_drains
```

Expected: compilation fails because `audio_poll_silence_horizon` does not exist.

- [ ] **Step 3: Implement the pure 20 ms horizon helper**

Add this private helper immediately before `impl WasapiPcmCapture`:

```rust
fn audio_poll_silence_horizon(until_pts_s: f64) -> Option<f64> {
    (until_pts_s.is_finite() && until_pts_s != f64::MAX)
        .then(|| (until_pts_s - FRAME_DURATION_S).max(0.0))
}
```

- [ ] **Step 4: Run the horizon tests and verify GREEN**

Run the two commands from Step 2 again. Expected: both tests pass.

- [ ] **Step 5: Wire finite polls to advance quiet audio**

Change `WasapiPcmCapture::poll_frames` to:

```rust
fn poll_frames(&mut self, until_pts_s: f64) -> Result<Vec<PcmFrame>, CaptureError> {
    self.drain_device()?;
    if let Some(horizon_pts_s) = audio_poll_silence_horizon(until_pts_s) {
        self.assembler.advance_with_silence(horizon_pts_s);
    }
    while let Some(frame) = self.assembler.pop_frame() {
        self.queue.push_back(frame);
    }
    let split = self
        .queue
        .iter()
        .position(|(pts_s, _)| pts_s + FRAME_DURATION_S > until_pts_s + 1e-9)
        .unwrap_or(self.queue.len());
    Ok(self.queue.drain(..split).collect())
}
```

- [ ] **Step 6: Run focused neutral and Windows audio tests**

Run:

```powershell
cargo test -p clipline-capture pcm::tests
cargo test -p clipline-capture windows::wasapi::tests::audio_poll_horizon
```

If the second filter matches no tests, run the two fully qualified horizon tests separately. Expected: all selected tests pass.

- [ ] **Step 7: Run the unchanged real shared-clock hardware test**

Run:

```powershell
cargo test -p clipline-capture windows::wgc::tests::real_engines_on_one_clock_produce_a_synced_timeline -- --nocapture
```

Expected on the development machine: PASS with a sync report; segment audio skew and total drift remain within the existing 45 ms tolerances even when the desktop output endpoint is idle. If WGC, the hardware encoder, or the endpoint is unavailable, retain the test's existing self-skip behavior and report that verification limitation.

- [ ] **Step 8: Run crate-level regression tests and Clippy**

Run:

```powershell
cargo test -p clipline-capture
cargo clippy -p clipline-capture --all-targets -- -D warnings
```

Expected: both commands exit successfully with no failed tests or warnings.

- [ ] **Step 9: Commit Task 2**

```powershell
git add crates/clipline-capture/src/windows/wasapi.rs
git commit -m "fix(capture): synthesize quiet WASAPI frames"
```

- [ ] **Step 10: Run workspace quality gates**

Run:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: the complete workspace test suite passes and Clippy emits no warnings.
