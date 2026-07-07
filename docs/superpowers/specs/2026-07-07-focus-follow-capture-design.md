# Focus-Follow Capture Design

## Goal

Add an optional Game detection mode that follows whichever enabled saved game window has foreground focus, while preserving one continuous replay buffer. When focus leaves enabled games, Clipline records a neutral privacy slate instead of switching to monitor capture or restarting the recorder.

## User-Facing Behavior

- Settings > Games gains an explicit `Follow focused game windows` option under Game detection.
- The option is off by default so existing automatic game detection behavior stays unchanged.
- When enabled, Clipline records the foreground enabled game window.
- If the user alt-tabs from one enabled game to another enabled game, the replay buffer stays alive and future Save Replay clips can include both games across the switch.
- If focus moves to a non-game window, the recording shows a neutral slate such as `Clipline is not capturing this window`.
- The neutral slate preserves timeline continuity and avoids recording private windows, chats, browsers, password managers, launchers, or Clipline's own UI.
- When focus returns to an enabled game, recording switches back to that game window without clearing the replay ring.
- System/output audio and microphone audio keep using the current configured behavior. Dynamic per-process audio following is not part of this feature.
- Existing manual Save Replay, game attribution, marker sidecars, and full-session files continue to work.

## Architecture

The current game detector restarts the recorder service whenever the detected active window identity changes. Focus-follow capture changes that boundary: the detector still owns foreground-window matching, but it sends target changes into the running recorder instead of forcing a service restart.

The recorder owns one clock, one encoder, one replay ring, and one full-session writer state. A new switchable video source sits inside the recorder pipeline and can replace the active WGC window source with another WGC window source or a generated slate source. All sources emit frames on the same `RelativeClock` and into the same fixed output size, so MP4 track parameters do not change mid-stream.

The first implementation should reuse the existing single-recorder pipeline. It should not pre-open multiple WGC sessions or build a multi-window compositor.

The encoder canvas is chosen once when the recorder starts. If a focused enabled game is available at startup, use its first frame as the source for output dimensions. If no focused enabled game is available, initialize the canvas from the configured fallback capture source, then switch immediately to the privacy slate. Later game windows and slate frames scale or pad into that fixed canvas.

## Components

- `crates/clipline-capture/src/windows/window.rs`
  - Add a foreground-window query helper around `GetForegroundWindow`.
  - Return the same visible-window metadata shape used by game detection when the foreground HWND is capturable.
  - Exclude Clipline's own process at the app layer, matching existing window-list behavior.

- `apps/clipline-app/src/settings/games.rs`
  - Persist a new focus-follow setting under `games`.
  - Default it to `false`.
  - Keep old settings readable by defaulting missing fields.

- `apps/clipline-app/src/games.rs`
  - Add foreground-aware detection for focus-follow mode.
  - Match the foreground window against built-in plugins first, then enabled custom games.
  - Return `None` when the foreground window is not an enabled game, even if another enabled game is running in the background.

- `apps/clipline-app/src/app.rs`
  - Keep existing service-restart behavior for normal Game detection.
  - When focus-follow mode is enabled, avoid restarting the service for focus-only target changes.
  - Send a new recorder command for game-window targets and slate targets.
  - Continue to restart the service for setting changes that truly require a new encoder, audio graph, storage root, or capture mode.

- `apps/clipline-app/src/service.rs`
  - Add a command such as `Cmd::SwitchCapture(SwitchCaptureTarget)`.
  - Add `SwitchCaptureTarget::Window { hwnd, title, active_game, active_game_plugin_id, recording_mode }`.
  - Add `SwitchCaptureTarget::Slate { reason }`.
  - Route target changes to the live recorder without rebuilding replay storage.
  - Update status events so the sidebar can distinguish game capture from privacy-slate capture.

- `crates/clipline-capture/src/pipeline.rs`
  - Add an API that lets the app-level live recorder switch capture sources while preserving pending GOP state, replay storage, audio sources, and full-session state.
  - The API should force or request a keyframe after a source switch where the active encoder can support it; if an encoder cannot force a keyframe, the switch still proceeds and the next natural keyframe becomes the clean save boundary.

- `crates/clipline-capture/src/windows/wgc.rs`
  - Reuse existing `WgcCapture::for_window_client_on(device, hwnd, clock)` for each focused game window.
  - Keep the caller-provided D3D11 device and `RelativeClock`.
  - Treat WGC init failure during a switch as a recoverable target error.

- New capture source wrapper
  - Add a focused wrapper near the live service layer, rather than a broad trait redesign.
  - The wrapper holds either a WGC window capture source or a slate source.
  - It emits frames at the configured FPS using existing cadence behavior.
  - It scales or pads frames into the fixed encoder canvas.

## Data Flow

1. The game detector wakes at the current polling cadence.
2. If focus-follow is off, it uses the existing detected-game path.
3. If focus-follow is on, it queries the foreground HWND and matches only that window against enabled games.
4. If the foreground window is an enabled game, the app sends `Cmd::SwitchCapture(Window { ... })`.
5. If the foreground window is absent, Clipline itself, minimized, hidden, or not an enabled game, the app sends `Cmd::SwitchCapture(Slate { reason })`.
6. The service opens the new WGC source or slate source on the recorder thread.
7. The recorder swaps the active video source while keeping the same encoder, audio sources, replay storage, marker log, and full-session state.
8. The replay ring receives one continuous encoded timeline.
9. Save Replay writes the trailing window across source switches when the requested window overlaps them.

## Timeline and Metadata

- Record switches as sidecar metadata so the review UI can eventually show source changes.
- V1 metadata can be minimal: timestamp, target kind, game id/name when present, and slate reason when not.
- Existing game event markers remain owned by active built-in plugin event sources.
- League markers should only be attributed while the active focused game is the League plugin.
- Full-session file attribution uses the active game at the time the full session starts.
- Switching from game capture to slate does not split or rename an already-open full-session file.
- Switching from one full-session game to another follows the Full-Session Behavior section.

## Full-Session Behavior

- Replays-only games only affect the replay buffer.
- A full-session game starts a full-session sink when it becomes the focused game and no compatible full-session sink is active.
- Switching temporarily to slate does not finalize the full-session file.
- Switching from one full-session game to a different full-session game finalizes the old full-session sink and starts a new one.
- If a full-session game window disappears, the current existing disappearance/finalization behavior still applies.

## Error Handling

- If a focused game window disappears before WGC opens it, switch to the privacy slate and emit a non-fatal warning.
- If WGC fails while switching to a focused game, keep the previous source if it is still valid; otherwise switch to the privacy slate.
- If repeated foreground polling reports the same target, do nothing.
- If the command channel is disconnected or the recorder is stopped, the detector should continue to update UI status but not try to restart recording just for focus-follow.
- If the slate source cannot allocate a GPU texture, surface a recorder error; this is equivalent to capture initialization failure.

## Testing

- Unit-test foreground matching in `apps/clipline-app/src/games.rs` using synthetic `CapturableWindow` values and an explicit foreground handle.
- Unit-test settings migration/defaults for the new focus-follow field.
- Unit-test app runtime behavior so focus-follow target changes send switch commands instead of stop/spawn restarts.
- Unit-test service command handling with mock switch targets at the command-to-state boundary.
- Unit-test capture wrapper behavior with mock frame sources: source A, slate, source B must produce monotonically increasing frame timestamps without clearing pending state.
- Extend `apps/clipline-app/tests/ui_contract.rs` for the new Settings > Games checkbox/label.
- Run `cargo test --workspace`.
- Run `cargo clippy --workspace --all-targets -- -D warnings`.
- Before local runtime testing, stop any existing `clipline-app.exe`, rebuild, and launch with `cargo run -p clipline-app`.

## Out of Scope

- Multi-window compositor or pre-opened WGC sessions.
- Recording non-game foreground windows.
- Falling back to display/monitor capture when focus leaves enabled games.
- Dynamic per-process audio following.
- Native UI for visualizing switch markers on the review timeline.
- Frame-perfect keyframe insertion on every encoder backend when the backend does not already expose a force-keyframe control.
- A new game-plugin system; focus-follow uses the existing built-in plugin and custom-game matching layers.
