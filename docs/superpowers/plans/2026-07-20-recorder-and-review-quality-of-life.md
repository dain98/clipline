# Recorder And Review Quality-Of-Life Plan

> **For agentic workers:** Execute this plan task-by-task with strict TDD. Steps use checkbox (`- [ ]`) syntax and remain unticked by repository convention.

**Goal:** Add an opt-in games-only recorder policy, an explicit administrator restart for elevated games, a direct route to newly exported trims, and fullscreen review playback.

**Architecture:** Persist the games-only policy beside game detection and keep the recorder's requested state separate from whether a service is currently running, so Clipline can remain armed while no game is open without capturing the desktop. Restore the existing exact-process UAC handoff design: the normal process launches its own executable with `runas`, the elevated child verifies the parent's creation time and waits for that exact process to exit before entering Tauri, and cancellation leaves the normal process alive. Keep trim navigation and fullscreen behavior in the renderer: export results already contain enough clip metadata to open immediately, while the stage can use WebView2's standard Fullscreen API without broadening Tauri capabilities.

**Defaults and constraints:**

- Desktop replay buffering remains enabled by default; `pause_when_no_game` defaults to `false` for new and existing settings.
- The games-only policy is effective only while automatic game detection is enabled.
- An armed recorder paused by policy exposes a distinct waiting state, disables Save Replay, resumes automatically when an enabled game appears, and can still be manually stopped.
- Leaving a game stops the current service without recovering its just-finalized temporary files as an abandoned run; entering a game starts a fresh replay buffer.
- Administrator restart is always explicit, applies only to the current launch, uses only the current executable path, and preserves the normal process if UAC is cancelled or denied.
- Fullscreen applies to the video stage and controls, exits through the platform Escape behavior, and does not alter application-window fullscreen state or Tauri permissions.

## Task 1: Lock persisted settings and UI contracts

**Files:**
- Modify: `apps/clipline-app/src/settings/games.rs`
- Modify: `apps/clipline-app/src/settings/tests.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/settings.js`
- Modify: `apps/clipline-app/ui/main.js`

- [ ] Add failing settings tests for the false default, serialization, and legacy settings migration.
- [ ] Add failing UI contracts for the games-only toggle, administrator restart action, trim-success action, and fullscreen controls/shortcut.
- [ ] Run the focused tests and verify RED.
- [ ] Add the settings field and complete the renderer settings wiring.

## Task 2: Pause and resume the recorder around detected games

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/service.rs`
- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/settings.js`

- [ ] Add failing neutral app-state tests for the effective policy, no-game pause, game-open resume, manual stop while waiting, settings transitions, and stale service generations.
- [ ] Extend recorder status with `waiting_for_game`, defaulting false for service-originated status.
- [ ] Keep `recording_desired` true while policy-paused, but remove the active sender and stop the service without an authoritative stopped event.
- [ ] Emit a synthetic waiting status when the policy suppresses service startup or stops a running fallback capture.
- [ ] Start a fresh service when a detected game appears and preserve existing restart generation/race protections.
- [ ] Surface `Waiting` in the rail and keep Save Replay disabled until recording is active.
- [ ] Run focused Rust and UI-contract tests.

## Task 3: Restore the explicit elevated-game restart safely

**Files:**
- Modify: `apps/clipline-app/src/windows/mod.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add failing Windows tests for handoff argument parsing, exact process identity, missing-parent races, and shell result classification.
- [ ] Add `runas` launch and exact-parent wait wrappers, keeping every new unsafe call in the Windows module.
- [ ] Parse and wait on the verified handoff before Tauri and the single-instance plugin start.
- [ ] Register `restart_as_administrator`; quit only after Windows successfully creates the elevated child.
- [ ] Restore the modal's affirmative action, in-flight cancellation guards, buffer-reset explanation, and retry behavior after UAC cancellation.
- [ ] Run focused Windows, app, and UI-contract tests.

## Task 4: Route successful trims directly to their result

**Files:**
- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add a failing UI contract for a status action beside `#deck-status`.
- [ ] Let deck status own and clear an optional action together with transient status text.
- [ ] On successful export, insert the returned clip into the cache and show `Open clip`; clicking it opens that exact export without a library round trip.
- [ ] Clear stale actions when a newer status replaces the export notification.
- [ ] Run focused UI-contract tests.

## Task 5: Add fullscreen review playback

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/ui/styles.css`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] Add failing pure keyboard-intent coverage mapping `F` to fullscreen.
- [ ] Add a failing UI contract for the fullscreen transport button, Fullscreen API calls, state synchronization, and fullscreen stage sizing.
- [ ] Toggle fullscreen on `#stage-frame`, keep its control state synchronized through `fullscreenchange`, and report API failures in the existing error area.
- [ ] Ensure fullscreen CSS overrides inline aspect sizing and retains the stage overlay/controls.
- [ ] Run focused player-core and UI-contract tests.

## Task 6: Quality gates, documentation, and native launch

**Files:**
- Modify: `ddoc.md`
- Modify: `handoff.md`

- [ ] Update product documentation for the new opt-in recorder lifecycle, elevation tradeoff, trim navigation, and fullscreen review control.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test --workspace`.
- [ ] Run fresh-cache app Clippy, then `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `git diff --check` and review the complete scoped diff.
- [ ] Stop an existing `clipline-app.exe`, build, and launch `cargo run -p clipline-app` for user acceptance.
- [ ] Manually verify waiting/resume behavior, UAC cancel and accept paths, exact exported-trim navigation, fullscreen button/`F`/Escape behavior, and ordinary desktop buffering with the new setting disabled.
- [ ] Update `handoff.md` with concrete implementation and verification results, then commit the implementation.
