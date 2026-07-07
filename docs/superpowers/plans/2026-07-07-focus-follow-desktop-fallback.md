# Focus-Follow Desktop Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Change focus-follow no-game-focused behavior from muted privacy slate to desktop capture with normal audio.

**Architecture:** Add a desktop fallback switch target and capture kind. Route focused-game detection misses to desktop fallback, open the primary monitor in the switch controller, keep the audio privacy gate disabled for desktop fallback, and reserve slate/mute for hard failures.

**Tech Stack:** Rust Tauri app, Windows Graphics Capture / Desktop Duplication capture primitives, vanilla HTML/CSS/JS UI, Rust UI contract tests.

## Global Constraints

- Plan-driven TDD: write failing tests before production code changes.
- Keep Windows-only capture code behind existing Windows modules and safe service wrappers.
- Do not add dependencies.
- Preserve full-session continuity across focus changes.
- Preserve marker gating so markers only attach to the active focused game.

---

### Task 1: Route No-Focus To Desktop Fallback

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Produces: `SwitchCaptureTarget::Desktop`
- Produces: `FocusFollowTargetKey::Desktop`

- [ ] **Step 1: Write failing app test**

Add a test next to the existing focus-follow update tests:

```rust
#[test]
fn focus_follow_no_focused_game_sends_desktop_fallback() {
    let (tx, rx) = mpsc::channel();
    let mut settings = AppSettings::default();
    settings.games.follow_focused_windows = true;
    let mut inner = runtime_inner_with_sender(tx, settings);

    let emit = RuntimeState::prepare_focus_follow_update(&mut inner, None).unwrap();

    assert!(emit);
    assert!(inner.active_game.is_none());
    assert!(matches!(
        rx.try_recv(),
        Ok(Cmd::SwitchCapture(service::SwitchCaptureTarget::Desktop))
    ));
}
```

- [ ] **Step 2: Run failing app test**

Run: `cargo test -p clipline-app focus_follow_no_focused_game_sends_desktop_fallback`

Expected: FAIL because `SwitchCaptureTarget::Desktop` does not exist.

- [ ] **Step 3: Implement minimal app/service target**

Add `Desktop` to `SwitchCaptureTarget`, add `Desktop` to `FocusFollowTargetKey`, and make `focus_follow_key(None)` / `focus_follow_command(None)` use desktop fallback.

- [ ] **Step 4: Run app test**

Run: `cargo test -p clipline-app focus_follow_no_focused_game_sends_desktop_fallback`

Expected: PASS.

### Task 2: Capture Desktop With Audio On Fallback

**Files:**
- Modify: `apps/clipline-app/src/service.rs`
- Test: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Consumes: `SwitchCaptureTarget::Desktop`
- Produces: `CaptureKind::Desktop`

- [ ] **Step 1: Write failing service tests**

Add tests proving desktop fallback is a distinct state and keeps audio unmuted:

```rust
#[test]
fn focus_run_state_desktop_fallback_keeps_audio_public() {
    let mut state = FocusRunState::from_options(&ServiceOptions::default());
    state.apply_target(&SwitchCaptureTarget::Desktop);

    assert_eq!(state.capture_kind, CaptureKind::Desktop);
    assert_eq!(state.active_game, None);
    assert_eq!(state.active_game_plugin_id, None);
    assert_eq!(state.recording_mode, RecordingMode::ReplaysOnly);
    assert_eq!(state.slate_reason, None);
    assert!(!state.should_mute_audio());
}

#[test]
fn full_session_transition_keeps_recording_when_focus_moves_to_desktop() {
    let old = FocusRunState {
        capture_kind: CaptureKind::Game,
        active_game: Some(ActiveGame { id: "a".into(), name: "A".into() }),
        active_game_plugin_id: None,
        recording_mode: RecordingMode::FullSession,
        slate_reason: None,
    };
    let next = FocusRunState {
        capture_kind: CaptureKind::Desktop,
        active_game: None,
        active_game_plugin_id: None,
        recording_mode: RecordingMode::ReplaysOnly,
        slate_reason: None,
    };

    assert_eq!(
        full_session_transition(Some("a"), &old, &next),
        FullSessionTransition::None
    );
}
```

- [ ] **Step 2: Run failing service tests**

Run: `cargo test -p clipline-app focus_run_state_desktop_fallback_keeps_audio_public full_session_transition_keeps_recording_when_focus_moves_to_desktop`

Expected: FAIL because `CaptureKind::Desktop` and `should_mute_audio` do not exist.

- [ ] **Step 3: Implement desktop capture state**

Add `CaptureKind::Desktop`, `SwitchTargetIdentity::Desktop`, and service handling that opens `CaptureSource::PrimaryMonitor` for desktop fallback. Change audio privacy writes to call `focus_state.should_mute_audio()` so only slate mutes audio.

- [ ] **Step 4: Run focused service tests**

Run: `cargo test -p clipline-app focus_run_state_desktop_fallback_keeps_audio_public full_session_transition_keeps_recording_when_focus_moves_to_desktop switch_target_identity_dedupes_desktop_slate_and_window`

Expected: PASS.

### Task 3: Update User-Facing Copy And Metadata

**Files:**
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `handoff.md`

**Interfaces:**
- Consumes: `capture_kind: "desktop"`
- Produces: truthful Settings > Games status copy.

- [ ] **Step 1: Write failing UI contract expectations**

Update the existing focus-follow contract to require:

```rust
"Desktop fallback active. Focus a saved game to switch back."
"Recording the focused saved game; other windows record from the desktop."
```

- [ ] **Step 2: Run failing UI contract**

Run: `cargo test -p clipline-app --test ui_contract focus_follow_settings_contract`

Expected: FAIL while UI still mentions privacy slate.

- [ ] **Step 3: Update UI copy and handoff**

Change Settings > Games helper text and active status copy from privacy slate wording to desktop fallback wording. Update `handoff.md` to describe desktop fallback with continuing audio.

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
cargo test -p clipline-app focus_follow_no_focused_game_sends_desktop_fallback focus_run_state_desktop_fallback_keeps_audio_public full_session_transition_keeps_recording_when_focus_moves_to_desktop
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
git commit -m "fix(capture): use desktop fallback for focus-follow gaps"
git push
```

Expected: branch pushed to PR #82.

- [ ] **Step 4: Relaunch app**

Stop any existing `clipline-app.exe`, rebuild if needed, and start `target/debug/clipline-app.exe` from this worktree.

