# Discord Audio Safety-Track Default Fix Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax for tracking and remain unticked by repository convention.

**Goal:** Keep late-starting Discord and other app audio audible after the 0.1.34 review-player default-selection change.

**Architecture:** Preserve the existing experimental per-process tracks and the always-recorded mixed Output Audio safety track. When split tracks exist, default review/export selection to the mixed output plus non-output tracks such as the microphone; users can still opt into individual process tracks. Dynamic process discovery remains a separate, larger recorder enhancement.

**Tech Stack:** Vanilla JavaScript player model tested through `boa_engine`, existing Rust UI contract tests, native Windows WASAPI acceptance.

### Task 1: Lock the safe split-track default

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs`

- [ ] Change the split-output default-selection expectation to require `Output Audio` and `Microphone`, excluding startup-only process tracks.
- [ ] Run the focused player-core test and verify RED against the current process-track default.

### Task 2: Prefer the mixed safety track

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`

- [ ] Make split-output defaults select non-process tracks instead of excluding the mixed output.
- [ ] Preserve explicit per-process selection and output-master toggle behavior.
- [ ] Run the focused player-core tests and verify GREEN.

### Task 3: Verify and hand off

**Files:**
- Modify: `handoff.md`

- [ ] Record the native reproduction, measured stream levels, regression commit, and workaround/fix.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run `cargo clean -p clipline-app` followed by `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `git diff --check` and keep the fix scoped to default audio selection.
- [ ] Relaunch Clipline, restore Experimental app audio tracks to its original disabled setting, and leave recording active.
