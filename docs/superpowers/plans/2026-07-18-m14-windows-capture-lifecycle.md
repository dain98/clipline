# M-14 Windows Capture Lifecycle Plan

**Goal:** Observe the configured WASAPI/WGC lifecycle contracts so process audio is drained under a supported buffering model and closing a captured target ends recording instead of replaying its last frame forever.

## Process loopback buffering contract

- [ ] Add a deterministic configuration regression proving process loopback uses pull-mode flags and a one-second shared buffer rather than registering an event nobody waits on.
- [ ] Remove the unused event handle lifecycle and initialize process loopback as a polling capture client with the same headroom used by endpoint loopback.
- [ ] Keep per-video-cadence draining and the existing bounded PCM discontinuity handling; exercise process-loopback construction/poll/drop on a real Windows session when available.

## WGC target closure

- [ ] Add queue regressions proving an explicit close wakes a blocked receiver, wins over retained sender clones, discards stale queued textures, and rejects later frame callbacks.
- [ ] Register `GraphicsCaptureItem.Closed`, retain both WinRT event tokens, and mark the bounded frame channel closed from the callback.
- [ ] Revoke `Closed` and `FrameArrived` handlers during deterministic teardown before closing the capture objects.
- [ ] Propagate a closed channel as `Ok(None)` so `CadencedCapture` ends the recording rather than treating closure as an idle timeout and duplicating the last texture.

## Verification and handoff

- [ ] Run focused capture/app tests, fresh-cache Clippy for changed crates, CI-mode workspace tests, and workspace Clippy with warnings denied.
- [ ] Rebuild and open Clipline, verify normal startup, and record finding/commit evidence in the master ledger and `handoff.md`.
- [ ] Add manual tests for a real per-process audio source and closing a live captured window, because both depend on Windows device/session events that deterministic queue/configuration tests cannot generate.
