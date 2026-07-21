# Recorder Control and Hook Readiness Plan

**Goal:** A user Stop must remain authoritative across automatic game-detection restarts, and hotkey configuration must never report success before the low-level keyboard hook is actually installed.

**Architecture:** Runtime state gains an explicit `recording_desired` flag alongside the existing monotonically increasing recording generation. A game-triggered restart prepares options and reserves a generation under the mutex, spawns outside it, then installs only if both desired state and generation still match. Stop clears desired state and advances the generation even when no sender is currently installed; Start and newer restarts supersede older work. Stale spawned services receive an immediate non-announcing Stop.

The keyboard hook follows the mouse hook's readiness protocol. Its thread creates a message queue, installs `WH_KEYBOARD_LL`, reports either the real thread id or an installation error, and waits for installer acknowledgement before entering the message loop. The global hook state is published only after readiness. Stored thread ids provide explicit teardown for partially completed installation, and hotkey updates fail when no hook is installed.

## Task 1: Recorder restart TDD

- [ ] Add a deterministic test that prepares a game restart, issues Stop while no sender is installed, and proves the spawned replacement is rejected and stopped.
- [ ] Add a test that Start during the restart gap supersedes the pending replacement without losing the newer sender.
- [ ] Add a test that a newer game restart while the first is spawning supersedes the old plan and installs the newest replacement.
- [ ] Confirm the tests fail against the current unconditional installation path.

## Task 2: Generation-aware desired recording state

- [ ] Add `recording_desired` to runtime state and keep it synchronized across start, stop, service termination, and sender installation.
- [ ] Make game restart preparation advance the generation and return a commit token, including when it supersedes an already-pending restart.
- [ ] Commit replacements only while desired state and generation still match; immediately stop rejected services.
- [ ] Preserve the existing rule that option-building failure cannot drop a currently installed sender.
- [ ] Run focused app state tests.

## Task 3: Keyboard hook readiness TDD and implementation

- [ ] Add pure readiness-channel tests for successful installation, hook failure, disconnect, and timeout/error propagation.
- [ ] Add an acknowledged keyboard-hook startup protocol and unhook on cancellation or message-loop exit.
- [ ] Publish `SAVE_HOOK` only after the keyboard hook reports success; keep failed installation retryable.
- [ ] Store the keyboard thread id in hook state and stop partial hooks if mouse setup or singleton publication fails.
- [ ] Make `set_save_hotkeys` return an error when the singleton is unavailable.
- [ ] Run focused hotkey tests.

## Task 4: Verify and document

- [ ] Run `cargo test --workspace`.
- [ ] Clean changed app artifacts and run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Update `handoff.md` with the desired-state/generation invariant and hook readiness contract.
- [ ] Stop any running `clipline-app.exe`, launch `cargo run -p clipline-app`, and verify the native UI/hotkey initialization remains healthy.

