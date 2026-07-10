# Resilient Seeking and Lazy Audio Preview Design

**Date:** 2026-07-10

## Goal

Prevent rapid relative seeking from permanently resetting playback to the beginning, including
while an audio-preview source swap is loading. Remove the automatic whole-file audio-preview work
that makes this race easier to trigger and causes severe playback stuttering on large multi-track
clips.

This design supersedes the narrow latest-position audio-preview source-swap design in
`2026-07-09-reliability-and-playback-fixes-design.md`.

## Evidence and Root Cause

The persistent reset was reproduced by rapidly pressing the five-second forward shortcut. During
the reproduction, the player became severely sluggish and later recovered without an app restart.
The audio-preview cache received several large files during the sluggish period and reached 12.1
GiB. The preview path currently reads an entire source file into memory, constructs an entire MP4
preview in memory, and writes that preview to disk. The frontend ignores stale asynchronous
results, but it does not stop their backend work.

The playback fix merged in PR #83 preserves only the latest pending seek. That value stops being
authoritative once it is assigned to `video.currentTime`. A source load or an earlier `seeked`
event can then interleave with a later seek, consume or clear the pending value, and leave no target
for the new source's `loadedmetadata` path to apply. The metadata path may also skip restoration
because a later seek changed the seek revision, even when that later seek never landed. The media
element remains at zero because the requested position has been lost rather than merely painted
incorrectly.

Browser media events do not identify the source generation that emitted them. A generation counter
can reject stale asynchronous callbacks, but it cannot by itself prove that a generic `seeked`
event belongs to the current source. Confirmation must also require current-source metadata to be
ready and the media element to have arrived at the requested time.

## Constraints

- Keep player state transitions and time calculations in the DOM-free `player-core.js` layer.
- Preserve keyboard, button, scrubber, marker, trim-range, playback-rate, and play/pause behavior.
- Do not change recording, export, or source media files.
- Do not add a frontend framework, database, background service, or new media format.
- Do not build a streaming or cancellable mixer in this change. Preview jobs will instead become
  lazy, serialized, and coalesced.
- Preserve the user's existing uncommitted shortcut and GSI-spike work.
- Add failing regression tests before implementation.

## Design

### 1. Authoritative logical seek target

The review player will maintain a logical seek state separate from `video.currentTime`. Its relevant
fields are:

- the latest finite requested time, or no active target;
- the current source generation;
- the generation whose metadata is ready;
- whether the target has been applied to the current source.

Every seek entry point records a clamped logical target before interacting with the media element.
Relative seeks use the active logical target as their base; when no target is active, they use the
current finite media time. Five rapid five-second seeks therefore accumulate to one target twenty-
five seconds ahead, subject to the existing playback bounds.

When current-source metadata is ready, the integration layer assigns the latest target to
`video.currentTime`. If metadata is not ready, the target remains stored and is applied by that
generation's valid `loadedmetadata` callback. A new request replaces the earlier target but never
removes it merely because an assignment was attempted.

A `seeked` event confirms a target only when all of the following are true:

1. metadata is ready for the current source generation;
2. `video.currentTime` is finite;
3. `video.currentTime` is within 100 milliseconds of the latest clamped target.

An event that fails these checks cannot clear the target. When it represents completion of an
earlier seek, the current target is applied again. This time-and-readiness gate also makes a stale
source event at zero harmless even though the browser event carries no generation identifier.

The seek target is cleared only after confirmed arrival or when the user closes the clip. Normal
playback resumes using `video.currentTime` once no target is active.

### 2. Source assignment and timeline rendering

Every source assignment increments the source generation, marks current metadata as unavailable,
and captures a resume target from the active logical target or the current finite media time. It
does not discard a logical target.

Only callbacks created for the current source generation may mark metadata ready, apply a target,
restore playback state, or report a source-load failure. The current generation's metadata callback
always applies the latest logical target after clamping it to the newly known duration. It does not
skip restoration solely because the seek revision changed; that revision means a newer target must
win, not that the target already landed.

While a target is active, timeline painting, overview painting, time labels, and subsequent relative
seek calculations use the logical target. This prevents a temporary source-load time of zero from
moving the visible playhead backward. After confirmed arrival, those consumers return to the media
element's current time.

Playback rate, trim range, and play/pause intent continue through source swaps using the existing
restoration behavior. The seek coordinator changes only position ownership and confirmation.

### 3. Immediate original-source playback

Opening or switching to a clip assigns its original media source immediately and never starts audio
preview generation. For a multi-track file, the player uses the file's directly playable fallback
audio track. The audio controls show that fallback as the active default; microphone and process
tracks are not shown as audible until the user explicitly selects a mix that includes them.

Returning to the fallback-only selection restores the original media source without generating a
preview. Selecting a different track or a multi-track combination requests a preview because
WebView2 cannot directly switch among the embedded audio tracks through the current player API.

### 4. Serialized and coalesced preview requests

An explicit audio-selection change records the newest desired request, identified by clip identity,
ordered track IDs, and selection revision. The currently playing source remains active while the
preview is prepared. The request-queue transition rules will be pure `player-core.js` logic so
serialization and coalescing can be tested independently from Tauri commands.

At most one preview build is submitted by the review UI at a time. If the user changes the
selection or clip while a build is running, the UI replaces the queued desired request instead of
submitting concurrent work. When the active build finishes:

- its result may replace the source only if clip identity, track IDs, selection revision, and source
  generation captured by the request still match the latest desired request;
- a stale successful result may remain cached but cannot affect playback;
- if a newer preview request is still desired, exactly that newest request starts next;
- if the newest selection is the directly playable fallback, no new preview starts.

The swap to a valid preview goes through the same source-assignment and logical-seek path described
above. Its completion cannot restore the request-start time over later user seeks.

Serialization does not make an already-running native job cancellable, but it prevents the current
fan-out of multiple simultaneous whole-file reads and writes. Removing automatic preview generation
means ordinary clip opening and browsing start no such work.

### 5. Preview cache lifecycle

Reusable audio previews have a 2 GiB least-recently-used byte limit. Cache cleanup runs during app
startup and after a preview is completed. A cache hit refreshes the file's recency. The frontend
passes the currently loaded preview path, when any, as a protected path to cleanup. Cleanup removes
oldest unprotected reusable entries until the retained cache is within the limit.

The preview currently loaded by the player is never deleted. If one selected preview alone exceeds
2 GiB, it is treated as a transient active preview: it may temporarily exceed the reusable-cache
limit, remains present while loaded, and becomes eligible for deletion after the player leaves it.
The limit therefore bounds retained reusable data without breaking playback of an explicitly
requested large mix.

Preview output is written to a unique partial path and atomically renamed to its content-addressed
cache path only after successful completion. Failed jobs remove their partial output. Startup
cleanup removes abandoned partial files as well as enough old previews to bring the existing cache
under the limit. Source clips are outside this cache and are never candidates for deletion.

### 6. Error behavior

Invalid or non-finite seek requests are ignored without replacing a valid active target. Duration
changes clamp the latest target to the same playback bounds used by normal seeking.

A preview-generation failure leaves the current source, position, play/pause intent, and playback
rate intact. The audio controls revert to the selection that is actually audible, and the UI
reports a concise nonfatal audio-preview error. A source load failure is handled only by the current
source generation; failures from superseded assignments are ignored.

Cache cleanup is best effort. Failure to evict an old preview is logged but does not prevent opening
a clip or using a successfully generated preview.

## Testing

### Pure player-state tests

The Rust-hosted `player-core.js` suite will model state transitions without relying on browser event
timing. At minimum it will cover:

- five rapid relative seeks accumulating from the logical target;
- a source assignment temporarily reporting zero without changing the logical target;
- an old or early `seeked` event at zero failing to confirm a later target;
- a `seeked` event for an earlier rapid seek reapplying the newest target;
- current-source metadata applying the newest target even after the seek revision changes;
- confirmation clearing the target only within tolerance;
- clamping after a new duration becomes known;
- timeline time selection preferring the logical target until confirmation.

The central regression sequence is: request several rapid forward seeks, start a source swap,
observe zero, deliver an early `seeked`, deliver current-source metadata, and then confirm that the
final accumulated target is applied and survives until arrival.

### UI contract tests

UI tests will verify that:

- opening a multi-track clip assigns the original source without invoking preview generation;
- the fallback audio selection matches what is immediately audible;
- explicit non-fallback selection starts a preview while playback remains on the current source;
- repeated selections create one active build and retain only the newest queued request;
- stale completion, stale source callbacks, and preview errors do not replace working playback;
- timeline rendering consumes logical time during an outstanding seek;
- a valid preview swap uses the common source-assignment and seek-coordinator path.

### Cache tests

Rust tests will inject a small byte limit and small fixture files to prove LRU eviction, recency
refresh, protected-path preservation, oversized-active-preview handling, partial-file cleanup, and
atomic publication. No test requires a large media fixture.

### Manual acceptance

Using a large multi-track clip:

1. Open and switch among clips; verify playback starts immediately and no preview job or cache file
   is created automatically.
2. Repeatedly spam the right-arrow shortcut and five-second forward button across multiple timeline
   events, including during clip and source changes.
3. Verify the visible playhead and actual playback position never move to zero unless zero was
   explicitly requested.
4. Explicitly select microphone or process audio, verify the old source keeps playing while the
   preview builds, and verify a completed swap preserves the latest requested position.
5. Rapidly change audio selections and verify preview work remains serialized and only the newest
   selection becomes audible.
6. Restart the app and verify preview cache usage is reduced to the configured retained limit,
   except for any single currently active oversized preview.

## Verification Gates

Implementation follows plan-driven TDD. Each behavior begins with a focused failing test, followed
by the smallest production change that makes it pass. Final verification is:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- fresh clippy for materially changed Rust crates where a warm cache could hide warnings
- launch `cargo run -p clipline-app` after stopping the existing process
- complete the manual acceptance sequence above

## Out of Scope

- Streaming or incremental native preview mixing
- Cancellation of a preview job already executing in `spawn_blocking`
- Changes to recording or exported audio composition
- Changes to marker-navigation semantics
- Deleting or modifying source clips
