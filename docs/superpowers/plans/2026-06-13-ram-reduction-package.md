# Clipline RAM Reduction Package

## Goal

Reduce Clipline's real and reported memory footprint while preserving the current recorder behavior:

- show resident private RAM instead of process-tree working set,
- size the in-memory replay ring to the configured replay window plus headroom,
- allow the WebView2 UI to be destroyed while the recorder keeps running,
- add an explicitly acknowledged advanced disk replay buffer,
- bound WGC queued frame textures and avoid full-monitor copies for display-region capture,
- stream trim exports instead of holding source and output MP4s in memory.

## Constraints

- Disk replay storage is off by default.
- Disk replay storage must warn about continuous writes and SSD wear.
- Disk replay storage must require a folder, quota, and acknowledgement.
- Disk replay storage must stop on cache write or low-disk failure instead of falling back to RAM.
- The replay cache folder must be separate from the saved media folder.
- Minimize keeps the current behavior; close destroys the WebView and keeps the recorder alive.

## Implementation Steps

- [ ] Add resident-private process-tree memory accounting and update the UI field.
- [ ] Normalize replay buffer duration to `replay_window_s + 15s` and lower the RAM estimate floor.
- [ ] Add replay storage settings, validation, persistence, and Storage-tab controls.
- [ ] Add a disk-backed replay ring that stores GOP payloads in per-run files.
- [ ] Wire disk replay storage into the recorder service and surface storage errors.
- [ ] Bound the WGC frame queue to two frames and drop the oldest frame when overloaded.
- [ ] Copy only the selected display-region texture before queueing when region capture is active.
- [ ] Add streaming MP4 trim/export APIs and use them from the Tauri export command.
- [ ] Destroy the main WebView on close, add tray Open Clipline, and recreate the window on demand.
- [ ] Verify with workspace tests, clippy, JS syntax check, build, and live app launch.

## Acceptance Tests

- Default settings keep replay storage in memory and do not require acknowledgement.
- Existing settings migrate to `buffer_seconds = replay_window_s + 15`.
- Disk mode validation rejects missing folder, missing acknowledgement, invalid quota, or cache/media folder overlap.
- Disk replay cache writes bounded segment files and deletes evicted files.
- RAM and disk replay rings produce equivalent saved replay MP4s.
- Low disk or cache write failures are surfaced as recorder errors.
- Close destroys WebView2 children while hotkey save keeps working.
- Tray Open Clipline recreates the UI and shows current recorder state.
- Display-region capture queues region-sized textures.
- Trim/export output and marker sidecar cropping remain correct.
