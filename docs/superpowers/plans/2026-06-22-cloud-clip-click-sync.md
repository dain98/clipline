# Cloud Clip Click Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refresh a clip's saved cloud upload record from Clipline Cloud when the user opens that clip, so deleted remote clips and visibility changes are reflected in the UI.

**Architecture:** Add one focused Tauri command in `apps/clipline-app/src/cloud.rs` that looks up the local cloud record by path, checks the remote clip with `CloudClient::get_clip`, updates or removes the persisted record, and returns a small sync result. Wire `apps/clipline-app/ui/main.js` to call this command in the background from `openClip`, then update the local `currentSettings.cloud.uploads` cache and rerender the affected controls.

**Tech Stack:** Rust/Tauri commands, `clipline-cloud-api::CloudClient`, persisted `CloudSettings`, vanilla JS UI, Rust static UI contract tests.

---

### Task 1: Backend Cloud Record Sync Helpers

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Test: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write failing tests for remote detail application and remote deletion policy**

Add tests in `mod tests`:

```rust
#[test]
fn cloud_clip_detail_updates_record_visibility_status_and_url() {
    let cloud = CloudSettings {
        public_url: Some("https://clips.example.com".into()),
        ..CloudSettings::default()
    };
    let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_public", 10);
    record.remote_clip_id = Some("remote-1".into());
    record.remote_url = Some("https://clips.example.com/old".into());

    apply_remote_clip_to_record(&cloud, &mut record, &clip_detail("remote-1", "unlisted", "ready", Some("https://share.example.com/c/1")));

    assert_eq!(record.visibility, "unlisted");
    assert_eq!(record.upload_status, "uploaded_public");
    assert_eq!(record.remote_url.as_deref(), Some("https://share.example.com/c/1"));
    assert!(record.error.is_none());
}

#[test]
fn missing_remote_clip_removes_finalized_records_but_keeps_processing_records() {
    assert!(missing_remote_should_remove_record(&upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_public", 10)));
    assert!(!missing_remote_should_remove_record(&upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_processing", 10)));
    assert!(!missing_remote_should_remove_record(&upload_record("local", "D:\\Videos\\clip.mp4", "processing", 10)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
cargo test -p clipline-app cloud::tests::cloud_clip_detail_updates_record_visibility_status_and_url
cargo test -p clipline-app cloud::tests::missing_remote_clip_removes_finalized_records_but_keeps_processing_records
```

Expected: both tests fail because helpers do not exist.

- [ ] **Step 3: Implement minimal helper functions**

Add helpers:

```rust
fn apply_remote_clip_to_record(
    cloud: &CloudSettings,
    record: &mut CloudUploadRecord,
    clip: &ClipDetailResponse,
) {
    record.visibility = clip.visibility.clone();
    record.remote_clip_id = Some(clip.id.clone());
    record.remote_url = clip.public_url.clone().or_else(|| cloud_clip_url(cloud, &clip.id));
    record.upload_status = upload_status_for_remote_clip(clip);
    record.error = None;
    record.updated_at_unix = unix_now();
}

fn upload_status_for_remote_clip(clip: &ClipDetailResponse) -> String {
    if clip.status != "ready" {
        "uploaded_processing".to_string()
    } else if clip.visibility == "private" {
        "uploaded_private".to_string()
    } else {
        "uploaded_public".to_string()
    }
}

fn missing_remote_should_remove_record(record: &CloudUploadRecord) -> bool {
    record.upload_status.starts_with("uploaded_") && record.upload_status != "uploaded_processing"
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```powershell
cargo test -p clipline-app cloud::tests::cloud_clip_detail_updates_record_visibility_status_and_url
cargo test -p clipline-app cloud::tests::missing_remote_clip_removes_finalized_records_but_keeps_processing_records
```

Expected: both tests pass.

### Task 2: Tauri Command

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing command wiring test**

Extend the cloud wiring assertion in `apps/clipline-app/tests/ui_contract.rs`:

```rust
&& app_rs().contains("crate::cloud::sync_cloud_clip_status")
&& main_js().contains("sync_cloud_clip_status")
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls`

Expected: FAIL because neither command wiring nor JS invocation exists.

- [ ] **Step 3: Implement command and register it**

Add request/result types and command in `cloud.rs`:

```rust
#[derive(Debug, Deserialize)]
pub struct SyncCloudClipStatusRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct CloudClipStatusSyncResult {
    pub path: String,
    pub record: Option<CloudUploadRecord>,
    pub removed: bool,
}
```

Register `crate::cloud::sync_cloud_clip_status` in `tauri::generate_handler!`.

- [ ] **Step 4: Run command wiring test**

Run: `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls`

Expected: PASS after JS wiring exists in Task 3.

### Task 3: Frontend Background Sync On Open

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI contract for click-time sync**

Add assertions to `audio_preview_generation_is_not_eager_on_clip_open` or the cloud wiring test:

```rust
assert!(
    open_clip.contains("syncCloudClipStatus(clip);"),
    "opening a clip should refresh its cloud record in the background"
);
assert!(
    js.contains("function applyCloudClipSyncResult(result)")
        && js.contains("removeCloudUploadRecordForPath(result.path)")
        && js.contains("upsertCloudUploadRecord(result.record)"),
    "cloud sync results must update or remove the local cloud record cache"
);
```

- [ ] **Step 2: Run UI contract test to verify it fails**

Run: `cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open`

Expected: FAIL because `syncCloudClipStatus` and result handling do not exist.

- [ ] **Step 3: Implement UI cache update and background command call**

Add:

```js
function removeCloudUploadRecordForPath(path) {
  const cloud = cloudSettings();
  const uploads = cloud.uploads || {};
  const nextUploads = {};
  let changed = false;
  for (const [key, record] of Object.entries(uploads)) {
    if (record && record.path === path) {
      changed = true;
      continue;
    }
    nextUploads[key] = record;
  }
  if (!changed) return false;
  cloud.uploads = nextUploads;
  if (currentSettings) currentSettings.cloud = cloud;
  return true;
}

function applyCloudClipSyncResult(result) {
  if (!result) return false;
  let changed = false;
  if (result.removed) changed = removeCloudUploadRecordForPath(result.path);
  if (result.record) {
    upsertCloudUploadRecord(result.record);
    changed = true;
  }
  if (changed) {
    renderClips();
    syncUploadClipButton();
  }
  return changed;
}

async function syncCloudClipStatus(clip) {
  const record = clipCloudRecord(clip);
  if (!clip || !record || !record.remote_clip_id || !cloudConnected()) return;
  try {
    const result = await invoke("sync_cloud_clip_status", { request: { path: clip.path } });
    applyCloudClipSyncResult(result);
  } catch (_) {
    // Keep the last known cloud state if the status check is unavailable.
  }
}
```

Call `syncCloudClipStatus(clip);` from `openClip` after initial render/playback setup.

- [ ] **Step 4: Run UI contract test**

Run: `cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open`

Expected: PASS.

### Task 4: Full Verification And PR Update

**Files:**
- Modify: `docs/superpowers/plans/2026-06-22-cloud-clip-click-sync.md` remains with unchecked boxes by project convention.

- [ ] **Step 1: Stop the running app before rebuilding**

Run: `Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process -Force`

Expected: no running `clipline-app.exe`.

- [ ] **Step 2: Run workspace tests**

Run: `cargo test --workspace`

Expected: all tests pass.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: no warnings or errors.

- [ ] **Step 4: Commit implementation**

Run:

```powershell
git add docs/superpowers/plans/2026-06-22-cloud-clip-click-sync.md apps/clipline-app/src/cloud.rs apps/clipline-app/src/app.rs apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(cloud): sync clip status on open"
```

- [ ] **Step 5: Push and watch CI**

Run:

```powershell
git push
gh run list --branch codex/bug-scan-app-reliability-final --limit 1 --json databaseId,status,conclusion,headSha
gh run watch <run-id> --interval 10 --exit-status
```

Expected: CI passes on Ubuntu and Windows.
