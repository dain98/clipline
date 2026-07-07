# Focus-Follow Capture Target Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Change focus-follow no-game-focused behavior from muted privacy slate to configured Capture target capture with normal audio.

**Architecture:** Add a Capture target fallback switch target and capture kind. Route focused-game detection misses to the configured Capture target, open `ServiceOptions.capture_source` through the normal screen-capture path, keep the audio privacy gate disabled for Capture target fallback, and reserve slate/mute for hard failures.

**Tech Stack:** Rust Tauri app, Windows Graphics Capture / Desktop Duplication capture primitives, vanilla HTML/CSS/JS UI, Rust UI contract tests.

## Global Constraints

- Plan-driven TDD: write failing tests before production code changes.
- Keep Windows-only capture code behind existing Windows modules and safe service wrappers.
- Do not add dependencies.
- Preserve full-session continuity across focus changes.
- Preserve marker gating so markers only attach to the active focused game.
- Preserve the user's configured Capture target, including SET REGION and selected backend.

---

### Task 1: Route No-Focus To Capture Target Fallback

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Produces: `SwitchCaptureTarget::CaptureTarget`
- Produces: `FocusFollowTargetKey::CaptureTarget`

- [ ] **Step 1: Write failing app test**

Add a test next to the existing focus-follow update tests:

```rust
#[test]
fn focus_follow_no_focused_game_sends_capture_target_fallback() {
    let (tx, rx) = mpsc::channel();
    let mut settings = AppSettings::default();
    settings.games.follow_focused_windows = true;
    let mut inner = runtime_inner_with_sender(tx, settings);

    let emit = RuntimeState::prepare_focus_follow_update(&mut inner, None).unwrap();

    assert!(emit);
    assert!(inner.active_game.is_none());
    assert!(matches!(
        rx.try_recv(),
        Ok(Cmd::SwitchCapture(service::SwitchCaptureTarget::CaptureTarget))
    ));
}
```

- [ ] **Step 2: Run failing app test**

Run: `cargo test -p clipline-app focus_follow_no_focused_game_sends_capture_target_fallback`

Expected: FAIL because `SwitchCaptureTarget::CaptureTarget` does not exist or is not routed yet.

- [ ] **Step 3: Implement minimal app/service target**

Add `CaptureTarget` to `SwitchCaptureTarget`, add `CaptureTarget` to `FocusFollowTargetKey`, and make `focus_follow_key(None)` / `focus_follow_command(None)` use Capture target fallback.

- [ ] **Step 4: Run app test**

Run: `cargo test -p clipline-app focus_follow_no_focused_game_sends_capture_target_fallback`

Expected: PASS.

### Task 2: Capture Configured Target With Audio On Fallback

**Files:**
- Modify: `apps/clipline-app/src/service.rs`
- Test: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Consumes: `SwitchCaptureTarget::CaptureTarget`
- Produces: `CaptureKind::CaptureTarget`
- Consumes: `ServiceOptions.capture_source`

- [ ] **Step 1: Write failing service tests**

Add tests proving Capture target fallback is a distinct state, keeps audio unmuted, and uses the configured source:

```rust
#[test]
fn focus_run_state_capture_target_fallback_keeps_audio_public() {
    let mut state = FocusRunState::from_options(&ServiceOptions::default());
    state.apply_target(&SwitchCaptureTarget::CaptureTarget);

    assert_eq!(state.capture_kind, CaptureKind::CaptureTarget);
    assert_eq!(state.active_game, None);
    assert_eq!(state.active_game_plugin_id, None);
    assert_eq!(state.recording_mode, RecordingMode::ReplaysOnly);
    assert_eq!(state.slate_reason, None);
    assert!(!state.should_mute_audio());
}

#[test]
fn configured_capture_target_fallback_uses_saved_display_region() {
    let region = CaptureRegion {
        display_id: Some("DISPLAY2".into()),
        x: 10,
        y: 20,
        width: 1280,
        height: 720,
    };
    let opts = ServiceOptions {
        focus_follow_enabled: true,
        capture_source: CaptureSource::DisplayRegion(region.clone()),
        active_game: None,
        ..ServiceOptions::default()
    };

    assert_eq!(
        configured_capture_target_source(&opts),
        &CaptureSource::DisplayRegion(region)
    );
}
```

- [ ] **Step 2: Run failing service tests**

Run: `cargo test -p clipline-app capture_target`

Expected: FAIL because `CaptureKind::CaptureTarget`, `SwitchCaptureTarget::CaptureTarget`, or `configured_capture_target_source` is missing.

- [ ] **Step 3: Implement configured Capture target fallback**

Add `CaptureKind::CaptureTarget`, `SwitchTargetIdentity::CaptureTarget`, and service handling that opens `ServiceOptions.capture_source` with `open_screen_capture` for Capture target fallback. Change audio privacy writes to call `focus_state.should_mute_audio()` so only slate mutes audio.

- [ ] **Step 4: Run focused service tests**

Run: `cargo test -p clipline-app capture_target`

Expected: PASS.

### Task 3: Update User-Facing Copy And Metadata

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `handoff.md`

**Interfaces:**
- Consumes: `capture_kind: "capture_target"`
- Produces: truthful Settings > Games status copy.

- [ ] **Step 1: Write failing UI contract expectations**

Update the existing focus-follow contract to require:

```rust
"Capture target active. Focus a saved game to switch back."
"Recording the focused saved game; other windows use your Capture target."
```

- [ ] **Step 2: Run failing UI contract**

Run: `cargo test -p clipline-app --test ui_contract focus_follow_settings_contract`

Expected: FAIL while UI still mentions privacy slate or desktop fallback.

- [ ] **Step 3: Update UI copy and handoff**

Change Settings > Games helper text and active status copy from privacy slate/desktop wording to Capture target wording. Update `handoff.md` to describe Capture target fallback with continuing audio.

- [ ] **Step 4: Run UI contract**

Run: `cargo test -p clipline-app --test ui_contract focus_follow_settings_contract`

Expected: PASS.

### Task 4: Verify And Ship PR Update

**Files:**
- All changed files.

**Interfaces:**
- Produces: pushed PR branch with green local and remote verification.

- [ ] **Step 1: Run focused tests**

Run:

```powershell
cargo test -p clipline-app capture_target
cargo test -p clipline-app focus_follow
cargo test -p clipline-app --test ui_contract focus_follow_settings_contract
```

Expected: PASS.

- [ ] **Step 2: Run full verification**

Run:

```powershell
cargo test --workspace
cargo clean -p clipline-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Commit and push**

Run:

```powershell
git add apps/clipline-app/src/app.rs apps/clipline-app/src/service.rs apps/clipline-app/ui/index.html apps/clipline-app/ui/settings.js apps/clipline-app/tests/ui_contract.rs handoff.md docs/superpowers/plans/2026-07-07-focus-follow-desktop-fallback-design.md docs/superpowers/plans/2026-07-07-focus-follow-desktop-fallback.md
git commit -m "fix(capture): use configured target for focus gaps"
git push
```

Expected: branch pushed to PR #82.

- [ ] **Step 4: Relaunch app**

Stop any existing `clipline-app.exe`, rebuild if needed, and start `target/debug/clipline-app.exe` from this worktree.
