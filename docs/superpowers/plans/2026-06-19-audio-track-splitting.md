# Audio Track Splitting

## Goal

Record output audio as per-app/process MP4 tracks, keep microphone audio as its own track,
persist labels in Clipline's sidecar metadata, let the review UI show selectable tracks, and let
upload choose which audio tracks to include without mutating the local source clip.

## Scope

- Enumerate current Windows render sessions at recorder start and capture each process with
  `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` when available.
- Fall back to the old mixed output loopback track if process-loopback capture is unavailable or no
  process track starts.
- Keep microphone capture as a separate Opus track when enabled.
- Keep all enabled tracks in saved replays and full-session recordings.
- Persist user-facing track metadata in `<clip>.markers.json` so the Library can render tracks
  without parsing MP4s during startup.
- Add review/upload UI checkboxes backed by stable track ids.
- For upload, remux a temporary in-memory MP4 containing the original video track plus only selected
  audio tracks; upload checksum/file validation uses the remuxed bytes.

## Out of Scope

- Dynamic discovery of new audio sessions after a recording has already started.
- Collapsing multi-process apps into one branded track beyond the process tree Windows captures.
- Re-encoding audio or mixing selected tracks into one upload track.

## TDD Steps

- [ ] Add `ClipAudioTrack` sidecar schema with backwards-compatible serde defaults.
- [ ] Add tests that marker sidecars with audio tracks serialize/deserialize and that cropped
      exports preserve/crop track metadata.
- [ ] Add Windows process-loopback activation for one render-session PID.
- [ ] Add render-session enumeration with process labels from display name or executable stem.
- [ ] Change the app service audio setup so output tracks prefer per-process `AudioSource`s, with
      mixed output as fallback and microphone as a separate source.
- [ ] Add service tests that sidecar writing emits audio track metadata even when a clip has no
      markers.
- [ ] Add `clipline-mp4` selected-audio remux tests: all selected is a no-op-equivalent path,
      one selected drops the other audio track, none selected emits video-only.
- [ ] Add `upload_clip_to_cloud` selection plumbing and tests that subset uploads checksum/remux the
      selected bytes while the persisted upload record still points at the original local clip.
- [ ] Add review/upload UI contract tests for the track list and upload dialog checkboxes.
- [ ] Verify with `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
