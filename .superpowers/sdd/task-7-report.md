# Task 7 Report: Wire Service Switching, Audio Muting, Markers, And Full Sessions

## What I implemented

- Added recorder-side focus-follow run state in `apps/clipline-app/src/service.rs`:
  - `CaptureKind`
  - `FocusRunState`
  - `CaptureSwitchLog`
  - `FullSessionTransition`
  - full-session transition reconciliation on successful focused-window switches
- Wrapped recorder audio inputs with `clipline_capture::PrivacyAudioGate` and drove slate/game muting from runtime focus state.
- Updated `Cmd::SwitchCapture` handling to:
  - apply successful focused-window/slate transitions to service state
  - push source-switch timeline entries
  - fall back to privacy slate on switch failure
  - preserve retryability by emitting `Event::FocusFollowRetry` for failed window switches
  - refresh status immediately after state changes
- Gated marker / player-summary / match lifecycle attribution to the currently active built-in plugin.
- Switched save-time session metadata attribution from static startup options to the mutable live focus state.
- Extended `<clip>.markers.json` with `source_switches` and `ClipSourceSwitch` entries.
- Propagated the new sidecar field through crop/export/read helper code and compile-required test fixtures.
- Added status payload fields for the frontend:
  - `capture_kind`
  - `capture_label`
  - `slate_reason`
- Wired frontend state in:
  - `ui/app-core.js`
  - `ui/main.js`
  - `ui/settings.js`
  so the Games settings status shows `Privacy slate active. Focus a saved game to resume capture.` when focus-follow is enabled and the recorder is on slate.
- Added/updated tests for service state helpers and UI contract wiring.

## TDD evidence

### RED

Because Cargo rejects the brief's original multi-filter form for this target layout, I used equivalent single-filter commands.

1. UI contract RED

Command:

```powershell
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --exact
```

Result:

- Failed as expected.
- Key failure:
  - `focus-follow status wiring must include var capturePrivacyState = { kind: "game", label: null, slate_reason: null }`

Reason:

- Frontend privacy-state wiring and slate status text did not exist yet.

2. Service-state RED

Command:

```powershell
cargo test -p clipline-app focus_run_state_uses_latest_active_game_for_save_meta
```

Result:

- Failed as expected.
- Key failures:
  - `cannot find struct, variant or union type FocusRunState in this scope`
  - `cannot find function full_session_transition in this scope`
  - `use of undeclared type CaptureSwitchLog`

Reason:

- Task 7 service-state helpers and switch-log support did not exist yet.

### GREEN

Focused GREEN verification:

```powershell
cargo test -p clipline-app focus_run_state_
cargo test -p clipline-app full_session_transition_splits_between_different_full_session_games
cargo test -p clipline-app capture_switch_log_filters_to_saved_window
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --exact
cargo fmt -- --check
```

Results:

- All commands passed.

Final verification:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Results:

- Both commands passed.

## Tests run and results

- `cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --exact` - PASS
- `cargo test -p clipline-app focus_run_state_` - PASS
- `cargo test -p clipline-app full_session_transition_splits_between_different_full_session_games` - PASS
- `cargo test -p clipline-app capture_switch_log_filters_to_saved_window` - PASS
- `cargo fmt -- --check` - PASS
- `cargo test --workspace` - PASS
- `cargo clippy --workspace --all-targets -- -D warnings` - PASS

## Files changed

- `crates/clipline-events/src/markers.rs`
- `crates/clipline-events/src/lib.rs`
- `apps/clipline-app/src/service.rs`
- `apps/clipline-app/ui/app-core.js`
- `apps/clipline-app/ui/main.js`
- `apps/clipline-app/ui/settings.js`
- `apps/clipline-app/tests/ui_contract.rs`
- compile-required sidecar call-site updates:
  - `apps/clipline-app/src/library.rs`
  - `apps/clipline-app/src/cloud.rs`
  - `apps/clipline-app/src/osu_enrichment.rs`
  - `apps/clipline-app/src/util.rs`

## Self-review findings

- No blocking issues found after diff review and full verification.
- I intentionally kept the extra file edits to compile-required sidecar propagation only.
- `FocusRunState::from_options` treats non-focus-follow runs as `game` by default so ordinary non-focus-follow captures do not incorrectly present as slate or mute audio when `active_game` is `None`.

## Concerns

- None.

---

# Task 7 Review Fix: Marker Source Switching

## What I implemented

- Added regression coverage for marker-source identity switching and stale-plugin marker rejection in [apps/clipline-app/src/service.rs](C:/Users/dain/Projects/clipline/.worktrees/codex-focus-follow-capture/apps/clipline-app/src/service.rs).
- Replaced the recorder's startup-only marker-source choice with a small `MarkerRuntime` helper that:
  - tracks the current marker-source key,
  - respawns the marker receiver when the focused plugin's event-source identity changes,
  - keeps using current `FocusRunState` gating so stale markers from the previous plugin are rejected after a focus switch.
- Kept the existing Task 6 retry behavior intact; `warn_retryable_switch_failure` and `Event::FocusFollowRetry` behavior were unchanged and re-verified by the existing service test.

## TDD evidence

### RED

Command:

```powershell
cargo test -p clipline-app marker_source_key_switches_from_slate_to_plugin_when_focus_enters_event_source_game
```

Result:

- Failed as expected.
- Key failures:
  - `cannot find function marker_source_key_for in this scope`
  - `use of undeclared type MarkerSourceKey`

Reason:

- The service had no marker-source identity helper and no runtime seam for switching marker receivers across focus/plugin changes.

### GREEN

Focused GREEN verification:

```powershell
cargo test -p clipline-app service::tests::
cargo fmt -- --check
```

Results:

- `cargo test -p clipline-app service::tests::` passed with all 36 service tests green, including:
  - `marker_source_key_switches_from_slate_to_plugin_when_focus_enters_event_source_game`
  - `marker_source_key_distinguishes_between_plugin_event_sources`
  - `focus_run_state_rejects_stale_plugin_markers_after_plugin_switch`
  - `retryable_window_switch_failure_emits_retry_event_before_error`
- `cargo fmt -- --check` initially failed on formatting only; after `cargo fmt`, `cargo fmt -- --check` passed.

## Tests run and results

- `cargo test -p clipline-app marker_source_key_switches_from_slate_to_plugin_when_focus_enters_event_source_game` - FAIL (expected RED: missing `marker_source_key_for` / `MarkerSourceKey`)
- `cargo test -p clipline-app marker_source_` - PASS
- `cargo test -p clipline-app focus_run_state_` - PASS
- `cargo test -p clipline-app service::tests::` - PASS
- `cargo fmt -- --check` - FAIL (formatting only)
- `cargo fmt` - PASS
- `cargo fmt -- --check` - PASS

## Files changed

- `apps/clipline-app/src/service.rs`
- `.superpowers/sdd/task-7-report.md`

## Self-review findings

- The fix stays within the service seam and does not broaden the plugin system.
- Marker-source switching now follows focused plugin identity instead of the startup option snapshot.
- Stale plugin markers are still rejected by `FocusRunState` after focus changes.
- The retryable failed-window-switch behavior from Task 6 is still covered and passing.

## Concerns

- None.
