# Active Encoder Status Acceptance Fix Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Surface the recorder's real active encoder in the native Clipline UI so Automatic selection can be verified as `Software · H.264` on software-only Windows VMs.

**Architecture:** Preserve the existing Rust encoder selection and status event. Retain the most recent `status.encoder` value in the frontend and include it in the recording status control's user-visible tooltip/accessibility description; clear it when recording stops. No encoder ranking, capture, FFmpeg, or settings behavior changes.

**Tech Stack:** Vanilla HTML/CSS/JavaScript frontend, existing Rust UI contract tests, Tauri status events.

### Task 1: Lock the active-encoder UI contract

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a failing structural test requiring the status listener to retain `s.encoder` and the rail status renderer to include it while recording.
- [ ] Run the focused UI contract test and verify RED for the missing frontend wiring.

### Task 2: Surface the active encoder

**Files:**
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/main.js`

- [ ] Track the active encoder label from backend status events.
- [ ] Render `Stop recording · <active encoder>` in the recording status control while active, with the existing fallback when the label is unavailable.
- [ ] Run the focused UI contract test and verify GREEN.

### Task 3: Verify and hand off

**Files:**
- Modify: `handoff.md`

- [ ] Update the acceptance checkpoint with the native replay and active-encoder UI findings.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Review `git diff --check` and keep the change scoped to status presentation.
- [ ] Rebuild and relaunch Clipline with the staged FFmpeg, then verify Computer Use sees `Software · H.264` in the active recorder status.

