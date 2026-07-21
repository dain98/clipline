# PR #100 Review Remediation

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox
> (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Resolve every current actionable review thread on PR #100 without regressing the
games-only recorder lifecycle or the privilege-invariant RAM meter.

**Review clusters:** Two Greptile threads report that a stopped event from an obsolete recorder can
overwrite the armed Waiting UI. Two Codex threads cover startup status replay and an in-flight
game-restart generation race. One Codex thread requires a fallback for Windows versions that do not
support `PROCESS_MEMORY_COUNTERS_EX2`.

## Task 1: Reject stale service statuses and replay Waiting after frontend startup

**Files:**
- Modify: `apps/clipline-app/src/app.rs`

- [ ] Add failing tests proving a stale service status cannot overwrite a newer Waiting generation.
- [ ] Add a failing test proving the current Waiting state can be queried when the frontend becomes
      ready.
- [ ] Gate service status forwarding and sender clearing through one generation-aware state method.
- [ ] Have `frontend_ready` emit the current Waiting status after the frontend listener is attached.
- [ ] Run the focused app tests and verify GREEN.

## Task 2: Invalidate in-flight detector restarts on every committed Waiting transition

**Files:**
- Modify: `apps/clipline-app/src/app.rs`

- [ ] Add a failing test that prepares a game-detection restart, commits settings into Waiting while
      no sender is installed, and rejects the stale replacement.
- [ ] Advance the recorder generation for every committed Waiting transition, including when
      `old_tx` is already absent.
- [ ] Run the focused recorder lifecycle tests and verify GREEN.

## Task 3: Preserve RAM sampling on older supported Windows builds

**Files:**
- Modify: `apps/clipline-app/src/memory.rs`

- [ ] Add failing tests proving an unavailable EX2 query selects the legacy resident-private walker
      and that a successful EX2 query does not invoke the fallback.
- [ ] Try `PROCESS_MEMORY_COUNTERS_EX2` first with limited query rights.
- [ ] When EX2 is unavailable, reopen child processes with `PROCESS_VM_READ` and use the previous
      `VirtualQueryEx` plus `K32QueryWorkingSetEx` resident-private calculation.
- [ ] Preserve child-skip behavior when neither query is available.
- [ ] Run the focused memory tests and verify GREEN.

## Task 4: Validate and hand off

- [ ] Run `cargo fmt --check` and `git diff --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clean -p clipline-app` and
      `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Update `handoff.md` with the review remediation and verification results.
- [ ] Commit and push the implementation so PR #100 updates.
- [ ] Rebuild and relaunch Clipline for manual testing.

## Task 5: Guard the manual-start Waiting notification

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a failing contract regression requiring `start_recording` to re-check the durable Waiting
      state after releasing the runtime lock.
- [ ] Emit the manual-start Waiting status only when `current_waiting_status` still reports Waiting.
- [ ] Run the focused contract test, workspace tests, and fresh-cache warning-denied Clippy.
- [ ] Update `handoff.md`, commit, push, rebuild, and relaunch Clipline.
