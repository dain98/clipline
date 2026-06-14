# 2026-06-13 Full Session Recording

Goal: make each custom game's `recording_mode` real. Games set to `full_session`
should automatically save a finalized MP4 for the whole detected game window
session, while games set to `replays_only` keep the current replay-buffer-only
behavior.

Plan:

- Add a neutral recorder full-session sink that writes sealed GOP segments to a
  Hybrid MP4 as they are produced, sharing the existing encoder and audio tracks
  with the replay ring.
- Keep Save Replay working during full-session recording by continuing to push
  every sealed segment into the replay ring.
- Thread each detected custom game's recording mode through game detection and
  runtime service options.
- When a full-session game capture starts, create a `session_<timestamp>.mp4`
  under that recorder run's session folder. On game disappearance, target
  switch, service stop, or capture end, finalize it, write marker sidecars,
  enforce storage quota, emit the normal saved event, and refresh the library.
- Update tests at the pipeline, settings/game-detection, app-state, and UI
  contract layers. Run workspace tests, clippy, build, then reopen the app for
  manual testing.

