# Focus-Follow Capture Design

## Goal

Add an optional Game detection mode that follows whichever enabled saved game window has foreground focus, while preserving one continuous replay buffer. When focus leaves enabled games, Clipline records a neutral privacy slate and muted audio instead of switching to monitor capture or restarting the recorder.

## User-Facing Behavior

- Settings > Games gains an explicit `Follow focused game windows` option under Game detection.
- The option is off by default so existing automatic game detection behavior stays unchanged.
- When enabled, Clipline records the foreground enabled game window.
- If the user alt-tabs from one enabled game to another enabled game, the replay buffer stays alive and future Save Replay clips can include both games across the switch.
- If focus moves to a non-game window, the recording shows a neutral slate such as `Clipline is not capturing this window`.
- The neutral slate preserves timeline continuity and avoids recording private windows, chats, browsers, password managers, launchers, or Clipline's own UI.
- Output audio and microphone audio are muted while the slate is active. Save Replay clips that overlap a private window contain slate video and silence for that interval.
- When focus returns to an enabled game, recording switches back to that game window without clearing the replay ring.
- When a focused game is active, output audio and microphone audio keep using the current configured behavior. Dynamic per-process audio following is not part of this feature.
- Existing manual Save Replay, game attribution, marker sidecars, and full-session files continue to work.

## Architecture

The current game detector restarts the recorder service whenever the detected active window identity changes. Focus-follow capture changes that boundary: the detector still owns foreground-window matching, but it sends target changes into the running recorder instead of forcing a service restart.

The recorder owns one clock, one encoder, one replay ring, and one full-session writer state. A new mutable service-layer capture source is the recorder's compile-time `C: CaptureEngine`. It can replace its active WGC window source with another WGC window source or a generated slate source. The `Recorder` generic type does not need a broad source-swap API because the source mutability lives inside that concrete `CaptureEngine`.

The first implementation reuses the existing single-recorder pipeline. It does not pre-open multiple WGC sessions or build a multi-window compositor.

The encoder canvas is chosen once when the recorder starts. If a focused enabled game is available at startup, use its first frame as the source for output dimensions. If no focused enabled game is available, derive the canvas from the configured fallback display/region metadata and start directly on the privacy slate without opening monitor capture or pushing a desktop frame into the ring. Later game windows and slate frames fit into that fixed canvas.

The existing NV12 converter already rebuilds its video processor when input texture dimensions change, so switching between differently-sized WGC textures does not require wrapper-side scaling for correctness. V1 adds aspect-preserving fit with letterbox/pillarbox instead of stretching mismatched source aspects into the output canvas. The slate is generated at the encoder canvas size and does not require letterboxing.

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
  - Track the current focus-follow target separately from existing detected-game state so repeated foreground polls do not send duplicate switch commands.

- `apps/clipline-app/src/service.rs`
  - Add a command such as `Cmd::SwitchCapture(SwitchCaptureTarget)`.
  - Add `SwitchCaptureTarget::Window { hwnd, title, active_game, active_game_plugin_id, recording_mode }`.
  - Add `SwitchCaptureTarget::Slate { reason }`.
  - Route target changes to the live recorder without rebuilding replay storage.
  - Keep mutable run state for current capture target, current active game, current active plugin id, current recording mode, current marker-source gate, and current full-session attribution.
  - Use mutable current active game state for `write_session_game_meta`, saved clip events, and sidecar attribution instead of immutable `opts.active_game`.
  - Gate plugin marker ingestion by the current focused plugin id. League marker events are ignored or paused while the focused target is slate or a non-League game.
  - Restart or reconfigure marker sources only when a later implementation needs multiple plugin event sources; V1 keeps the startup marker source and gates its output against the current focused plugin id.
  - Mute audio during slate intervals by wrapping each configured `AudioSource` in a focus-aware audio gate. The gate still polls inner sources to drain device queues, but emits encoded silence packets for slate intervals so muxed audio timelines stay continuous.
  - Finalize and start full-session sinks on game-to-game focus switches that change the active full-session game. Treat this as real recorder work that can briefly block while the writer finalizes.
  - Update status events so the sidebar can distinguish game capture from privacy-slate capture.

- `crates/clipline-capture/src/windows/wgc.rs`
  - Reuse existing `WgcCapture::for_window_client_on(device, hwnd, clock)` for each focused game window.
  - Keep the caller-provided D3D11 device and `RelativeClock`.
  - Treat WGC init failure during a switch as a recoverable target error.
  - Close/drop the old `WgcCapture` before opening the next one so two frame-arrival handlers do not transiently share the immediate context.

- `crates/clipline-capture/src/windows/nv12.rs`
  - Extend `VideoConverter` with an aspect-preserving fit mode.
  - Clear the output frame to black and blit the source into a centered destination rectangle when source and output aspects differ.
  - Keep the existing stretch behavior available for existing callers if needed, but use fit mode for focus-follow capture.

- New mutable capture source
  - Add a focused wrapper near the live service layer, rather than a broad trait redesign.
  - The wrapper holds either a WGC window capture source or a slate source.
  - It emits frames at the configured FPS using existing cadence behavior.
  - It produces monotonically increasing timestamps across source switches.
  - It exposes a small switch method that the service loop can call when `Cmd::SwitchCapture` arrives.

- New slate source
  - Generate a GPU texture at the fixed encoder canvas size.
  - Render a neutral, non-game frame with simple local drawing primitives or a precomputed pixel buffer uploaded to the D3D11 texture.
  - Reuse the slate texture while slate remains active.

- New audio gate
  - Wrap each configured `AudioSource`.
  - Share the active capture privacy state with the service switch handler.
  - When game capture is active, pass through inner packets.
  - When slate is active, drain and discard inner packets up to the requested timestamp and emit Opus silence packets matching the elapsed 20 ms packet cadence.

## Data Flow

1. The game detector wakes at the current polling cadence.
2. If focus-follow is off, it uses the existing detected-game path.
3. If focus-follow is on, it queries the foreground HWND and matches only that window against enabled games.
4. If the foreground window is an enabled game, the app sends `Cmd::SwitchCapture(Window { ... })`.
5. If the foreground window is absent, Clipline itself, minimized, hidden, or not an enabled game, the app sends `Cmd::SwitchCapture(Slate { reason })`.
6. The service opens the new WGC source or slate source on the recorder thread. Game-window switches close the old WGC session before opening the new one.
7. The mutable capture source swaps active video source while the `Recorder` keeps the same encoder, replay storage, marker log, audio gates, and full-session state.
8. The audio gates pass through configured audio during game capture and emit silence during slate capture.
9. The replay ring receives one continuous encoded timeline.
10. Save Replay writes the trailing window across source switches when the requested window overlaps them.

## Timeline and Metadata

- Record switches as sidecar metadata so the review UI can eventually show source changes.
- V1 metadata can be minimal: timestamp, target kind, game id/name when present, and slate reason when not.
- Existing game event markers remain owned by active built-in plugin event sources, but ingestion is gated by the current focused plugin id.
- League markers are recorded only while the active focused game is the League plugin.
- Save-time game attribution uses the current active game at the time of save, not the game that was active when the recorder thread started.
- Full-session file attribution uses the active game at the time the full session starts.
- Switching from game capture to slate does not split or rename an already-open full-session file.
- Switching from one full-session game to another follows the Full-Session Behavior section.
- A Save Replay window that spends most of its duration on slate is allowed to produce mostly slate footage and silence; this is the expected privacy tradeoff.

## Full-Session Behavior

- Replays-only games only affect the replay buffer.
- A full-session game starts a full-session sink when it becomes the focused game and no compatible full-session sink is active.
- Switching temporarily to slate does not finalize the full-session file.
- Switching from one full-session game to a different full-session game finalizes the old full-session sink using the existing temp-file, marker-sidecar, min-duration, and final-rename path, then starts a new sink for the new game.
- Mid-run full-session finalization can briefly block the recorder loop while the writer thread joins. This is acceptable for V1 because it happens only on game-to-game focus switches, not on ordinary frame capture.
- If a full-session game window disappears, the current existing disappearance/finalization behavior still applies.

## Error Handling

- If a focused game window disappears before WGC opens it, switch to the privacy slate and emit a non-fatal warning.
- If WGC fails while switching to a focused game, keep the previous source if it is still valid; otherwise switch to the privacy slate.
- If repeated foreground polling reports the same target, do nothing.
- If the command channel is disconnected or the recorder is stopped, the detector should continue to update UI status but not try to restart recording just for focus-follow.
- If the slate source cannot allocate a GPU texture, surface a recorder error; this is equivalent to capture initialization failure.
- If the audio gate cannot generate encoded silence, fail recorder initialization rather than recording private-window audio under a slate.
- If aspect-fit blitting fails for a switched game frame, surface a recorder error rather than falling back to stretched output silently.

## Testing

- Unit-test foreground matching in `apps/clipline-app/src/games.rs` using synthetic `CapturableWindow` values and an explicit foreground handle.
- Unit-test settings migration/defaults for the new focus-follow field.
- Unit-test app runtime behavior so focus-follow target changes send switch commands instead of stop/spawn restarts.
- Unit-test service command handling with mock switch targets at the command-to-state boundary.
- Unit-test mutable capture source behavior with mock frame sources: source A, slate, source B must produce monotonically increasing frame timestamps.
- Unit-test recorder-level behavior that switching the mutable source does not clear pending GOP state or replay storage.
- Unit-test audio gate behavior: game intervals pass packets through, slate intervals emit silence, and inner sources are still drained while muted.
- Unit-test aspect-fit rectangle math for same-aspect, pillarbox, and letterbox cases.
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
- Frame-perfect keyframe insertion on every encoder backend when the backend does not already expose a force-keyframe control. V1 relies on the next natural keyframe after a switch.
- A new game-plugin system; focus-follow uses the existing built-in plugin and custom-game matching layers.
