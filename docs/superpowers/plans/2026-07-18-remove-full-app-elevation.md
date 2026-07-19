# Remove Full-Application Elevation Plan

**Goal:** Clipline must not invite users to elevate a per-user, user-writable application and then execute user-controlled subprocess candidates as administrator.

**Architecture:** Remove the in-app `runas` restart and its parent/child handoff protocol entirely. Elevated-game detection remains read-only and continues to explain why focused hotkeys are unavailable, but the only guidance is to run the game without administrator privileges; Clipline does not offer or perform elevation. This closes H-01 at the trust boundary instead of attempting a partial FFmpeg path filter that would leave the main executable and bundled resources user-writable. Removing the restart protocol also makes L-23 inapplicable because no privileged relaunch can lose command-line overrides.

## Task 1: No-elevation contract TDD

- [ ] Replace UI contracts that require a UAC restart with contracts requiring explanatory guidance and no restart action.
- [ ] Add structural assertions that the command allowlist, app source, Windows wrapper, and startup path contain no full-app elevation or handoff entry point.
- [ ] Confirm the new contracts fail while `restart_as_administrator`, `runas`, and the handoff argument remain.

## Task 2: Remove the privileged relaunch path

- [ ] Remove the Tauri restart command and command registration.
- [ ] Remove `ShellExecuteW("runas")`, the elevation handoff argument/parser/waiter, and their obsolete Windows tests/imports.
- [ ] Start Tauri directly for every ordinary launch.

## Task 3: Preserve safe elevated-game guidance

- [ ] Keep process-elevation and process-instance detection so the warning remains accurate and once-per-process.
- [ ] Replace the restart button with a single dismiss action and guidance to run the game without administrator privileges.
- [ ] Remove UAC in-flight state and reconciliation logic from the frontend.

## Task 4: Verify and document

- [ ] Run focused app/UI tests.
- [ ] Run `cargo test --workspace` and workspace clippy with warnings denied.
- [ ] Update `handoff.md` and the master audit ledger for H-01 and L-23.
- [ ] Rebuild/open the native app and add an elevated-game warning check to the final manual acceptance list.
