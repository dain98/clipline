# Clipline Trim/Export Editor (Milestone 10) Implementation Plan

> **For agentic workers:** Follow the repo's plan-driven TDD convention. Execute task-by-task,
> write failing tests before implementation, and leave checkboxes unticked.

**Goal:** Add the first clip editor/export path: a library user can open a saved clip, choose
start/end times, and export a new MP4 without altering the original. **Exit criterion:** Clipline
can keyframe-align a requested range, stream-copy selected H.264/Opus samples into a fresh
finalized MP4, crop marker sidecars into the exported clip, refresh the library, and pass local
workspace tests, clippy, push, and CI.

**Architecture:** Keep the media work neutral. Add `crates/clipline-mp4/src/trim.rs` with a small
reader for finalized MP4s written by `HybridMp4Writer`: parse `moov/trak/mdia/minf/stbl`, recover
`TrackConfig` from `stsd`, expand `stts/stsc/stsz/stss/co64|stco` into per-sample metadata, copy
sample payloads from the source `mdat`, and re-mux selected samples with `HybridMp4Writer`.

This milestone intentionally implements **keyframe-aligned stream copy only**:

- start is aligned backward to the latest video sync sample at or before the requested start
- end is aligned forward to the next video sync sample at or after the requested end, or EOF
- no boundary GOP re-encode and no frame-accurate cuts yet
- only finalized MP4s with H.264 video and optional Opus audio are supported

The Tauri app exposes an `export_clip(path, start_s, end_s)` command in `library.rs`. It validates
that the source path is an MP4 inside `Videos\Clipline`, writes a unique sibling file named from
the source stem plus the aligned range, and writes a cropped `.markers.json` sidecar when the source
has markers inside the exported range. The UI keeps the existing player overlay and adds compact
numeric in/out controls plus an Export button.

---

### Task 1: MP4 trim metadata reader

**Files:** `crates/clipline-mp4/src/trim.rs`, `crates/clipline-mp4/src/lib.rs`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn parse_clipline_mp4_recovers_tracks_and_samples() { /* writer-made H264+Opus fixture */ }

#[test]
fn rejects_unfinalized_or_missing_sample_tables() { /* ftyp/free/moov/moof layout */ }
```

- [ ] **Step 2: implement**

```rust
pub struct TrimInfo {
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub aligned_start_s: f64,
    pub aligned_end_s: f64,
    pub duration_s: f64,
}

pub fn trim_keyframe_aligned(input: &[u8], start_s: f64, end_s: f64)
    -> Result<(Vec<u8>, TrimInfo), TrimError>;
```

Internal reader types can stay private. Keep parsing intentionally narrow and explicit: one sample
description per track, version-0 `mdhd`, `stts/stsc/stsz`, optional `stss`, and `co64` or `stco`.

### Task 2: keyframe-aligned stream-copy output

**Files:** `crates/clipline-mp4/src/trim.rs`, optional integration test under
`crates/clipline-mp4/tests/`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn trims_to_previous_and_next_keyframes() { /* request 0.4..1.2 -> 0.0..2.0 */ }

#[test]
fn copied_output_keeps_h264_and_opus_samples_playable() { /* ffprobe when available */ }
```

- [ ] **Step 2: implement** sample selection:
  - video: selected samples from aligned start through aligned end
  - audio: samples whose sample interval overlaps the aligned range, rebased to output zero
  - write one fragment carrying all selected per-track samples, then finalize
- [ ] Ensure empty/invalid ranges return useful errors rather than panics.

### Task 3: app export command and marker cropping

**Files:** `apps/clipline-app/src/library.rs`.

- [ ] Add `ExportedClipInfo { path, name, requested_start_s, requested_end_s, aligned_start_s,
aligned_end_s, duration_s }`.
- [ ] Add `export_clip(path: String, start_s: f64, end_s: f64) -> Result<ExportedClipInfo, String>`.
- [ ] Reuse the existing clips-directory path validation.
- [ ] Write output next to the source with a unique name.
- [ ] If the source has markers, crop `[aligned_start_s, aligned_end_s)` and rebase `t_s` to the
new exported clip's zero point.

### Task 4: editor UI

**Files:** `apps/clipline-app/ui/index.html`, `apps/clipline-app/src/app.rs`.

- [ ] Register the new `export_clip` command.
- [ ] Add compact in/out numeric fields and an Export button inside the existing player overlay.
- [ ] Opening a clip initializes in/out to `0` and clip duration.
- [ ] Timeline clicks continue to seek. Export validates in/out client-side enough to avoid obvious
bad requests, then relies on backend validation for authority.
- [ ] On successful export, close or keep the player open, show aligned range in the status message,
and refresh Library/Storage.

### Task 5: gates and handoff

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] Update `handoff.md` with milestone 10 and new sharp edges.
- [ ] Push and verify CI on Ubuntu + Windows.

---

## Out of scope

- Frame-accurate trim via boundary GOP re-encode.
- Montage/joining multiple clips.
- GIF/WebM export.
- Native decode/scrub path for AV1/HEVC.
- Arbitrary destination picker or shell save dialog.
