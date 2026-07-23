# WASAPI Device-Loss Recovery

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking and remain unticked by repository convention.

**Goal:** A mid-recording WASAPI endpoint invalidation (`AUDCLNT_E_DEVICE_INVALIDATED`
0x88890004, `AUDCLNT_E_SERVICE_NOT_RUNNING` 0x88890010, `AUDCLNT_E_RESOURCES_INVALIDATED`
0x88890026) must no longer abort the recorder. The affected audio track fills the outage with
timeline-aligned silence and re-activates the endpoint when it returns; video and other tracks
are unaffected.

**Observed failure:** `recording: capture device lost: WASAPI: 0x88890004; additionally, finish:
capture device lost: WASAPI: 0x88890004`. The first error aborts `rec.step_with_frame`; the
second fires when shutdown drains the same dead client. Common trigger: default render endpoint
re-enumeration (headphone/USB/Bluetooth disconnect, monitor audio power-cycle, default-device
switch). Once an `IAudioClient` returns DEVICE_INVALIDATED it is permanently dead — only
re-enumeration and re-activation recovers it.

**Why silence works for free:** the existing idle-desktop machinery already advances the
assembler with capped silence once host-observed delivery idleness exceeds the quiet grace, and
`DevicePacketTimeline::note_synthesized_silence` forces the next live packet to establish a
fresh QPC anchor. A dead device looks exactly like an indefinitely idle one, so A/V sync across
the outage is preserved without new timeline code.

## Task 1: Failing neutral tests

- [ ] Add `DeviceReactivation` state-machine tests in `pcm.rs`: first failure schedules a retry
      after the configured interval without resetting the outage start; repeated failures keep
      the original outage start; `note_recovered` reports the outage and returns to live.
- [ ] Add `wasapi_error_recoverable` classification tests in `wasapi.rs` (Windows-only):
      0x88890004/0x88890010/0x88890026 recoverable, `E_FAIL` and other codes fatal.
- [ ] Add `WasapiDeviceLost` / `WasapiDeviceRecovered` display tests in `diagnostics.rs`
      matching the existing structured `key=value` style with an `action=` suffix.
- [ ] Run the focused tests and confirm they fail to compile/pass on the current code.

## Task 2: Factor endpoint activation

- [ ] Introduce `EndpointTarget` (`OutputLoopback { device_id }`, `ProcessOutput { pid }`,
      `Microphone { device_id, channels }`) owning every parameter needed to re-create the
      client (stream flags, buffer duration, fixed mix format for process loopback).
- [ ] Split `start_client` into `EndpointTarget::activate` (COM chain: enumerate/activate,
      initialize, get service, start) returning an `ActivatedDevice { client, capture, mix }`,
      and a constructor that assembles `WasapiPcmCapture` around it.
- [ ] Keep startup behavior byte-identical: initial activation failure remains a fatal
      `CaptureError::Init` (a device missing at recording start must stay loud).

## Task 3: Wire recovery into polling

- [ ] Store `target: EndpointTarget`, `reactivation: DeviceReactivation`, and a device-loss
      `DiagnosticRateLimiter` on `WasapiPcmCapture`.
- [ ] `drain_device`: classify COM errors — recoverable HRESULTs mark the device dead, emit a
      rate-limited `WasapiDeviceLost` diagnostic, and return `Ok(())` so the silence-fill path
      runs; contract violations (null buffer, sample overflow, decode failure) stay fatal.
- [ ] `collect_frames`: before draining, retry activation when due (1 s cadence). Process-output
      targets skip the attempt while the pid is dead. Success swaps the client/capture/mix in
      place, restarts the discontinuity fade, requires a fresh timestamp anchor, resets the
      resampler for the new mix format, and emits `WasapiDeviceRecovered`.
- [ ] `finish_frames` inherits the same path, so shutdown never errors on a dead endpoint.
- [ ] Keep `DeviceLost` fatal for non-recoverable HRESULTs and for the DXGI/video path.

## Task 4: Verify and hand off

- [ ] Focused capture tests, then the full workspace suite, green.
- [ ] Fresh-cache warning-denied Clippy for `clipline-capture`, then the workspace.
- [ ] Manual smoke: start recording, disable/enable the default output device in Sound settings
      mid-recording, confirm the recorder survives, the log shows `wasapi_device_lost` then
      `wasapi_device_recovered`, and the saved clip plays with a silence gap then live audio.
- [ ] Update `handoff.md`; commit as one conventional change.
