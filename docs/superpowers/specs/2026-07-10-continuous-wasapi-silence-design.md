# Continuous WASAPI Silence Design

## Problem

Clipline anchors each WASAPI source to the shared capture clock at `0.0`, but an empty anchor does not add PCM. `LoopbackAssembler` currently fills a gap only when a later device buffer reveals that time elapsed. If an output endpoint or process is completely inactive, WASAPI can return no buffers at all. The recorder can therefore seal a 0.5-second video GOP with no audio samples.

The hardware sync failure reports approximately `-0.5s` because the validator measures audio coverage minus video duration. It is one empty GOP, not disagreement between the WGC and WASAPI clocks. The same behavior can affect real recordings that contain a fully quiet interval.

## Goals

- Keep every enabled audio track continuous through quiet endpoint intervals.
- Preserve the shared QPC-derived timeline used by video and audio.
- Allow one 20 ms Opus frame for normal WASAPI delivery latency.
- Prevent late device buffers from duplicating or shifting time already represented by synthesized silence.
- Cover the behavior with deterministic, platform-neutral tests before relying on the Windows hardware test.

## Non-goals

- Do not relax A/V validation tolerances or skip empty audio tracks.
- Do not add a synthetic render client merely to keep an endpoint active.
- Do not make the hardware test play a tone; it must remain valid on a quiet desktop.
- Do not change the shared clock, GOP duration, Opus frame size, or MP4 layout.

## Design

### Neutral timeline advancement

`LoopbackAssembler` will gain an operation that advances its represented PCM timeline to a finite target PTS by appending stereo zero samples. The target is an absolute point on the existing shared timeline, not a duration.

Advancement is monotonic:

- A target at or behind the assembler's current end does nothing.
- A non-finite target does nothing. WASAPI also recognizes the existing `f64::MAX` monitor-drain sentinel and never passes it to timeline advancement.
- The existing maximum gap-fill bound remains the protection against corrupt timestamps or unreasonable single-call allocation.
- Generated samples use the same 48 kHz stereo grid as real and gap-filled PCM, so `pop_frame` continues to emit exact 20 ms frames.

### Overlap handling for late buffers

After silence is synthesized, a real WASAPI buffer may arrive with a timestamp before the assembler's current end. `push_chunk` will compare the buffer interval with the current end:

- Small timestamp jitter within the existing tolerance remains contiguous behavior.
- For a material overlap, the already-represented prefix is removed from the incoming interleaved stereo samples.
- If the incoming buffer is entirely behind the current end, it is discarded.
- Any remaining suffix begins at the current end and is appended normally.

This preserves a single monotonic PCM timeline. It intentionally prefers silence already committed to the recorder over retroactively replacing sealed audio.

### WASAPI polling horizon

After draining all currently available device buffers, `WasapiPcmCapture::poll_frames(until_pts_s)` will advance the assembler through `until_pts_s - 0.020` seconds when `until_pts_s` is finite and is not the `f64::MAX` monitor-drain sentinel. The 20 ms allowance gives the audio engine one Opus frame to deliver real samples before silence is synthesized.

The method then pops complete frames and retains the existing rule that only frames ending at or before the requested horizon are returned. Calls using `f64::MAX`, such as live level monitoring, drain only real buffered data and do not synthesize silence.

This behavior applies uniformly to system output, per-process output, microphone, and mixed sources because they all use `WasapiPcmCapture`. Mixed sources therefore receive a continuous frame grid even when one input is inactive.

## Data flow

1. The recorder requests audio packets through the current video frame PTS.
2. WASAPI drains every available device buffer into `LoopbackAssembler`.
3. For a finite request, the assembler advances with silence to one Opus frame before the request.
4. Complete PCM frames are encoded to Opus and returned to the recorder.
5. A later real device buffer that overlaps synthesized time has its overlapping prefix trimmed before its remaining samples are appended.
6. At the next video keyframe, the GOP has audio coverage within the existing 45 ms tolerance and can be sealed safely.

## Error handling and bounds

- No new recoverable error type is required; timeline advancement is deterministic in-memory work.
- Non-finite poll horizons and the `f64::MAX` monitor-drain sentinel never synthesize samples.
- Silence generation continues to use the existing five-second maximum single-gap bound.
- Timestamp discontinuity logging remains unchanged.
- Device-loss and Opus-encoding errors continue to propagate through the existing `CaptureError` path.

## Tests

Platform-neutral tests in `pcm.rs` will establish the core contract:

- An assembler anchored at zero and advanced to 0.5 seconds emits 25 consecutive silent 20 ms frames at PTS values `0.00` through `0.48`.
- Advancing to the same or an earlier target is idempotent.
- A late real chunk partially overlapping synthesized silence has only its uncovered suffix appended.
- A real chunk entirely covered by synthesized silence is discarded without extending the timeline.
- A non-finite target does not allocate or emit silence.

Windows-side coverage in `wasapi.rs` will verify the finite polling allowance without requiring hardware. A pure `audio_poll_silence_horizon(until_pts_s: f64) -> Option<f64>` helper will return `Some(max(until_pts_s - 0.020, 0.0))` for ordinary finite inputs and `None` for non-finite inputs or `f64::MAX`. The existing `real_engines_on_one_clock_produce_a_synced_timeline` hardware test remains unchanged and must pass on an idle desktop.

The final verification remains:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

## Files

- `crates/clipline-capture/src/pcm.rs`: neutral advancement and overlap behavior plus deterministic tests.
- `crates/clipline-capture/src/windows/wasapi.rs`: apply the finite poll horizon with a 20 ms delivery allowance through the tested `audio_poll_silence_horizon` helper.
- `crates/clipline-capture/src/windows/wgc.rs`: no production change expected; its existing hardware sync test is the integration proof.
