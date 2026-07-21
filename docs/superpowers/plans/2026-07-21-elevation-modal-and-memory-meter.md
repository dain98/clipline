# Elevation Modal and Memory Meter Follow-up Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox
> (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Require an explicit choice in the elevated-game warning and keep Clipline's RAM meter
accurate and stable across normal and administrator launches.

**Root cause:** The elevation dialog explicitly closes on backdrop clicks and only blocks Escape
while a restart is in flight. The memory sampler opens WebView2 children with `PROCESS_VM_READ` and
silently skips children that reject that right; elevation makes those same reads succeed, so the
rail appears to jump even when the process tree did not. Windows' extended process counters expose
`PrivateWorkingSetSize` with `PROCESS_QUERY_LIMITED_INFORMATION`, including for the sandboxed
WebView2 renderer in a normal launch.

## Task 1: Require an explicit elevation-dialog choice

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a failing UI contract proving backdrop clicks cannot close the elevation dialog and its
      `cancel` event always prevents Escape dismissal.
- [ ] Run the focused contract test and verify RED.
- [ ] Remove backdrop dismissal and always cancel the dialog's native Escape close.
- [ ] Run the focused contract test and verify GREEN.

## Task 2: Make process-tree RAM sampling privilege-invariant

**Files:**
- Modify: `apps/clipline-app/src/memory.rs`

- [ ] Add failing tests for the limited-rights query policy and a live current-process private
      working-set sample.
- [ ] Run the focused memory tests and verify RED.
- [ ] Replace `VirtualQueryEx` / `K32QueryWorkingSetEx` and `PROCESS_VM_READ` with
      `K32GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS_EX2)` opened through
      `PROCESS_QUERY_LIMITED_INFORMATION`.
- [ ] Preserve process-tree enumeration, cache behavior, saturating totals, exited-child tolerance,
      and `conhost.exe` exclusion.
- [ ] Run the focused memory tests and verify GREEN.

## Task 3: Documentation, validation, and PR update

**Files:**
- Modify: `ddoc.md`
- Modify: `handoff.md`

- [ ] Document the explicit modal choice and privilege-invariant private-working-set metric.
- [ ] Run `cargo fmt --check` and focused app tests.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clean -p clipline-app` and
      `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Rebuild with the staged FFmpeg runtime, relaunch Clipline, and verify normal-launch RAM now
      includes the WebView2 tree without requiring elevation.
- [ ] Commit, push the follow-up to the existing feature branch, and confirm PR #100 updates.
