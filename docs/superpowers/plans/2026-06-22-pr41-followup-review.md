# PR 41 Follow-Up Review Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the six follow-up PR review findings in cloud clip sync, upload audio mixing, debug autostart behavior, and review playback.

**Architecture:** Keep the existing cloud/status and vanilla JS structure. Add small pure decision helpers where the review exposed race or policy bugs, then wire those helpers into the current async commands and UI update paths.

**Tech Stack:** Rust/Tauri, `clipline_cloud_api`, ffmpeg as a subprocess, vanilla JavaScript, Rust source-contract tests for UI wiring.

---

### Task 1: Guard Cloud 404 Reconciliation

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write the failing test**

Add this test near the existing missing-remote tests in `apps/clipline-app/src/cloud.rs`:

```rust
#[test]
fn missing_remote_clip_requires_confirmation_before_removing_finalized_record() {
    let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_public", 10);

    assert_eq!(missing_remote_sync_action(&record), MissingRemoteSyncAction::ConfirmMissing);

    mark_remote_not_found_once(&mut record);

    assert_eq!(missing_remote_sync_action(&record), MissingRemoteSyncAction::Remove);
}
```

Run:

```powershell
cargo test -p clipline-app cloud::tests::missing_remote_clip_requires_confirmation_before_removing_finalized_record
```

Expected: FAIL because `missing_remote_sync_action` and `mark_remote_not_found_once` do not exist.

- [ ] **Step 2: Implement the confirmation policy**

Add a marker constant, action enum, and helpers in `apps/clipline-app/src/cloud.rs`:

```rust
const REMOTE_NOT_FOUND_SYNC_MARKER: &str = "remote clip not found during status sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingRemoteSyncAction {
    Keep,
    ConfirmMissing,
    Remove,
}

fn missing_remote_sync_action(record: &CloudUploadRecord) -> MissingRemoteSyncAction {
    if record.upload_status == "uploaded_processing"
        || !record.upload_status.starts_with("uploaded_")
    {
        return MissingRemoteSyncAction::Keep;
    }
    if record.error.as_deref() == Some(REMOTE_NOT_FOUND_SYNC_MARKER) {
        MissingRemoteSyncAction::Remove
    } else {
        MissingRemoteSyncAction::ConfirmMissing
    }
}

fn mark_remote_not_found_once(record: &mut CloudUploadRecord) {
    record.error = Some(REMOTE_NOT_FOUND_SYNC_MARKER.to_string());
    record.updated_at_unix = unix_now();
}
```

Change the `sync_cloud_clip_status` 404 branch to:

```rust
Err(error) if cloud_error_is_not_found(&error) => match missing_remote_sync_action(&record) {
    MissingRemoteSyncAction::Remove => {
        state.update_cloud(|cloud| {
            remove_upload_record(cloud, &record);
        })?;
        Ok(CloudClipStatusSyncResult {
            path: request.path,
            record: None,
            removed: true,
        })
    }
    MissingRemoteSyncAction::ConfirmMissing => {
        let mut updated = record;
        mark_remote_not_found_once(&mut updated);
        persist_record(&state, &updated)?;
        Ok(CloudClipStatusSyncResult {
            path: request.path,
            record: Some(updated),
            removed: false,
        })
    }
    MissingRemoteSyncAction::Keep => Ok(CloudClipStatusSyncResult {
        path: request.path,
        record: Some(record),
        removed: false,
    }),
},
```

Run the focused test again. Expected: PASS.

### Task 2: Stop Debug Builds From Mutating Shared Autostart

**Files:**
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write the failing tests**

Replace `debug_build_autostart_policy_refuses_startup_enable` with:

```rust
#[test]
fn debug_build_autostart_policy_skips_registry_mutation() {
    assert!(!autostart_should_mutate_for_build(true));
    assert!(autostart_should_mutate_for_build(false));
}

#[test]
fn debug_build_preserves_saved_autostart_preference() {
    assert!(saved_autostart_preference_for_build(false, true, true));
    assert!(!saved_autostart_preference_for_build(true, false, true));
    assert!(saved_autostart_preference_for_build(true, false, false));
    assert!(!saved_autostart_preference_for_build(false, true, false));
}
```

Run:

```powershell
cargo test -p clipline-app app::tests::debug_build_autostart_policy_skips_registry_mutation app::tests::debug_build_preserves_saved_autostart_preference
```

Expected: FAIL because the new helper names do not exist.

- [ ] **Step 2: Implement the debug-safe policy**

Replace the coercing autostart helper with:

```rust
fn autostart_should_mutate_for_current_build() -> bool {
    autostart_should_mutate_for_build(cfg!(debug_assertions))
}

fn autostart_should_mutate_for_build(debug_build: bool) -> bool {
    !debug_build
}

fn saved_autostart_preference_for_current_build(requested: bool, previous: bool) -> bool {
    saved_autostart_preference_for_build(requested, previous, cfg!(debug_assertions))
}

fn saved_autostart_preference_for_build(requested: bool, previous: bool, debug_build: bool) -> bool {
    if debug_build {
        previous
    } else {
        requested
    }
}
```

Update `get_autostart_status` to return `app.autolaunch().is_enabled()` without disabling anything.

Update `set_autostart` to call `enable`/`disable` only when `autostart_should_mutate_for_current_build()` is true; in debug builds it returns the requested state without touching the Run key.

Update `save_settings` so debug builds preserve `old.open_on_startup`:

```rust
let requested_open_on_startup = settings.open_on_startup;
settings.open_on_startup =
    saved_autostart_preference_for_current_build(requested_open_on_startup, old.open_on_startup);
if settings.open_on_startup != old.open_on_startup && autostart_should_mutate_for_current_build() {
    settings.open_on_startup = set_autostart(&app, settings.open_on_startup)
        .map_err(|e| format!("update Windows startup registration: {e}"))?;
}
```

Wrap the setup-time autostart sync in:

```rust
if autostart_should_mutate_for_current_build() {
    let autostart = app.autolaunch();
    let _ = if settings.open_on_startup {
        autostart.enable()
    } else {
        autostart.disable()
    };
}
```

Run the focused tests again. Expected: PASS.

### Task 3: Make Upload Mix Staging Deterministic and Lighter

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/library.rs`

- [ ] **Step 1: Write failing cloud tests**

Add these tests in `apps/clipline-app/src/cloud.rs`:

```rust
#[test]
fn upload_audio_selection_plan_mixes_without_source_bytes_for_multiple_tracks() {
    let markers = audio_markers();
    let selected = vec!["output".to_string(), "microphone".to_string()];

    assert_eq!(
        upload_audio_selection_plan(Some(&markers), Some(&selected)).unwrap(),
        UploadAudioSelectionPlan::Mix(vec![0, 1])
    );
}

#[test]
fn upload_mix_paths_are_unique_for_same_timestamp_and_process() {
    let a = upload_mix_path_for_parts(123, 7, 42);
    let b = upload_mix_path_for_parts(123, 8, 42);

    assert_ne!(a, b);
}
```

Run:

```powershell
cargo test -p clipline-app cloud::tests::upload_audio_selection_plan_mixes_without_source_bytes_for_multiple_tracks cloud::tests::upload_mix_paths_are_unique_for_same_timestamp_and_process
```

Expected: FAIL because `UploadAudioSelectionPlan`, `upload_audio_selection_plan`, and `upload_mix_path_for_parts` do not exist.

- [ ] **Step 2: Write failing library source-contract test**

Add this test near the library ffmpeg mix tests in `apps/clipline-app/src/library.rs`:

```rust
#[test]
fn audio_mix_ffmpeg_command_requests_deterministic_mp4_output() {
    let source = include_str!("library.rs");

    assert!(source.contains("\"-fflags\", \"+bitexact\""));
    assert!(source.contains("\"-map_metadata\", \"-1\""));
    assert!(source.contains("\"-flags\", \"+bitexact\""));
}
```

Run:

```powershell
cargo test -p clipline-app library::tests::audio_mix_ffmpeg_command_requests_deterministic_mp4_output
```

Expected: FAIL because the ffmpeg command does not contain those flags.

- [ ] **Step 3: Implement upload planning and unique paths**

In `apps/clipline-app/src/cloud.rs`, add:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum UploadAudioSelectionPlan {
    Original,
    Remux(Vec<u32>),
    Mix(Vec<u32>),
}
```

Add `static UPLOAD_MIX_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);` near the existing constants and import `std::sync::atomic::{AtomicU64, Ordering};`.

Replace the existing upload selection function with a planner that validates duplicate and unknown IDs, returns `Original` when no selection is supplied, returns `Mix(indices)` for two or more selected tracks, and returns `Remux(indices)` for zero or one selected track.

Change `upload_clip_to_cloud` to read metadata first, reject `meta.len() == 0`, read source bytes only for `Original` and `Remux`, and call `mix_upload_audio_tracks_with_ffmpeg` directly for `Mix`.

Change `unique_upload_mix_path` to call:

```rust
let counter = UPLOAD_MIX_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
upload_mix_path_for_parts(nanos, counter, std::process::id())
```

Add:

```rust
fn upload_mix_path_for_parts(nanos: u128, counter: u64, process_id: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "clipline-upload-audio-mix-{process_id}-{counter}-{nanos}.mp4"
    ))
}
```

Run the focused cloud tests. Expected: PASS.

- [ ] **Step 4: Add deterministic ffmpeg flags**

In `apps/clipline-app/src/library.rs`, change the ffmpeg command so it includes `-fflags +bitexact` before the input, strips metadata, and requests bitexact codec output:

```rust
.args(["-hide_banner", "-nostdin", "-y", "-fflags", "+bitexact", "-i"])
...
.args([
    "-map", "0:v:0", "-map", "[aout]", "-map_metadata", "-1", "-c:v", "copy", "-c:a",
    "libopus", "-b:a", "160k", "-fflags", "+bitexact", "-flags", "+bitexact", "-f", "mp4",
])
```

Run the focused library test. Expected: PASS.

### Task 4: Resume Source Playback if Preview Generation Fails

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing UI contract test**

Extend `audio_preview_generation_is_not_eager_on_clip_open` with:

```rust
assert!(
    js.contains("if (forceResume && currentClip && currentClip.path === clip.path) {")
        && js.contains("video.play().catch(() => syncPlayState());"),
    "preview generation failure while opening a clip must fall back to source playback"
);
```

Run:

```powershell
cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open
```

Expected: FAIL because the catch block only reports the preview error.

- [ ] **Step 2: Implement the playback fallback**

In `apps/clipline-app/ui/main.js`, update the `catch` block in `applySelectedAudioTracksToPlayback`:

```javascript
  } catch (e) {
    if (seq !== audioPreviewSeq) return;
    setDeckStatus("");
    $("error").textContent = String(e);
    if (forceResume && currentClip && currentClip.path === clip.path) {
      video.play().catch(() => syncPlayState());
    }
  }
```

Run the focused UI contract test. Expected: PASS.

### Task 5: Prevent Stale Open-Sync From Clobbering Fresh Uploads

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing UI contract test**

Add assertions to `audio_preview_generation_is_not_eager_on_clip_open`:

```rust
assert!(
    js.contains("function cloudUploadRecordForPath(path)")
        && js.contains("applyCloudClipSyncResult(result, { expectedLocalClipId, expectedUpdatedAtUnix })"),
    "cloud open-sync must capture the record identity it started from"
);
assert!(
    js.contains("current.local_clip_id !== expectedLocalClipId")
        && js.contains("Number(current.updated_at_unix || 0) > Number(expectedUpdatedAtUnix || 0)"),
    "cloud open-sync must ignore stale results once a newer upload record exists"
);
```

Run:

```powershell
cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open
```

Expected: FAIL because sync results currently apply without a freshness guard.

- [ ] **Step 2: Implement guarded cloud sync application**

In `apps/clipline-app/ui/main.js`, add:

```javascript
function cloudUploadRecordForPath(path) {
  const uploads = cloudSettings().uploads || {};
  return Object.values(uploads).find((record) => record && record.path === path) || null;
}
```

Change `clipCloudRecord` to call `cloudUploadRecordForPath(clip.path)`.

Change `applyCloudClipSyncResult` to accept `{ expectedLocalClipId = "", expectedUpdatedAtUnix = 0 } = {}`. Before applying `result.removed` or `result.record`, read the current record for `result.path`; if its `local_clip_id` differs from `expectedLocalClipId` or its `updated_at_unix` is newer than `expectedUpdatedAtUnix`, return `false`.

Change `syncCloudClipStatus` to capture:

```javascript
const expectedLocalClipId = record.local_clip_id || "";
const expectedUpdatedAtUnix = record.updated_at_unix || 0;
```

and call:

```javascript
applyCloudClipSyncResult(result, { expectedLocalClipId, expectedUpdatedAtUnix });
```

Run the focused UI contract test. Expected: PASS.

### Task 6: Final Verification and PR Update

**Files:**
- Modify: `handoff.md` if the implementation creates meaningful new sharp edges or follow-up notes.

- [ ] **Step 1: Format and inspect**

Run:

```powershell
cargo fmt
git diff --check
git diff -- apps/clipline-app/src/cloud.rs apps/clipline-app/src/app.rs apps/clipline-app/src/library.rs apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
```

Expected: formatting succeeds, whitespace check is clean, and the diff only contains PR follow-up fixes.

- [ ] **Step 2: Run full quality gates**

Stop any existing `clipline-app.exe`, then run:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clean -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: all tests pass and clippy reports no warnings.

- [ ] **Step 3: Commit and push**

Run:

```powershell
git add docs/superpowers/plans/2026-06-22-pr41-followup-review.md apps/clipline-app/src/cloud.rs apps/clipline-app/src/app.rs apps/clipline-app/src/library.rs apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "fix(cloud): harden upload sync follow-ups"
git push
```

Expected: commit and push succeed on `codex/bug-scan-app-reliability-final`.

- [ ] **Step 4: Launch for manual testing**

Run:

```powershell
cargo run -p clipline-app
```

Expected: the app opens for user verification.
