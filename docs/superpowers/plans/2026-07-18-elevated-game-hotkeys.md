# Elevated Game Hotkey Recovery Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Detect when a game is elevated above Clipline and offer an explicit, safe restart-as-administrator action so Save Replay hotkeys work while that game is focused.

**Architecture:** Keep Clipline `asInvoker` by default. Query Windows process-token elevation behind a safe Windows wrapper and add the blocked state to the existing game-detection event. The frontend shows one warning per elevated game process. On user acceptance, launch the same executable through the Windows `runas` verb with a parent-process wait argument; only exit the current instance after Windows successfully creates the elevated child, preventing UAC cancellation from closing Clipline and preventing overlap with the single-instance lock.

**Tech Stack:** Rust, Win32 process tokens and `ShellExecuteW`, Tauri commands/events, vanilla HTML/CSS/JavaScript, Rust unit and structural UI-contract tests, native Windows Computer Use acceptance.

### Task 1: Lock the elevation decision and UI contract

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add failing Rust coverage proving the warning is required only when a detected game is elevated and Clipline is not.
- [ ] Add failing structural UI coverage requiring an elevation dialog, one-warning-per-process behavior, and a restart command.
- [ ] Run the focused tests and verify RED.

### Task 2: Add safe Windows elevation primitives

**Files:**
- Create: `apps/clipline-app/src/windows.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Modify: `apps/clipline-app/Cargo.toml`

- [ ] Add safe wrappers for current/process token elevation checks.
- [ ] Add a `runas` launcher that passes the current PID and reports UAC cancellation without terminating the current app.
- [ ] Before Tauri/single-instance startup, make the elevated child wait for the parent PID to exit.
- [ ] Keep all new unsafe Win32 calls inside the Windows wrapper.
- [ ] Run focused Rust tests and verify GREEN.

### Task 3: Wire detection and the opt-in restart UI

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`

- [ ] Add the blocked-hotkey flag to game-detection events.
- [ ] Show a modal at most once per detected game PID, explaining the UAC boundary and rolling-buffer reset.
- [ ] Invoke `restart_as_administrator` only from the affirmative action; dismissal leaves Clipline unchanged.
- [ ] Register the backend command and surface launch errors in the existing app error area.
- [ ] Run focused Rust and UI tests and verify GREEN.

### Task 4: Quality gates and native acceptance

**Files:**
- Modify: `handoff.md`

- [ ] Stop the existing Clipline process before rebuilding.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run a fresh-cache `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `git diff --check` and review the scoped diff.
- [ ] Launch Clipline normally, use an elevated Windows test process to verify the warning, cancel UAC once and confirm Clipline remains alive, then accept UAC and confirm the replacement Clipline process is elevated.
- [ ] Confirm Save Replay works while the elevated test window is focused and leave Clipline open.
- [ ] Update `handoff.md` with concrete findings and commit the implementation.
