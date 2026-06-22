# Cloud Audio And Debug Autostart Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix cloud uploads so multi-track selections are audible as one mixed stream, and prevent debug/Cargo builds from registering themselves for Windows startup.

**Architecture:** Keep Windows 10 WGC border work out of this PR; it is tracked as GitHub issue #42. For cloud uploads, preserve the existing lightweight remux path for zero or one selected audio track, but route two-or-more selected tracks through an FFmpeg `amix` helper that emits one Opus audio stream with copied video. For autostart, centralize build-policy logic so debug builds always report/keep autostart disabled while release builds continue to use the Tauri autostart plugin.

**Tech Stack:** Rust, Tauri commands, `clipline-mp4`, FFmpeg subprocess mixing, existing app unit tests and UI contract tests.

---

### Task 1: Track Windows 10 Border Separately

**Files:**
- No local code files.

- [ ] **Step 1: Create the issue**

Run:

```powershell
$body = @'
## Problem

Windows 10 users can see a yellow border around the captured window/display while Clipline is recording. Clipline already calls `GraphicsCaptureSession::SetIsBorderRequired(false)` in the WGC path, but on normal Windows 10 client builds that request can be ignored or unavailable.

## Current behavior

- Window/display capture uses Windows Graphics Capture (WGC).
- `crates/clipline-capture/src/windows/wgc.rs` makes a best-effort `session.SetIsBorderRequired(false)` call.
- On Windows 10 machines that do not support/allow borderless WGC capture, the yellow privacy border still appears.

## Proposed direction

Add a non-WGC capture option for Windows 10 display capture, likely DXGI Desktop Duplication, so affected users can avoid the WGC privacy border when capturing a display/region.
'@
gh issue create --repo dain98/clipline --title "Add Windows 10 no-border display capture fallback" --label enhancement --body $body
```

Expected: GitHub returns the issue URL `https://github.com/dain98/clipline/issues/42`.

### Task 2: Cloud Upload Audio Mixing

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/library.rs`
- Test: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write the failing cloud upload test**

In `apps/clipline-app/src/cloud.rs`, replace the old all-tracks-selected upload test with:

```rust
#[test]
fn upload_audio_selection_mixes_multiple_selected_tracks_for_cloud_playback() {
    let source = two_audio_mp4();
    let markers = audio_markers();
    let selected = vec!["output".to_string(), "microphone".to_string()];

    let out = upload_bytes_for_audio_selection_with_mixer(
        Path::new("clip.mp4"),
        source,
        Some(&markers),
        Some(&selected),
        |source, indices| {
            assert_eq!(source, Path::new("clip.mp4"));
            assert_eq!(indices, &[0, 1]);
            Ok(b"mixed upload mp4".to_vec())
        },
    )
    .unwrap();

    assert_eq!(out, b"mixed upload mp4");
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```powershell
cargo test -p clipline-app cloud::tests::upload_audio_selection_mixes_multiple_selected_tracks_for_cloud_playback
```

Expected: FAIL because `upload_bytes_for_audio_selection_with_mixer` does not exist yet.

- [ ] **Step 3: Implement the upload mixer path**

Change `upload_clip_to_cloud` to pass `&target` into `upload_bytes_for_audio_selection`.

Add `upload_bytes_for_audio_selection_with_mixer` in `apps/clipline-app/src/cloud.rs`:

```rust
fn upload_bytes_for_audio_selection_with_mixer(
    source_path: &Path,
    source_bytes: Vec<u8>,
    markers: Option<&ClipMarkers>,
    selected_audio_track_ids: Option<&[String]>,
    mix_selected_audio_tracks: impl FnOnce(&Path, &[u32]) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let Some(selected_audio_track_ids) = selected_audio_track_ids else {
        return Ok(source_bytes);
    };
    let selected_ids: BTreeSet<&str> = selected_audio_track_ids.iter().map(String::as_str).collect();
    if selected_ids.len() != selected_audio_track_ids.len() {
        return Err("audio track selection contains duplicates".into());
    }
    let tracks = markers.map(|m| m.audio_tracks.as_slice()).unwrap_or(&[]);
    if tracks.is_empty() {
        if selected_ids.is_empty() {
            return clipline_mp4::remux_with_selected_audio_tracks(&source_bytes, &[])
                .map_err(|e| e.to_string());
        }
        return Err("this clip has no selectable audio track metadata".into());
    }
    let available: BTreeSet<&str> = tracks.iter().map(|track| track.id.as_str()).collect();
    if let Some(unknown) = selected_ids.iter().find(|selected| !available.contains(**selected)) {
        return Err(format!("unknown audio track {unknown:?}"));
    }
    let selected_indices: Vec<u32> = tracks
        .iter()
        .filter(|track| selected_ids.contains(track.id.as_str()))
        .map(|track| track.track_index)
        .collect();
    if selected_indices.len() > 1 {
        return mix_selected_audio_tracks(source_path, &selected_indices);
    }
    clipline_mp4::remux_with_selected_audio_tracks(&source_bytes, &selected_indices)
        .map_err(|e| e.to_string())
}
```

Add `mix_upload_audio_tracks_with_ffmpeg(source_path, selected_indices)` that creates a unique temp MP4 under `std::env::temp_dir()`, calls a shared library FFmpeg mixer helper, reads the output bytes, and removes the temp file.

In `apps/clipline-app/src/library.rs`, rename the private preview-specific mixer helper to a shared crate-visible helper:

```rust
pub(crate) fn mix_audio_tracks_with_ffmpeg(
    source: &Path,
    output: &Path,
    selected_audio_track_indices: &[u32],
) -> Result<(), String>
```

Keep the same command line: `-map 0:v:0 -map [aout] -c:v copy -c:a libopus -b:a 160k -f mp4`.

- [ ] **Step 4: Run the focused cloud audio tests**

Run:

```powershell
cargo test -p clipline-app cloud::tests::upload_audio_selection_mixes_multiple_selected_tracks_for_cloud_playback cloud::tests::upload_audio_selection_remuxes_only_selected_track cloud::tests::upload_audio_selection_rejects_unknown_track_id
```

Expected: PASS.

### Task 3: Debug Autostart Guard

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Test: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write the failing autostart policy tests**

Add tests near the existing app tests:

```rust
#[test]
fn debug_build_autostart_policy_refuses_startup_enable() {
    assert!(!autostart_enabled_for_build_request(true, true));
    assert!(!autostart_enabled_for_build_request(false, true));
}

#[test]
fn release_build_autostart_policy_honors_user_choice() {
    assert!(autostart_enabled_for_build_request(true, false));
    assert!(!autostart_enabled_for_build_request(false, false));
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```powershell
cargo test -p clipline-app app::tests::debug_build_autostart_policy_refuses_startup_enable app::tests::release_build_autostart_policy_honors_user_choice
```

Expected: FAIL because `autostart_enabled_for_build_request` does not exist yet.

- [ ] **Step 3: Implement the build policy and use it**

Add:

```rust
fn autostart_enabled_for_build_request(requested: bool, debug_build: bool) -> bool {
    requested && !debug_build
}

fn autostart_enabled_for_current_build(requested: bool) -> bool {
    autostart_enabled_for_build_request(requested, cfg!(debug_assertions))
}
```

Update `get_autostart_status`, `set_autostart`, `save_settings`, and setup autostart sync so debug builds disable stale autostart entries and return/save `false` when startup is requested from a Cargo/debug build.

- [ ] **Step 4: Run focused app tests**

Run:

```powershell
cargo test -p clipline-app app::tests::debug_build_autostart_policy_refuses_startup_enable app::tests::release_build_autostart_policy_honors_user_choice app::tests::window_request_actions_follow_general_settings
```

Expected: PASS.

### Task 4: Verification, Commit, And PR Update

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Update handoff**

Add a concise 2026-06-22 note that cloud multi-track uploads are mixed into one cloud-playable audio stream and debug builds no longer register themselves for startup.

- [ ] **Step 2: Run full local gates**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process -Force
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clean -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: all commands exit 0.

- [ ] **Step 3: Commit and push**

Run:

```powershell
git add docs/superpowers/plans/2026-06-22-cloud-audio-and-debug-autostart.md apps/clipline-app/src/cloud.rs apps/clipline-app/src/library.rs apps/clipline-app/src/app.rs handoff.md
git commit -m "fix(app): mix cloud upload audio and guard debug autostart"
git push
```

Expected: PR #41 updates with the new commit.

- [ ] **Step 4: Watch CI and relaunch app**

Run:

```powershell
gh run list --branch codex/bug-scan-app-reliability-final --limit 3 --json databaseId,status,conclusion,headSha,name,createdAt,url
$run = gh run list --branch codex/bug-scan-app-reliability-final --limit 1 --json databaseId | ConvertFrom-Json | Select-Object -First 1
gh run watch $run.databaseId --interval 10 --exit-status
Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-Command','Set-Location ''C:\Users\dain\Projects\clipline''; cargo run -p clipline-app' -WindowStyle Hidden
```

Expected: CI passes on Windows and Ubuntu, and the local app process starts for manual testing.
