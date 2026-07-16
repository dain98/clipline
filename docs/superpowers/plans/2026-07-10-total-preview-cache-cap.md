# Total Audio Preview Cache Cap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce the 2 GiB limit against all cached audio-preview MP4 bytes while never evicting a protected active preview.

**Architecture:** Keep the existing protected-path-aware LRU policy in `library.rs`, but track total preview bytes separately from reusable unprotected bytes. Protected files consume capacity without becoming eviction candidates; when protected bytes alone exceed the limit, every unprotected candidate is removed and the protected excess remains.

**Tech Stack:** Rust standard-library filesystem APIs, existing `TestDir` fixtures, Cargo tests and Clippy.

## Global Constraints

- `AUDIO_PREVIEW_CACHE_MAX_BYTES` remains exactly `2 * 1024 * 1024 * 1024`.
- Every matching `audio-preview-*.mp4` contributes to the eviction limit.
- Protected paths are never eviction candidates.
- If protected previews alone exceed the limit, remove all unprotected candidates and retain the protected excess.
- `AudioPreviewPruneReport.reusable_bytes` continues to report only surviving unprotected preview bytes.
- Preserve partial-file cleanup, modified-time-then-path LRU ordering, canonical protected-path matching, best-effort deletion, and filesystem error context.
- Modify only `apps/clipline-app/src/library.rs` for the implementation task.

---

### Task 1: Count protected previews toward the physical cache cap

**Files:**
- Modify: `apps/clipline-app/src/library.rs:1200-1260,2450-2510`

**Interfaces:**
- Consumes: `fn prune_audio_preview_cache(dir: &Path, protected: &[PathBuf], max_bytes: u64) -> Result<AudioPreviewPruneReport, String>`.
- Produces: the same function and report shape with total-byte eviction semantics; no caller changes.

- [ ] **Step 1: Change the existing LRU test into a RED total-cap regression**

Keep its fixture sizes and assertions, but pass a 26-byte cap so the 20-byte protected file leaves room for only the 6-byte newest reusable preview:

```rust
let report = prune_audio_preview_cache(
    dir.path(),
    std::slice::from_ref(&protected),
    26,
).unwrap();

assert!(!oldest.exists());
assert!(newest.exists());
assert!(protected.exists());
assert!(!partial.exists());
assert_eq!(report.reusable_bytes, 6);
```

The current implementation sees only 12 reusable bytes, considers that below 26, and incorrectly leaves `oldest` in place.

- [ ] **Step 2: Add a RED oversized-protected regression**

```rust
#[test]
fn audio_preview_cache_keeps_oversized_protected_and_evicts_all_reusable() {
    let dir = TestDir::new("clipline-library", "audio-preview-cache-oversized-protected");
    let oldest = dir.path().join("audio-preview-0001.mp4");
    let newest = dir.path().join("audio-preview-0002.mp4");
    let protected = dir.path().join("audio-preview-0003.mp4");
    std::fs::write(&oldest, [0_u8; 6]).unwrap();
    std::fs::write(&newest, [0_u8; 6]).unwrap();
    std::fs::write(&protected, [0_u8; 20]).unwrap();
    std::fs::File::options().write(true).open(&oldest).unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
    std::fs::File::options().write(true).open(&newest).unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(2)).unwrap();

    let report = prune_audio_preview_cache(
        dir.path(),
        std::slice::from_ref(&protected),
        10,
    ).unwrap();

    assert!(!oldest.exists());
    assert!(!newest.exists());
    assert!(protected.exists());
    assert_eq!(report.reusable_bytes, 0);
}
```

The current implementation evicts only `oldest`, stops when reusable bytes reach 6, and incorrectly leaves `newest` beside the already-oversized protected file.

- [ ] **Step 3: Run both regressions and verify RED**

Run:

```powershell
cargo test -p clipline-app audio_preview_cache_prunes_lru_and_partials_but_preserves_protected_file -- --nocapture
cargo test -p clipline-app audio_preview_cache_keeps_oversized_protected_and_evicts_all_reusable -- --nocapture
```

Expected: the first fails because `oldest.exists()` is true; the second fails because `newest.exists()` is true.

- [ ] **Step 4: Implement separate total and reusable byte accounting**

In `prune_audio_preview_cache`, declare total bytes beside the report and candidates:

```rust
let mut report = AudioPreviewPruneReport::default();
let mut total_bytes = 0_u64;
let mut candidates = Vec::new();
```

After partial handling, inspect every matching MP4's metadata before deciding whether it is protected. Count its length in `total_bytes`; only unprotected files contribute to `reusable_bytes` and candidates:

```rust
if !is_audio_preview_mp4(&path) {
    continue;
}
let metadata = entry.metadata()
    .map_err(|error| format!("read audio preview metadata {path:?}: {error}"))?;
let len = metadata.len();
total_bytes = total_bytes.saturating_add(len);
if audio_preview_path_is_protected(&path, protected) {
    continue;
}
report.reusable_bytes = report.reusable_bytes.saturating_add(len);
candidates.push(CachedAudioPreview {
    path,
    len,
    modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
});
```

Drive eviction with `total_bytes`, and subtract a successfully removed candidate from both totals:

```rust
for candidate in candidates {
    if total_bytes <= max_bytes {
        break;
    }
    if std::fs::remove_file(&candidate.path).is_ok() {
        report.removed_files += 1;
        report.removed_bytes = report.removed_bytes.saturating_add(candidate.len);
        report.reusable_bytes = report.reusable_bytes.saturating_sub(candidate.len);
        total_bytes = total_bytes.saturating_sub(candidate.len);
    }
}
```

- [ ] **Step 5: Run focused GREEN checks**

Run:

```powershell
cargo test -p clipline-app audio_preview_cache -- --nocapture
cargo test -p clipline-app audio_preview_write -- --nocapture
```

Expected: all matching cache-policy and atomic-write tests pass.

- [ ] **Step 6: Run app quality gates**

Run:

```powershell
cargo test -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: all app tests pass and Clippy reports zero warnings.

- [ ] **Step 7: Commit the correction**

Run:

```powershell
git add apps/clipline-app/src/library.rs
git commit -m "fix(player): cap total audio preview bytes"
```

Report the RED failure assertions, GREEN commands and counts, final commit, and self-review in `.superpowers/sdd/total-preview-cache-cap-report.md`; keep the report ignored and unstaged.
