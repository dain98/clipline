# Shareable Audio Mix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make cloud uploads and clipboard sharing produce externally playable clips whose selected audio is audible as one mixed stream.

**Architecture:** Keep local capture and review playback multi-track. Add a native Opus mix/remux helper in `clipline-mp4` so the app can replace two-or-more selected audio tracks with one mixed Opus track while stream-copying video. Route cloud uploads and clipboard sharing through the same compatibility export decision so Discord/cloud players do not receive silent or first-track-only MP4s.

**Tech Stack:** Rust, `clipline-mp4`, `audiopus`, Tauri commands, vanilla JS, existing app unit tests and UI contract tests.

---

### Task 1: Native MP4 Audio Mix Helper

**Files:**
- Modify: `crates/clipline-mp4/Cargo.toml`
- Modify: `crates/clipline-mp4/src/trim.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: `crates/clipline-mp4/src/trim.rs`

- [ ] **Step 1: Write failing native mix tests**

Add tests in `crates/clipline-mp4/src/trim.rs` that build a finalized MP4 with one video track and two real Opus audio tracks, call `remux_with_mixed_audio_track(&input, &[0, 1])`, and assert:
- the output has one video track and one audio track;
- the output audio packet decodes with non-silent PCM;
- invalid selected audio indices are rejected.

Run:

```powershell
cargo test -p clipline-mp4 remux_with_mixed_audio_track
```

Expected: FAIL because `remux_with_mixed_audio_track` does not exist.

- [ ] **Step 2: Add native Opus decode/mix/re-encode implementation**

Add `audiopus = "0.2"` to `crates/clipline-mp4/Cargo.toml`.

In `trim.rs`, add:

```rust
pub fn remux_with_mixed_audio_track(
    input: &[u8],
    selected_audio_track_indices: &[u32],
) -> Result<Vec<u8>, TrimError>
```

Implementation rules:
- parse the finalized MP4 with existing `parse_movie`;
- validate duplicate and out-of-range selected audio indices like `remux_with_selected_audio_tracks`;
- keep all video tracks and their original samples;
- require selected audio tracks to be Opus stereo 48 kHz, matching Clipline-authored files;
- decode each selected audio sample with `audiopus::coder::Decoder::decode_float`;
- group decoded frames by sample start tick, sum selected frames, clamp samples to `[-1.0, 1.0]`;
- encode each mixed frame with `audiopus::coder::Encoder::encode_float`;
- mux video samples plus one new audio track with `HybridMp4Writer::new_multi`.

Export the function from `crates/clipline-mp4/src/lib.rs`.

- [ ] **Step 3: Verify native mix tests pass**

Run:

```powershell
cargo test -p clipline-mp4 remux_with_mixed_audio_track
```

Expected: PASS.

### Task 2: Cloud Upload Uses Mixed Compatibility Audio

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Test: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write failing upload decision tests**

Change the multi-track upload tests so multiple selected tracks expect a native mix plan, not a multi-track remux. Keep single selected track and muted upload on the lightweight remux path.

Run:

```powershell
cargo test -p clipline-app cloud::tests::upload_audio_selection_plan_mixes_multiple_selected_tracks
```

Expected: FAIL because the current plan returns `Remux(vec![0, 1])`.

- [ ] **Step 2: Implement upload mix plan**

Change `UploadAudioSelectionPlan` to:

```rust
enum UploadAudioSelectionPlan {
    Original,
    Remux(Vec<u32>),
    Mix(Vec<u32>),
}
```

Change `upload_audio_selection_plan` so:
- `None` selection returns `Original`;
- empty explicit selection returns `Remux(Vec::new())`;
- one selected track returns `Remux(vec![track_index])`;
- two-or-more selected tracks returns `Mix(vec![...])`.

Change `upload_bytes_for_audio_selection_from_path` and test helper handling so `Mix(indices)` calls:

```rust
clipline_mp4::remux_with_mixed_audio_track(&source_bytes, &indices)
```

- [ ] **Step 3: Verify upload tests pass**

Run:

```powershell
cargo test -p clipline-app cloud::tests::upload_audio_selection_plan_mixes_multiple_selected_tracks cloud::tests::upload_audio_selection_remuxes_only_selected_track cloud::tests::upload_audio_selection_rejects_unknown_track_id
```

Expected: PASS.

### Task 3: Clipboard Sharing Uses Compatibility Export

**Files:**
- Modify: `apps/clipline-app/src/settings/persistence.rs`
- Modify: `apps/clipline-app/src/settings/mod.rs`
- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Test: `apps/clipline-app/src/library.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing clipboard/share tests**

Add library tests for a helper that chooses the clipboard file:
- no selected audio metadata returns the source path;
- one selected track creates/reuses a share export containing that track;
- two-or-more selected tracks creates/reuses a share export with mixed audio;
- share export cache paths live under `%APPDATA%\Clipline\share-exports`.

Add a UI contract assertion that `copy_clip_to_clipboard` receives `audioTrackIds` from `selectedAudioTrackIdsForClip(currentClip)`.

Run:

```powershell
cargo test -p clipline-app library::tests::clipboard_share_export_mixes_multiple_selected_tracks
cargo test -p clipline-app --test ui_contract clipboard_copy_sends_selected_audio_tracks
```

Expected: FAIL because clipboard copy currently passes only the original path.

- [ ] **Step 2: Add share export cache and clipboard request**

Add `share_export_cache_dir()` beside `audio_preview_cache_dir()`.

Change `copy_clip_to_clipboard` to accept a request object:

```rust
#[derive(Debug, Deserialize)]
pub struct CopyClipToClipboardRequest {
    pub path: String,
    pub audio_track_ids: Option<Vec<String>>,
}
```

Before placing a file on the clipboard, resolve a share-compatible path:
- if no marker-sidecar audio metadata or no explicit selected ids, copy the original path;
- if selected ids are explicit, use the same index decision as upload;
- write generated MP4s under `share-exports` with a cache key based on source path, length, modified time, selected ids, and export mode;
- prune old share exports on write, similar to audio previews;
- keep generated files after setting the clipboard because the paste target reads the CF_HDROP path later.

- [ ] **Step 3: Wire selected audio from the UI**

Change `copyClipToClipboard()` in `apps/clipline-app/ui/main.js` to invoke:

```js
await invoke("copy_clip_to_clipboard", {
  request: {
    path: currentClip.path,
    audioTrackIds: clipAudioTracks(currentClip).length
      ? selectedAudioTrackIdsForClip(currentClip)
      : null,
  },
});
```

- [ ] **Step 4: Verify clipboard/share tests pass**

Run:

```powershell
cargo test -p clipline-app library::tests::clipboard_share_export_mixes_multiple_selected_tracks
cargo test -p clipline-app --test ui_contract clipboard_copy_sends_selected_audio_tracks
```

Expected: PASS.

### Task 4: Handoff And Verification

**Files:**
- Modify: `handoff.md`
- Optionally modify: `README.md`

- [ ] **Step 1: Update handoff**

Add a 2026-06-25 note explaining:
- the 0.1.12/0.1.14 remux-only behavior could produce silent or first-track-only external playback;
- upload and clipboard now generate a one-track mixed compatibility MP4 for multi-track selections;
- the fix is native Opus mixing, not FFmpeg, so users do not need a separate FFmpeg install.

- [ ] **Step 2: Run focused tests**

Run:

```powershell
cargo test -p clipline-mp4 remux_with_mixed_audio_track
cargo test -p clipline-app cloud::tests::upload_audio_selection_plan_mixes_multiple_selected_tracks library::tests::clipboard_share_export_mixes_multiple_selected_tracks
cargo test -p clipline-app --test ui_contract clipboard_copy_sends_selected_audio_tracks
```

Expected: PASS.

- [ ] **Step 3: Run workspace gates**

Run:

```powershell
cargo test --workspace
cargo clean -p clipline-mp4
cargo clean -p clipline-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Launch app for manual test**

Stop any existing `clipline-app.exe`, then run:

```powershell
cargo run -p clipline-app
```

Manual checks:
- Upload a clip with output plus mic selected; the cloud copy should have audible output and mic.
- Copy the same clip to clipboard and paste into Discord; the uploaded Discord media should include mic audio.
- Single-track clips still copy/upload without unnecessary mixing.

- [ ] **Step 5: Commit implementation**

Commit with:

```powershell
git add crates/clipline-mp4/Cargo.toml crates/clipline-mp4/src/trim.rs crates/clipline-mp4/src/lib.rs apps/clipline-app/src/cloud.rs apps/clipline-app/src/settings/persistence.rs apps/clipline-app/src/settings/mod.rs apps/clipline-app/src/library.rs apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs handoff.md
git commit -m "fix(app): mix shareable audio exports"
```
