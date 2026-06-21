# Bug Scan App Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the first bug-scan slice for app-side reliability: recorder restart safety, settings persistence, cloud upload terminal states, replay-cache validation, and split-output review playback.

**Architecture:** Keep the fixes surgical and aligned with existing modules. Backend fixes stay in `app.rs`, `settings.rs`, and `cloud.rs` with unit tests. Frontend behavior is guarded through the existing Rust UI contract tests plus `player-core` helpers; normal clip open remains lazy, split-output default playback gets a targeted preview handoff.

**Tech Stack:** Rust workspace tests, Tauri command modules, vanilla JS UI, `boa_engine` player-core tests, `cargo test`, `cargo clippy`.

---

### Task 1: Preserve Recorder Sender on Restart Option Errors

**Files:**
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write failing unit tests**

Add tests showing that `restart` and `set_detected_game` do not clear `inner.tx` when service options fail after an active recorder exists. Use invalid disk replay settings to make `RuntimeState::options` fail.

Run: `cargo test -p clipline-app app::tests::recording_sender_survives_restart_option_error app::tests::recording_sender_survives_game_restart_option_error`

Expected before fix: tests fail because `inner.tx` is `None`.

- [ ] **Step 2: Implement minimal fix**

Build `next_options` before `inner.tx.take()` in `restart` and both restart branches of `set_detected_game`. Only clear `last_save_request` and send `Cmd::Stop` after the options are available.

- [ ] **Step 3: Verify targeted tests**

Run: `cargo test -p clipline-app app::tests::recording_sender_survives_restart_option_error app::tests::recording_sender_survives_game_restart_option_error`

Expected after fix: both tests pass.

### Task 2: Make Settings Saves Atomic and Avoid Holding Runtime Lock During Disk I/O

**Files:**
- Modify: `apps/clipline-app/src/settings.rs`
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write failing settings test**

Add a unit test that writes settings over an existing file and asserts no `settings.json.tmp` remains after save. This documents the temp-file contract.

Run: `cargo test -p clipline-app settings::tests::save_to_replaces_settings_via_temp_file`

Expected before fix: fails because `save_to` writes directly and never exercises the temp-file path.

- [ ] **Step 2: Implement atomic save**

Change `AppSettings::save_to` to write `settings.json.tmp` in the same directory, flush and `sync_all`, then rename it over the destination. Remove the temp on error where possible.

- [ ] **Step 3: Reduce runtime lock hold**

Change `RuntimeState::update_cloud` so it mutates and clones settings under the mutex, releases the mutex, saves to disk, then reacquires the mutex to publish the saved settings if no newer cloud state has appeared. Preserve upload-record merge behavior.

- [ ] **Step 4: Verify targeted tests**

Run: `cargo test -p clipline-app settings::tests::save_to_replaces_settings_via_temp_file cloud::tests::upload_record_supersedes_older_record_for_same_path`

Expected after fix: tests pass.

### Task 3: Fix Windows Replay Cache Overlap Validation

**Files:**
- Modify: `apps/clipline-app/src/settings.rs`

- [ ] **Step 1: Write failing validation test**

Add a Windows-only test for media/cache overlap where only path casing differs.

Run: `cargo test -p clipline-app settings::tests::disk_replay_rejects_case_variant_media_folder_overlap`

Expected before fix on Windows: test fails because `same_or_nested_path` misses the overlap.

- [ ] **Step 2: Implement path comparison fix**

Teach `same_or_nested_path` to compare normalized path components case-insensitively on Windows, without changing non-Windows behavior.

- [ ] **Step 3: Verify targeted test**

Run: `cargo test -p clipline-app settings::tests::disk_replay_rejects_case_variant_media_folder_overlap`

Expected after fix: test passes.

### Task 4: Make Cloud Processing Timeout Retryable and Clean Posters

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`

- [ ] **Step 1: Write failing cloud tests**

Add pure unit tests for a helper that maps ready-poll timeout to a non-busy record state and for cloud local cleanup removing `poster.jpg` alongside the clip and markers.

Run: `cargo test -p clipline-app cloud::tests::ready_timeout_marks_upload_failed_with_retryable_error cloud::tests::delete_uploaded_local_files_removes_poster_sidecar`

Expected before fix: helper/tests fail because behavior does not exist.

- [ ] **Step 2: Implement cloud helpers**

Add `mark_ready_timeout` to set `upload_status = "failed"` and a clear error while preserving `remote_clip_id`/`remote_url`. Add `delete_uploaded_local_files` and use it from `delete_local_after_upload`.

- [ ] **Step 3: Wire timeout path**

When `wait_for_ready_clip` returns `Ok(None)`, persist and emit the failed record before returning.

- [ ] **Step 4: Verify targeted tests**

Run: `cargo test -p clipline-app cloud::tests::ready_timeout_marks_upload_failed_with_retryable_error cloud::tests::delete_uploaded_local_files_removes_poster_sidecar`

Expected after fix: tests pass.

### Task 5: Align Split-Output Open Playback With Default Selection

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI contract tests**

Update the existing no-eager-preview test so it still forbids unconditional preview generation, but requires `openClip` to call a split-output helper when default selected IDs differ from source tracks.

Run: `cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open`

Expected before fix: fails because the helper call does not exist.

- [ ] **Step 2: Add player-core predicate test**

Add a `PlayerCore.selectionNeedsPreview(tracks, selectedIds)` test: all tracks returns false; split-output default without mixed output returns true.

Run: `cargo test -p clipline-app --test player_core split_output_default_selection_requires_preview`

Expected before fix: fails because the helper does not exist.

- [ ] **Step 3: Implement minimal frontend fix**

Add `selectionNeedsPreview` in `player-core.js`. In `main.js`, add a helper called from `openClip` that invokes `applySelectedAudioTracksToPlayback()` only when the current effective selection needs a preview. Keep normal all-track clip opens lazy.

- [ ] **Step 4: Verify UI tests**

Run: `cargo test -p clipline-app --test player_core split_output_default_selection_requires_preview && cargo test -p clipline-app --test ui_contract audio_preview_generation_is_not_eager_on_clip_open`

Expected after fix: both tests pass.

### Task 6: OpenClip Teardown

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing UI contract test**

Extend the `openClip` contract to require `cancelAnimationFrame(rafId)` and `pendingSeek = null` near the top of `openClip`.

Run: `cargo test -p clipline-app --test ui_contract open_clip_clears_previous_playback_loop_and_pending_seek`

Expected before fix: fails because `openClip` does not clear either value.

- [ ] **Step 2: Implement teardown**

At the start of `openClip`, cancel `rafId` and clear `pendingSeek` before assigning the new clip.

- [ ] **Step 3: Verify targeted test**

Run: `cargo test -p clipline-app --test ui_contract open_clip_clears_previous_playback_loop_and_pending_seek`

Expected after fix: test passes.

### Task 7: Final Verification and PR

**Files:**
- Modify: `handoff.md` if the final change set is significant enough to affect current state notes.

- [ ] **Step 1: Run workspace tests**

Run: `cargo test --workspace`

Expected: all tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: no warnings or errors.

- [ ] **Step 3: Commit and publish**

Stage only files touched for this first PR. Commit with `fix(app): harden bug-scan reliability issues`, push `codex/bug-scan-app-reliability`, and open a draft PR into `main`.
