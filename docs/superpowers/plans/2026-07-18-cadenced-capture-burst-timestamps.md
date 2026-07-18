# Cadenced Capture Burst Timestamp Fix Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Prevent a timeout-generated duplicate frame from being followed by a stale real WGC frame only 0.1 ms later, preserving Clipline's configured capture cadence and producing browser-friendly MP4 sample timing.

**Architecture:** Keep `CadencedCapture` as the sole cadence owner. When a real frame arrives before the next scheduled presentation time, retain its newer texture as the duplication source but continue waiting only for the remainder of the current cadence interval. Emit either the first real frame at or after the scheduled time or one timeout duplicate at that scheduled time. Leave encoder, MP4, audio, marker, and player behavior unchanged.

**Tech Stack:** Rust, WGC capture abstraction, existing neutral `TimedFrameSource`/`CaptureEngine` test doubles, existing Hybrid MP4 pipeline.

## Constraints

- Preserve the configured FPS during both changing and unchanged capture.
- Preserve the newest available frame data when an early/stale frame is suppressed.
- Do not busy-loop or restart a full timeout after every early frame.
- Do not alter encoder selection, MP4 keyframe policy, audio timing, marker mapping, or player seeking.
- Follow strict TDD and observe the regression fail for the intended 0.1 ms burst before implementation.

### Task 1: Reproduce the timeout/real-frame collision

**Files:**
- Modify: `apps/clipline-app/src/service.rs`

- [ ] Add a scripted `TimedFrameSource` test double that records requested timeout durations.
- [ ] Add a regression where a timeout emits the scheduled duplicate, a stale real frame arrives immediately afterward, and a later real frame reaches the next cadence boundary.
- [ ] Assert that the stale real frame is not emitted at `last_pts + 0.0001`, the later frame is emitted at the next cadence point, and the second wait uses only the remaining interval.
- [ ] Run the focused test and verify RED against the current `CadencedCapture` behavior.

### Task 2: Suppress early real-frame bursts

**Files:**
- Modify: `apps/clipline-app/src/service.rs`

- [ ] Make `CadencedCapture::next_frame` loop across early real frames until its existing `next_pts_s` deadline.
- [ ] Retain early frame data without advancing `last_emit_pts_s` or the cadence deadline.
- [ ] Pass only the remaining duration to the next `next_frame_timeout` call.
- [ ] Preserve current timeout duplication and on-time real-frame behavior.
- [ ] Run the focused cadence tests and verify GREEN.

### Task 3: Quality gates and acceptance

**Files:**
- Modify: `handoff.md`

- [ ] Stop any running `clipline-app.exe` before rebuilding.
- [ ] Run the focused app tests.
- [ ] Run `cargo test --workspace`.
- [ ] Run a fresh-cache clippy check for the changed app crate, then `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `cargo fmt --check` and `git diff --check`.
- [ ] Update `handoff.md` with the artifact evidence, fix, gates, and remaining WebView reproduction uncertainty.
- [ ] Relaunch Clipline with the staged FFmpeg runtime and leave it open for the user.
