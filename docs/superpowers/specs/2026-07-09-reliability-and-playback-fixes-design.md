# Reliability and Playback Fixes Design

**Date:** 2026-07-09

## Goal

Fix seven verified correctness defects without broad architectural rewrites:

1. Preserve a finalized full-session recording when its final rename fails.
2. Prevent late settings-save failures from disconnecting the active recorder.
3. Prevent stale cloud-library requests from surviving disconnects or account changes.
4. Keep pending osu! enrichment attached to a clip when its MP4 is renamed.
5. Treat clip-title metadata as a first-class sidecar during quota accounting and deletion.
6. Write valid MP4 duration metadata for recordings whose media duration exceeds 32 bits.
7. Preserve the latest playback position when an asynchronous audio-preview swap completes.

The marker-navigation wrap behavior is deliberately out of scope. The reported playback reset was
reproduced with the five-second forward control and does not travel through marker navigation.

## Constraints

- Preserve the current uncommitted review-player shortcut changes.
- Use failing tests before implementation for every defect.
- Keep neutral logic cross-platform and Windows-specific behavior behind existing platform
  boundaries.
- Do not introduce a new media format, database, frontend framework, or background service.
- Keep each fix independently reviewable and commit it as one logical change.

## Design

### 1. Recoverable full-session finalization

`HybridMp4Writer` is intentionally crash-safe while recording, and startup recovery already
recognizes non-empty `*.mp4.recording` files. A successful writer finalization followed by a failed
filesystem rename must therefore leave the temporary recording in place. The service will emit a
clear warning containing the recoverable path and will not delete the file.

The same preservation rule applies when the full-session writer reports a finalization error after
having written footage: retain the non-empty recording for startup recovery. Empty files and the
existing deliberately discarded short osu! startup transients may still be deleted.

Tests will prove that rename/finalization failures preserve non-empty temporary recordings and that
the existing empty/short-session cleanup remains intact.

### 2. Transactional recorder restart during settings save

Settings restart preparation will be split into two phases:

- **Plan:** validate and build the prospective `ServiceOptions` without mutating runtime state or
  removing the active command sender.
- **Commit:** after all fallible settings persistence, tray-label, and hotkey operations succeed,
  atomically install the new settings and take the old sender for restart.

Failures before commit leave the existing recorder sender installed. External shortcut/hotkey
changes made during staging will use the existing rollback path where applicable. The recorder is
stopped only after the replacement options are known to be valid and no fallible UI/hook operation
remains between sender removal and replacement startup.

Tests will inject failures at restart planning and late staging boundaries and assert that the
recording sender survives.

### 3. Account-scoped cloud-library request generations

Cloud clip loading will carry a monotonically increasing request generation plus an account key
derived from the active host/user/credential identity. Reset, disconnect, and reconnect invalidate
the current generation. A response may update `cloudClipsCache`, loading state, or error state only
when both its generation and account key still match.

A forced refresh requested while another load is active will invalidate the older generation and
start a new request immediately. Both requests may finish, but only the newer generation may
publish its result.

Focused JavaScript tests will cover stale-response rejection and forced-refresh behavior; UI
contract tests will ensure the loader uses the guarded result path. Any extracted helper will stay
with the cloud loader instead of coupling account state to player logic.

### 4. Transactional osu! enrichment sidecar rename

Renaming an MP4 will include its optional `.osu-enrichment.json` sidecar. Before committing the
move, Clipline will parse the sidecar, update its embedded `clip_path` to the destination MP4, and
write the updated document through a temporary file. The MP4, marker, clip metadata, pending
enrichment, and poster moves will participate in the existing rollback sequence.

If the pending sidecar is malformed or cannot be rewritten, the rename fails without moving the
MP4. A successful rename leaves no pending sidecar at the old basename, and later enrichment writes
markers beside the renamed clip.

Tests will cover successful move/path rewrite, destination collision, malformed input, and rollback.

### 5. Complete clip sidecar lifecycle

The canonical per-clip sidecar set is:

- `.markers.json`
- `.clipline.json`
- `.osu-enrichment.json`
- `.poster.jpg`

The storage crate will include clip metadata in quota size accounting and GC deletion. App-side
manual deletion and delete-after-upload will use the same complete set of extensions, expressed
through a small local helper where crate boundaries prevent direct reuse. Regenerable posters remain
best-effort cleanup; user-authored title metadata is treated as persistent clip data.

Tests will assert byte accounting, GC cleanup, session-directory cleanup, and cloud delete cleanup
for `.clipline.json`.

### 6. Version-1 MP4 duration boxes

`mvhd`, `tkhd`, and `mdhd` will continue emitting compact version-0 layouts while their duration
fits in `u32`. When a duration exceeds `u32::MAX`, the writer will emit the ISO BMFF version-1
layout with 64-bit creation time, modification time, and duration fields. Timescale and all
unrelated fields remain unchanged.

Duration conversion to the movie timescale will use a `u128` intermediate so multiplication cannot
overflow before division. Existing readers already recognize version-1 movie headers; parser tests
will be extended for all three box types.

Tests will exercise the exact `u32::MAX` boundary and the first overflowing value without creating
large media payloads, then verify ordinary short recordings retain version 0.

### 7. Latest-position audio-preview source swap

Multi-audio clips may generate a mixed playback preview asynchronously. The current implementation
captures a resume time before awaiting that work, allowing subsequent seeks or normal playback to
make the captured value stale.

Immediately before swapping the video source, the frontend will resolve the resume position from:

1. the latest queued seek target, when a seek is still in flight;
2. otherwise the current finite `video.currentTime`;
3. otherwise the request-start fallback.

The source swap will clear or transfer the consumed queued seek so an old source's `seeked` event
cannot overwrite the new source position. Playback rate, play/pause intent, and trim range continue
to survive the swap.

The position-selection rule will live in `player-core.js` as pure logic with Rust-hosted unit tests.
UI contract coverage will verify that preview completion resolves the position after the await,
not before it. Rapid five-second seeks across any number of timeline events must land at the latest
requested position.

## Verification

For each fix:

1. Add and run the focused regression test; confirm it fails for the expected reason.
2. Implement only the corresponding root-cause fix.
3. Run the focused test and its subsystem test suite.

After all fixes:

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clean -p` plus fresh clippy for each materially changed Rust crate where cache-sensitive
  lints could be hidden
- Launch `cargo run -p clipline-app` after stopping any existing app process and manually verify the
  playback and settings-restart scenarios

## Commit Structure

The intended implementation commits are:

1. `fix(recording): preserve recoverable full sessions`
2. `fix(settings): stage recorder restart transactionally`
3. `fix(cloud): ignore stale library responses`
4. `fix(library): carry osu enrichment through rename`
5. `fix(storage): include clip metadata in cleanup`
6. `fix(mp4): write 64-bit duration boxes when needed`
7. `fix(player): preserve seeks across audio preview swaps`
