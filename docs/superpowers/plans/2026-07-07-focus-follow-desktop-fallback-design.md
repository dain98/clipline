# Focus-Follow Capture Target Fallback Design

## Goal

When Follow focused game windows is enabled, leaving a saved game should keep recording useful footage and audio by switching back to the configured Capture target instead of a privacy slate. If the user selected SET REGION, the gap records that region. Users can crop or omit private sections before upload.

## Behavior

- A focused enabled saved game still captures that game window and attributes clips to that game.
- No enabled saved game in the foreground switches capture to the configured Capture target and keeps output audio flowing.
- Full-session recordings stay open across Capture target fallback, so alt-tabbing out of a full-session game does not create a new session clip.
- Marker sources are detached while on Capture target fallback, matching the old slate behavior, so game events only attach to the focused saved game.
- Privacy slate and muted audio remain available for hard switch failures, where Clipline cannot open the requested target and needs a safe fallback frame.

## Architecture

The app layer should stop treating `None` from focused-game detection as a slate command. Instead, it should send a Capture target fallback switch target and dedupe it separately from windows and hard-failure slate.

The service layer should model Capture target fallback as its own capture kind. It opens `ServiceOptions.capture_source` through the normal screen-capture path, reports `capture_kind: "capture_target"` to the UI, writes `"capture_target"` source switches to sidecars, and does not enable `AudioPrivacyState`. Existing slate behavior remains for `SwitchFailed`.

## Testing

- App tests should prove `prepare_focus_follow_update(None)` sends a Capture target fallback command.
- Service tests should prove Capture target fallback is deduped separately, does not mute audio, detaches marker attribution, keeps full-session recording open, and uses the configured `CaptureSource`.
- UI contract tests should require Capture target fallback copy rather than privacy-slate copy for normal no-game-focused behavior.
