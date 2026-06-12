# Clipline Disk Quota + Auto-GC (Milestone 8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or
> superpowers:subagent-driven-development to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Videos\Clipline` gets a storage manager slice from ddoc §10: a configurable quota,
oldest-first automatic garbage collection, and visible quota usage in the app library. **Exit
criterion:** after a clip is saved, older clips are deleted until the directory fits under the
quota, marker sidecars are deleted with their clips, the just-saved clip is protected from
immediate deletion, and the UI shows used space / quota / clip count.

**Architecture:** Add a neutral `clipline-storage` crate for filesystem-only clip inventory and
quota enforcement. The app layer owns the platform-specific clips directory and Tauri serialization.
`clipline-storage` scans managed clip pairs (`*.mp4` plus optional `*.markers.json`), computes
usage, and enforces a byte quota by sorting by MP4 modified time ascending. `enforce_quota`
accepts an optional protected path so save-time GC never removes the clip the user just created;
if the protected/newest clip alone exceeds the quota, the report says the directory is still over
budget instead of deleting the fresh save. `apps/clipline-app` calls GC after successful saves and
exposes `storage_status` for the UI. Configurability is CLI-first until the Settings milestone:
`--disk-quota-gb <n>` with `0` disabling automatic GC.

**Tech Stack:** no non-std dependencies for `clipline-storage`; app reuses existing `serde`.

---

### Task 1: neutral storage inventory and quota report

**Files:** `Cargo.toml`, `crates/clipline-storage/Cargo.toml`,
`crates/clipline-storage/src/lib.rs`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn status_counts_mp4_and_marker_sidecars() { /* temp dir with two clips */ }

#[test]
fn inventory_ignores_non_mp4_files() { /* notes.txt does not count */ }
```

- [ ] **Step 2: implement**

```rust
pub struct StorageStatus {
    pub clip_count: usize,
    pub total_bytes: u64,
    pub quota_bytes: Option<u64>,
}

pub fn storage_status(dir: &Path, quota_bytes: Option<u64>) -> io::Result<StorageStatus>;
```

- [ ] Run `cargo test -p clipline-storage`.

### Task 2: oldest-first GC with sidecar deletion

**Files:** `crates/clipline-storage/src/lib.rs`.

- [ ] **Step 1: failing tests**

```rust
#[test]
fn enforce_quota_deletes_oldest_until_under_budget() { /* three clips */ }

#[test]
fn enforce_quota_deletes_marker_sidecar_with_clip() { /* mp4 + markers.json */ }

#[test]
fn enforce_quota_protects_the_fresh_clip_even_if_still_over_budget() { /* protected newest */ }
```

- [ ] **Step 2: implement**

```rust
pub struct GcReport {
    pub deleted_clips: usize,
    pub freed_bytes: u64,
    pub status: StorageStatus,
}

pub fn enforce_quota(
    dir: &Path,
    quota_bytes: Option<u64>,
    protect: Option<&Path>,
) -> io::Result<GcReport>;
```

- [ ] Run `cargo test -p clipline-storage`.

### Task 3: app integration and CLI-configured cap

**Files:** `apps/clipline-app/Cargo.toml`, `apps/clipline-app/src/service.rs`,
`apps/clipline-app/src/library.rs`, `apps/clipline-app/src/app.rs`.

- [ ] Add `clipline-storage` as a Windows-only app dependency.
- [ ] Add `ServiceOptions::disk_quota_bytes: Option<u64>` defaulting to 10 GiB.
- [ ] Parse `--disk-quota-gb <n>`; `0` means `None`.
- [ ] After a successful save and sidecar write, run `clipline_storage::enforce_quota` with the
saved path protected.
- [ ] Extend `Event::Saved` with deleted/freed/storage fields for the UI.
- [ ] Add `#[tauri::command] storage_status()` returning used MB, quota MB, clip count, and over
quota state.
- [ ] Add app-side tests for CLI quota parsing if parsing is factored into a pure helper.

### Task 4: UI storage surface

**Files:** `apps/clipline-app/ui/index.html`.

- [ ] Add a compact storage row below the buffer row showing used/limit/clip count.
- [ ] Refresh storage status on load, after save, and after delete.
- [ ] Show a short cleanup result when save-time GC deleted old clips.
- [ ] Keep the visual style consistent with the current dense tray app; no settings UI yet.

### Task 5: gates and handoff

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] Optional live check: `cargo run -p clipline-app -- --disk-quota-gb 0`
- [ ] Update `handoff.md` with milestone 8 status and the next frontier.

---

## Out of scope

- Full persisted settings UI.
- Per-game folders.
- Disk-spill replay buffer.
- User-initiated "clean now" controls.
