# Fast Audio Sidecar Switching Design

**Date:** 2026-07-15

## Goal

Make an uncached audio-track switch on a long clip become audible in approximately 0.5 to 2
seconds, with cached switches becoming nearly immediate. Audio switching must not reload the video,
move the playhead, interrupt seeking, or reserialize the source clip's video samples.

This design supersedes the full-file audio-preview source-swap mechanism described in
`2026-07-10-resilient-seeking-lazy-audio-preview-design.md`. Its logical-seek ownership,
serialized latest-request queue, failure behavior, and bounded cache remain valid; only the media
artifact and browser playback architecture change.

## Evidence and Root Cause

The current preview command reads an entire source file into memory, clones every retained video
sample into a new fragment, muxes another complete MP4 in memory, writes that MP4 to disk, and then
reloads it in the review video element. Changing only the audible tracks therefore performs work
proportional to the video's bytes and duration.

For the reproduced 31-minute, 1.88 GiB clip, one selection can involve approximately 3.8 GiB of
disk traffic and several GiB of simultaneous buffers. Each cached preview is about 1.9 GiB, so a
2 GiB cache holds roughly one combination and repeatedly evicts useful work. The subsequent
WebView2 source reload adds another large-file load and creates the observed stutter.

Measurements on that clip with the packaged FFmpeg establish the useful boundary:

- copying one embedded Opus track into an audio-only MP4 took 1.87 seconds and produced a 23.9 MB
  file;
- copying two tracks in one FFmpeg invocation took 0.50 seconds and produced about 47.7 MB total;
- decoding, mixing, and re-encoding two tracks into one audio-only file took 15.0 seconds;
- WebView2 reports native playback and Media Source support for Opus in MP4, but its media element
  exposes no usable `audioTracks` API for switching embedded tracks directly.

The selected approach must therefore copy individual audio tracks without touching video and let
the browser mix selected sidecars during playback.

## Constraints

- Keep the original video source assigned throughout audio switching.
- Preserve the authoritative logical seek target and all existing keyboard, button, scrubber,
  marker, trim-range, playback-rate, and play/pause behavior.
- Keep player decisions DOM-free and testable in `player-core.js`; browser integration remains in
  the existing UI modules.
- Keep at most one native extraction request active from the review UI and coalesce pending work to
  the newest desired selection.
- Reuse the existing external FFmpeg process. Do not link FFmpeg or add GPL dependencies.
- Never modify source clips or export behavior.
- Keep cache publication atomic and cache deletion best effort.
- Add failing regression tests before each production change.

## Alternatives Considered

### One pre-mixed audio-only sidecar

This would require only one browser audio element, but the measured decode/mix/encode step took 15
seconds on the long clip. It cannot meet the accepted uncached-switch target.

### Streaming or Media Source full previews

Progressive full-preview muxing could make initial playback begin before completion, but it still
moves all video bytes and introduces substantial WebView2 buffering, seeking, and cancellation
complexity. A custom Media Source player would also replace stable native media-element behavior.

### File-to-file streaming remux

Removing the in-memory copies would reduce memory pressure, but a full preview would still read and
write almost 4 GiB for the measured clip and reload the video. It improves the current mechanism
without solving its fundamental latency.

## Design

### 1. Per-track audio sidecars

The native preview command will return an ordered list of selected track IDs and audio-only media
paths. Each reusable sidecar contains exactly one embedded source audio track, copied without
decoding or re-encoding. It contains no video track.

The cache identity includes the canonical source identity and metadata used by the existing cache,
the embedded track ID, and a new sidecar format version. A track has one reusable artifact
regardless of which later selection combines it with other tracks. Exact fallback-only playback
continues using the original video and needs no sidecar.

For a request containing uncached tracks, one FFmpeg process reads the source once and creates one
output per missing track. The command maps each requested audio stream explicitly, disables video,
copies the audio codec, strips irrelevant metadata, and writes each output to a unique sibling
temporary path. Existing sidecars are cache hits and are omitted from the FFmpeg outputs.

All new outputs form one publication unit. If FFmpeg fails, produces a missing output, or any output
fails validation, every temporary output from that invocation is removed and no new path is
returned. After successful validation, each temporary file is atomically renamed to its
content-addressed cache path. A rename collision with a valid file produced elsewhere accepts the
existing winner and removes the redundant temporary file.

### 2. Original video remains the transport

The review `<video>` element remains assigned to `clip.path` when audio selections change. The
player no longer swaps the video source to an audio preview. Video decode state, buffered ranges,
logical seek ownership, and the visible playhead therefore survive an audio switch unchanged.

Explicit selections that require sidecars retain the currently audible selection while extraction
and media loading run. Once every sidecar for the latest valid request is ready, the browser seeks
them to the current authoritative video time, applies the current playback rate, starts them when
the video is playing, and makes the transition audible. Only then is the original video's audio
silenced.

Each selected track receives one hidden audio element. Playing the selected elements together
preserves the existing unnormalized additive mix behavior without a native re-encode. Returning to
the exact directly playable fallback destroys or detaches the sidecar elements and restores the
original video's audio. Selecting no tracks detaches sidecars and leaves the original video
silenced.

The player's logical audible state is kept separately from the transport-level muting needed to
silence the video's embedded audio in sidecar mode. User-visible mute and volume changes, if
present, apply to whichever media elements currently provide sound rather than accidentally
reactivating the original audio.

### 3. Video-authoritative synchronization

The video element is the sole authoritative clock. Sidecar elements never update the logical
playhead and never initiate source or timeline changes.

Browser integration mirrors these video transitions to every active sidecar:

- `play` starts ready sidecars at the current video time;
- `pause` pauses all sidecars;
- `seeking` and `seeked` move all sidecars to the latest authoritative video time;
- playback-rate changes set the same rate on every sidecar;
- clip close, clip replacement, suspend, release, or rename detaches all sidecars and invalidates
  their pending callbacks.

A pure `player-core.js` synchronization decision compares finite video and sidecar state. It asks
the integration layer to reseek only when drift exceeds 100 milliseconds, or when an explicit seek
or activation requires alignment. Smaller differences are left alone to avoid repeated corrections
and audible glitches. The integration layer evaluates this decision on transport events and a
bounded periodic check while playing.

Sidecar activation is generation-gated. Readiness, play promises, and error callbacks from an old
clip or selection cannot alter current audio state.

### 4. Serialized latest-selection orchestration

The existing pure audio-preview queue remains the ownership mechanism. It continues to provide one
active native request, one coalesced latest desired request, monotonically increasing revisions,
and an apply result only for the latest successful request.

The queued request records clip identity, ordered selected track IDs, selection key, source
generation, and the complete set of currently audible sidecar paths that cache pruning must
protect. A successful native response may activate only when all of those current-state gates still
match. A stale successful extraction may populate the cache but cannot change playback.

Fallback or muted selections cancel the desired request without attempting to cancel an FFmpeg
process already running. Rapid changes retain only the newest selection. After any await, activation
reads the current logical video time, play/pause state, playback rate, and generation rather than
restoring values captured when extraction began.

### 5. Cache lifecycle and protection

Sidecars use a new cache version and retain the existing `audio-preview-*.mp4` family so startup and
LRU cleanup recognize them. Old full-video preview files are never reused by the new key and become
ordinary eviction candidates.

Cache pruning continues to cap total preview bytes, order reusable files by recency, remove
abandoned partials, and leave source files untouched. Protection changes from one active preview
path to a set of active sidecar paths. Every currently audible sidecar counts toward the total cap
but is never deleted. Newly generated sidecars are also protected during the request's
post-generation prune so the command cannot evict the paths it is about to return.

A cache hit refreshes each requested sidecar's recency. Because combinations reuse per-track files,
switching between previously encountered selections neither duplicates video data nor creates a new
artifact for every combination.

### 6. Error and transition behavior

An extraction, validation, media-load, or sidecar-play failure leaves the current audible source,
position, play/pause state, rate, and video source unchanged. The checkboxes return to the selection
that is actually audible and the deck shows a concise nonfatal error.

The transition to sidecar audio is all-or-nothing from the user's perspective. Original audio is
not silenced until every required sidecar is ready for the latest selection. A partial set never
becomes audible. Superseded elements and their listeners are detached without affecting the active
set.

Cache pruning and recency-touch failures remain best effort and are logged without blocking
playback. Atomic temporary-file cleanup covers process failure, validation failure, publication
failure, and application shutdown recovery through the existing partial sweep.

## Testing

### Native extraction and cache tests

Tests use an injected extractor or small fixture and begin red. They verify:

- selected track IDs map to ordered audio-only paths;
- multiple missing tracks are requested in one extraction invocation;
- cached tracks are reused while only missing tracks are extracted;
- cache identity is per source and track rather than per selection combination;
- every output contains audio and no video track;
- successful publication is atomic and failures remove all sibling partials;
- active and newly returned path sets remain protected during pruning;
- old full-preview cache entries are not reused.

### Pure player-state tests

DOM-free tests cover the sidecar synchronization decision and orchestration inputs:

- activation mirrors current time, rate, and play/pause state;
- drift at or below 100 milliseconds produces no seek;
- drift above 100 milliseconds requests the authoritative video time;
- non-finite media values cannot produce an invalid assignment;
- explicit seeks require alignment even within normal drift tolerance;
- the existing queue still applies only the latest selection after rapid changes and cancellation.

### UI contract tests

Structural and behavior-focused tests verify that:

- audio switching never assigns a preview path to the video element;
- the original clip remains loaded while extraction runs;
- all sidecars are ready before original audio is silenced;
- play, pause, seek, rate, close, suspend, and clip replacement synchronize or detach sidecars;
- stale native responses and media callbacks cannot activate;
- fallback and muted selections clear sidecar playback correctly;
- extraction failure restores checkbox state without changing the video source.

### Manual acceptance

Using the reproduced 31-minute multi-track clip:

1. Make a first uncached one-track and multi-track selection; each should become audible in the
   accepted 0.5-to-2-second range on the measured machine.
2. Repeat those selections and verify cached switches are nearly immediate.
3. Seek repeatedly and spam the right-arrow shortcut while extraction is running; verify the
   playhead never returns to zero and video does not reload or stutter.
4. Change selections rapidly and verify only the newest selection becomes audible.
5. While sidecars are active, exercise play, pause, scrubbing, playback-rate changes, clip changes,
   fallback, and muted mode; verify audio follows the video without persistent drift or duplication.
6. Force an extraction or media-load failure and verify the previously audible selection continues.
7. Restart the app and verify total preview-cache bytes remain within the existing policy except
   for protected active files.

## Verification Gates

Implementation follows plan-driven TDD, with each behavior receiving a focused failing test before
production code. Final verification requires:

- `cargo test --workspace`
- `cargo clean -p clipline-app` followed by
  `cargo clippy -p clipline-app --all-targets -- -D warnings`
- `cargo clippy --workspace --all-targets -- -D warnings`
- launch `cargo run -p clipline-app` after stopping the existing worktree app
- complete the manual acceptance sequence above

## Out of Scope

- Re-encoding or pre-mixing selected tracks
- Pre-extracting every track automatically when a clip opens
- A custom Media Source video player
- Cancellation of an FFmpeg process already executing
- Recording, capture, export, or source-file format changes
- Deleting or modifying source clips
