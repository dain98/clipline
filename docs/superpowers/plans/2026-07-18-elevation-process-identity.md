# Elevation Process Identity Hardening Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Prevent Windows PID reuse from redirecting Clipline's elevated restart handoff or suppressing the elevated-game warning for a later process.

**Architecture:** Represent a Windows process instance with its PID plus kernel process creation timestamp. Pass both values to the elevated replacement and verify them on the same owned process handle before waiting. Add the same stable instance identity to game-detection events and key the frontend warning cache by it. Keep Win32 calls inside the existing safe Windows wrapper and preserve all current UAC cancellation/retry behavior.

**Tech Stack:** Rust, Win32 `GetProcessTimes`, Tauri events, vanilla JavaScript, Rust unit and UI-contract tests.

### Task 1: Lock process-instance contracts with failing tests

**Files:**
- Modify: `apps/clipline-app/src/windows.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a failing argument round-trip test requiring both parent PID and creation timestamp.
- [ ] Add a failing identity-comparison test proving a recycled PID does not match the original process instance.
- [ ] Add failing event/UI coverage requiring the warning cache to use a process-instance identity rather than PID alone.
- [ ] Run focused tests and verify RED.

### Task 2: Verify the elevated parent before waiting

**Files:**
- Modify: `apps/clipline-app/src/windows.rs`

- [ ] Query the current process creation timestamp before launching the elevated replacement.
- [ ] Pass the PID and timestamp through the private handoff argument.
- [ ] Open the candidate parent once with query and synchronize access, compare its creation timestamp, and wait only when it matches.
- [ ] Treat a mismatched identity as the original parent already gone.
- [ ] Keep all unsafe calls inside the safe Windows wrapper and run focused tests GREEN.

### Task 3: Key elevated-game warnings by process instance

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add the process-instance identity to active game-detection events.
- [ ] Use that identity for warning membership, insertion, and UAC retry deletion.
- [ ] Preserve once-per-process dismissal, transient inactive handling, and UAC cancel/retry behavior.
- [ ] Run focused app and UI-contract tests GREEN.

### Task 4: Gates and PR review resolution

- [ ] Stop Clipline before rebuilding.
- [ ] Run `cargo fmt --check`, `cargo test --workspace`, fresh-cache clippy for the changed crate, workspace clippy with warnings denied, and `git diff --check`.
- [ ] Commit and push the scoped fixes.
- [ ] Reply to the three Greptile threads: document the `windows-sys 0.61.2` import false positive and the two process-identity fixes, then resolve them.
- [ ] Relaunch Clipline for the user.
