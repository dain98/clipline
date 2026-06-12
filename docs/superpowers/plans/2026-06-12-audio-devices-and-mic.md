# Clipline Audio Devices + Mic Track Implementation Plan

**Goal:** Implement CODEX_NOTES #1 and #2: record microphone audio alongside system audio, and
add Settings > Capture controls for audio output/input device selection, output/input volume, mic
Mono handling, and a direct mic test.

**Shape:** Keep neutral settings and UI testable on CI. Keep Windows device work behind
`clipline-capture::windows::wasapi`, reusing the existing `AudioSource` path. When output and mic
are both enabled, mix them into one normal Opus track so regular playback hears both.

## Tasks

- [x] Add persisted audio settings with legacy defaults:
  - output enabled/default device/100% volume
  - mic disabled/default device/100% volume
  - mic channel mode mono or stereo
- [x] Add Settings > Capture audio controls and structural UI tests.
- [x] Add device enumeration Tauri command for render/capture endpoints.
- [x] Generalize WASAPI source creation:
  - default or selected endpoint by device id
  - render loopback for system audio
  - capture endpoint for microphone audio
  - volume gain clamp
  - mic mono/stereo transform before Opus
- [x] Add common PCM decode and resampling so non-48 kHz mic endpoints still feed Opus correctly.
- [x] Add Test mic command/UI with selected-device live playback and a level meter.
- [x] Wire service startup to attach output-only, mic-only, or mixed output+mic audio.
- [x] Verify with focused tests, workspace tests, clippy, and a live app launch.

## Manual Test Checklist

- Open Settings > Capture and verify Audio output and Microphone sections render.
- Confirm output/input device menus populate and include a Default option.
- Toggle microphone on, check/uncheck Mono, adjust volumes, save, and confirm settings persist.
- Click Test mic while speaking and confirm the button toggles to Stop testing, mic playback is
  audible, and the meter/status responds until stopped.
- Save a replay with output and mic enabled, then play it in the library to confirm both desktop
  audio and mic are audible in the normal clip audio.
