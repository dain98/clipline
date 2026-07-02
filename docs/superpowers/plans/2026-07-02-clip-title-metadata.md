# Clip Title Metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make normal clip renaming edit a fast display title while keeping filesystem rename as a secondary action.

**Architecture:** Add a small per-clip `.clipline.json` sidecar that stores `title` and stable `kind`. `list_clips` returns both fields, gallery/review/cloud use them for display and classification, and the existing inline title editor writes metadata instead of renaming the MP4. Keep the existing MP4 rename behavior behind a new `rename_clip_file` command and a secondary context-menu action.

**Tech Stack:** Rust Tauri commands in `apps/clipline-app/src/library.rs`, vanilla JS/CSS/HTML in `apps/clipline-app/ui`, and Rust-backed UI contract/player-core tests.

---

### Task 1: Backend Metadata Contract

**Files:**
- Modify: `apps/clipline-app/src/library.rs`
- Test: `apps/clipline-app/src/library.rs`

- [ ] **Step 1: Write failing Rust tests**

Add tests proving `rename_clip` writes a display title without moving the MP4, `list_clips_from_dir` returns `title` and stable `kind`, and `rename_clip_file` still moves the MP4 plus marker/poster sidecars.

- [ ] **Step 2: Verify the backend tests fail**

Run: `cargo test -p clipline-app library::tests::rename_clip_updates_title_metadata_without_moving_file library::tests::rename_clip_file_preserves_kind_and_moves_sidecars`

Expected: FAIL because `ClipInfo` has no `title` or `kind`, `rename_clip` still moves the MP4, and `rename_clip_file` is not exposed.

- [ ] **Step 3: Implement metadata helpers and commands**

Add `ClipMetadata { title: Option<String>, kind: Option<String> }`, `clip_metadata_path`, read/write helpers, `clip_kind_for_path`, `clip_title_for_path`, and `rename_clip_file`. Change `rename_clip` to validate title text, write `.clipline.json`, and return the same path plus title/kind.

- [ ] **Step 4: Verify backend tests pass**

Run: `cargo test -p clipline-app library::tests::rename_clip_updates_title_metadata_without_moving_file library::tests::rename_clip_file_preserves_kind_and_moves_sidecars`

Expected: PASS.

### Task 2: UI Display Title Flow

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/ui/library.js`
- Modify: `apps/clipline-app/ui/review-player.js`
- Test: `apps/clipline-app/tests/player_core.rs`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI tests**

Add player-core tests for `clipKind({ kind: "session", name: "Ranked win.mp4" })` and gallery preview preferring `clip.title` over the filename stem. Add a UI contract check that gallery filtering calls `clipKind(c)` and rename submit patches `title` without requiring a path change.

- [ ] **Step 2: Verify the UI tests fail**

Run: `cargo test -p clipline-app --test player_core clip_kind_prefers_backend_kind_for_renamed_clips gallery_card_preview_prefers_custom_title`

Expected: FAIL because `clipKind` only accepts a filename string and `galleryCardPreview` only reads `clip.name`.

- [ ] **Step 3: Wire JS through metadata**

Allow `clipKind` to accept either a clip object or string, preferring `clip.kind`. Update gallery cards, filters, search, upload default titles, review header display, and inline rename save handling to use `clip.title || filename stem`. Preserve current playback behavior and `clipsCache` patching.

- [ ] **Step 4: Verify UI tests pass**

Run: `cargo test -p clipline-app --test player_core clip_kind_prefers_backend_kind_for_renamed_clips gallery_card_preview_prefers_custom_title`

Expected: PASS.

### Task 3: Secondary File Rename Action

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/library.js`
- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Test: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing contract coverage**

Require a secondary `clip-menu-rename-file` context-menu item and JS call to `rename_clip_file`, while the header pencil continues to call display-title rename.

- [ ] **Step 2: Verify the contract test fails**

Run: `cargo test -p clipline-app --test ui_contract clip_context_menu_is_app_owned`

Expected: FAIL because the menu has no file-rename item and JS has no `rename_clip_file` call.

- [ ] **Step 3: Add secondary file rename UI**

Add a compact file rename dialog opened only from the context menu. It calls `rename_clip_file`, patches `path`, `name`, `title`, `kind`, cloud record paths, and poster cache keys, then refreshes the current review source only if the renamed clip is open.

- [ ] **Step 4: Verify the contract test passes**

Run: `cargo test -p clipline-app --test ui_contract clip_context_menu_is_app_owned`

Expected: PASS.

### Task 4: Final Verification

**Files:**
- Modify: `handoff.md`

- [ ] **Step 1: Update handoff**

Add a short note that normal rename is now metadata-backed and file rename is secondary.

- [ ] **Step 2: Run targeted and workspace verification**

Run:

```powershell
cargo test -p clipline-app
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all PASS with no clippy warnings.

- [ ] **Step 3: Launch the app**

If `clipline-app.exe` is running, stop it, then run `cargo run -p clipline-app`.

Expected: app opens for manual testing.
