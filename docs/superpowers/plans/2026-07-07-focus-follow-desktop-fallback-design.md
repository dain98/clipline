# Focus-Follow Desktop Fallback Design

## Goal

When Follow focused game windows is enabled, leaving a saved game should keep recording useful footage and audio by switching to the desktop instead of a privacy slate. Users can crop or omit private sections before upload.

## Behavior

- A focused enabled saved game still captures that game window and attributes clips to that game.
- No enabled saved game in the foreground switches capture to the primary desktop/monitor and keeps output audio flowing.
- Full-session recordings stay open across desktop fallback, so alt-tabbing out of a full-session game does not create a new session clip.
- Marker sources are detached while on desktop fallback, matching the old slate behavior, so game events only attach to the focused saved game.
- Privacy slate and muted audio remain available for hard switch failures, where Clipline cannot open the requested window and needs a safe fallback frame.

## Architecture

The app layer should stop treating `None` from focused-game detection as a slate command. Instead, it should send a new desktop fallback switch target and dedupe it separately from windows and hard-failure slate.

The service layer should model desktop fallback as its own capture kind. It opens `CaptureSource::PrimaryMonitor`, reports `capture_kind: "desktop"` to the UI, writes `"desktop"` source switches to sidecars, and does not enable `AudioPrivacyState`. Existing slate behavior remains for `SwitchFailed`.

## Testing

- App tests should prove `prepare_focus_follow_update(None)` sends a desktop fallback command.
- Service tests should prove desktop fallback is deduped separately, does not mute audio, detaches marker attribution, and keeps full-session recording open.
- UI contract tests should require desktop fallback copy rather than privacy-slate copy for normal no-game-focused behavior.

