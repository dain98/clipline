# Fast Audio Sidecar Switching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> task-by-task. Every task begins with a focused failing test, receives an independent review, and
> ends in one conventional commit. Leave all checkboxes unticked as required by repository
> convention.

**Goal:** Replace whole-video audio previews with reusable per-track audio-only sidecars so an
uncached switch on the reproduced long clip becomes audible in approximately 0.5 to 2 seconds and
cached switches become nearly immediate, without reloading the video.

**Architecture:** A new native command copies every missing selected audio stream to its own cached
MP4 sidecar in one FFmpeg process. The existing serialized latest-request queue owns extraction and
readiness. The original `<video>` remains the sole clock and visual source; hidden audio elements
play the selected sidecars, with pure `player-core.js` decisions governing drift correction and
output muting.

**Tech Stack:** Rust workspace, `clipline-mp4`, Tauri 2, external FFmpeg, vanilla JavaScript,
WebView2 media elements, Boa-hosted pure JavaScript tests, Rust filesystem and structural UI tests.

**Approved design:**
`docs/superpowers/specs/2026-07-15-audio-sidecar-switching-design.md`

## Global Constraints

- Execute in the isolated `fix/resilient-seeking-lazy-previews` worktree from design commit
  `f4a0877`; never modify or stage the original checkout's uncommitted UI and `.gsi-spike` work.
- Keep the original review video source assigned while audio selection changes.
- Preserve authoritative logical seeking, trim state, playback rate, play/pause, marker navigation,
  keyboard shortcuts, and timeline behavior.
- Keep state/time policy DOM-free in `player-core.js`; browser effects remain in the existing UI
  modules.
- Keep one active native request and coalesce pending selections to the latest desired request.
- Use external FFmpeg through `clipline_capture::ffmpeg::locate`; do not link FFmpeg or add a media
  dependency.
- Keep source clips, recording, exports, and share audio behavior unchanged.
- Protect every audible sidecar from cache eviction and publish new sidecars atomically.
- Write the failing test before production code and retain the RED/GREEN evidence in each task
  report.
- Use one conventional commit per task and do not include `.superpowers/`, `target/`, or other
  scratch output in commits.

## File Map

- `crates/clipline-mp4/src/trim.rs` — neutral parsed media-track counts used to validate sidecars.
- `crates/clipline-mp4/src/lib.rs` — public export for the neutral inspection interface.
- `apps/clipline-app/src/library.rs` — Tauri request/response, per-track keys, cache reuse,
  multi-output extraction, validation, atomic publication, and cache protection.
- `apps/clipline-app/src/app.rs` — temporary parallel command registration, then legacy command
  removal.
- `apps/clipline-app/ui/player-core.js` — pure sidecar synchronization and output-routing decisions.
- `apps/clipline-app/ui/app-core.js` — shared audio transport state.
- `apps/clipline-app/ui/review-player.js` — sidecar media lifecycle, queue orchestration, and audio
  activation.
- `apps/clipline-app/ui/main.js` — video transport, rate, and volume events mirrored to sidecars.
- `apps/clipline-app/tests/player_core.rs` — Boa tests for pure sidecar decisions and existing queue
  invariants.
- `apps/clipline-app/tests/ui_contract.rs` — native-command and browser ownership contracts.
- `handoff.md` — final architecture, measurements, verification, and remaining limitations.

---

### Task 1: Neutral Media Track Inspection

**Files:**

- Modify: `crates/clipline-mp4/src/trim.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`

**Interfaces:**

- Add public `MediaTrackCounts { video: usize, audio: usize }` with `Debug`, `Clone`, `Copy`,
  `PartialEq`, and `Eq`.
- Add `media_track_counts(input: &[u8]) -> Result<MediaTrackCounts, TrimError>`.
- Preserve `audio_track_count`; implement it through `media_track_counts` to keep one parser path.

- [ ] **Step 1: Add RED parser tests**

In the existing `trim.rs` test module, add a test that builds one normal fixture containing one
video plus two audio tracks and one audio-only fixture containing one audio track. Assert exact
`MediaTrackCounts` for both. Extend the local fixture helper only as much as needed to omit video.

- [ ] **Step 2: Confirm RED**

Run:

```powershell
cargo test -p clipline-mp4 media_track_counts -- --nocapture
```

Expected: FAIL because `media_track_counts` and `MediaTrackCounts` do not exist.

- [ ] **Step 3: Implement the neutral helper**

Parse once through the existing `parse_movie`, count `TrackConfig::Video` and
`TrackConfig::Audio`, return the public value object, and delegate `audio_track_count` to its
`audio` field. Export both new names from `lib.rs`.

- [ ] **Step 4: Verify Task 1**

```powershell
cargo test -p clipline-mp4 media_track_counts -- --nocapture
cargo test -p clipline-mp4
cargo clippy -p clipline-mp4 --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```powershell
git add crates/clipline-mp4/src/trim.rs crates/clipline-mp4/src/lib.rs
git commit -m "feat(mp4): expose media track counts"
```

---

### Task 2: Native Per-Track Sidecar Command

**Files:**

- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**

- Add Tauri command `prepare_clip_audio_sidecars` alongside the still-registered legacy command.
- Request shape:
  `{ path, audioTrackIds, protectedPreviewPaths: string[] }`.
- Response shape: ordered `[{ audioTrackId, path }]`.
- Add a private resolved-track value containing the stable track ID and embedded audio-stream index.
- Add a private extraction-output value containing an audio-stream index, final cache path, and
  sibling temporary path.
- Cache key version: `audio-track-sidecar-v1`, hashed from canonical source identity, source length,
  source modification time, and one track ID.

- [ ] **Step 1: Add RED native behavior tests**

Add focused `audio_sidecar_*` tests in `library.rs` using a temporary cache and an injected extractor.
Use small valid audio-only MP4 bytes produced by the existing MP4 writer helpers. Cover:

1. two selected uncached tracks invoke the extractor exactly once and return two paths in marker
   order;
2. each returned file validates as `{ video: 0, audio: 1 }` and is materially smaller than the
   source fixture;
3. a second selection reuses existing per-track files and extracts only its missing track;
4. the key is per track rather than per selection combination;
5. active protected paths and every returned path survive pre/post-generation pruning;
6. an extractor error or invalid/missing output removes all temporary siblings and publishes no
   partial result;
7. the generated FFmpeg argument list has one input, one output per missing stream, explicit
   `-map 0:a:N`, `-vn`, `-c:a copy`, `-map_metadata -1`, and no video map or audio encoder.

Update `ui_contract.rs` with a registration/schema contract for the new command while leaving the
legacy browser contract untouched until Task 5.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p clipline-app audio_sidecar -- --nocapture
```

Expected: FAIL because the new command, response type, path policy, and extractor do not exist.

- [ ] **Step 3: Resolve selected tracks deterministically**

Read marker audio metadata through `markers_with_inferred_audio_tracks`, reject duplicate or unknown
IDs with the existing validation semantics, and resolve the selected entries in marker order. An
empty selection is rejected at this native boundary; fallback and muted modes never call it.

- [ ] **Step 4: Implement cache planning and reuse**

Create the cache directory, calculate one final path per resolved track, touch existing valid hits,
and partition the ordered request into hits and missing outputs. Before extraction, protect both the
frontend-provided active paths and requested cache hits from LRU pruning.

Do not read the source file into memory. Only metadata and small completed sidecars may be read for
validation. Treat an existing invalid sidecar as missing after removing it best effort.

- [ ] **Step 5: Implement one-process multi-output FFmpeg extraction**

Build one command for all missing outputs. For each output, map exactly its embedded audio stream,
disable video, stream-copy audio, strip metadata, select MP4, and target its unique sibling temp.
Use the existing console suppression, null stdin/stdout, and captured stderr conventions.

On process failure, delete every temp and return stderr context. After success, validate every temp
with `media_track_counts == { video: 0, audio: 1 }` and nonzero metadata before publishing any.

- [ ] **Step 6: Publish as one user-visible unit**

Atomically rename validated temps to their content-addressed finals. Track which finals this
invocation created. A collision accepts an already valid winner and removes the redundant temp. If
a later publication fails, remove remaining temps and any finals created by this invocation; never
remove a collision winner.

After success, protect frontend-active plus all returned sidecars during pruning, allow every
returned cache file through Tauri's asset protocol scope, and serialize the ordered response.

- [ ] **Step 7: Verify Task 2**

```powershell
cargo test -p clipline-app audio_sidecar -- --nocapture
cargo test -p clipline-app audio_preview_cache -- --nocapture
cargo test -p clipline-app --test ui_contract audio_sidecar -- --nocapture
cargo clippy -p clipline-app --all-targets -- -D warnings
```

The existing legacy command remains functional and registered in this intermediate commit so the
app is bisectable before browser integration changes.

- [ ] **Step 8: Commit**

```powershell
git add apps/clipline-app/src/library.rs apps/clipline-app/src/app.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(player): prepare cached audio sidecars"
```

---

### Task 3: Pure Sidecar Synchronization and Output Policy

**Files:**

- Modify: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/player-core.js`

**Interfaces:**

- Add `audioSidecarSyncDecision(videoState, sidecarState, options = {})`.
- Input fields used from `videoState`: `currentTime`, `playbackRate`, `paused`, and `ended`.
- Input field used from `sidecarState`: `currentTime`.
- Option: `forceSeek` defaults false.
- Decision shape: `{ seekTime: number|null, playbackRate: number, shouldPlay: boolean }`.
- Add `reviewAudioOutputDecision(mode, muted, volume)` for modes `direct`, `sidecars`, and `muted`.
- Output shape: `{ videoMuted, sidecarMuted, volume }`.

- [ ] **Step 1: Add RED Boa tests**

Add `audio_sidecar_sync_*` tests proving:

- activation/forced sync seeks to the finite nonnegative video time;
- ordinary drift at exactly or below 100 ms produces `seekTime: null`;
- drift above 100 ms seeks to the video time;
- a non-finite sidecar time seeks when the video time is valid;
- a non-finite video time never yields an invalid seek assignment;
- playback rate is copied when finite and positive, otherwise normalized to `1`;
- `shouldPlay` is false when paused or ended and true otherwise;
- direct mode makes only the video audible, sidecar mode makes only sidecars audible, muted mode
  silences both, and volume is clamped to `[0, 1]`.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p clipline-app --test player_core audio_sidecar -- --nocapture
```

Expected: FAIL because both pure helpers are absent.

- [ ] **Step 3: Implement the minimal decisions**

Use a private `AUDIO_SIDECAR_DRIFT_TOLERANCE_S = 0.1`. Do not add timers, DOM access, media
elements, promises, or mutations to `player-core.js`. Export both helpers from `PlayerCore`.

`reviewAudioOutputDecision` treats zero volume as muted and makes transport-level video muting
independent from the user-visible mute flag. Unknown modes must fail closed by muting both outputs.

- [ ] **Step 4: Verify Task 3**

```powershell
cargo test -p clipline-app --test player_core audio_sidecar -- --nocapture
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
```

- [ ] **Step 5: Commit**

```powershell
git add apps/clipline-app/tests/player_core.rs apps/clipline-app/ui/player-core.js
git commit -m "feat(player): define audio sidecar transport policy"
```

---

### Task 4: Browser Sidecar Transport Primitives

**Files:**

- Modify: `apps/clipline-app/ui/app-core.js`
- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**State:**

- `reviewAudioMode`: `direct`, `sidecars`, or `muted`.
- `reviewAudioMuted` and `reviewAudioVolume`: user-facing state independent of `video.muted`.
- `activeReviewAudioSidecars`: ordered records containing track ID, path, element, and generation.
- `reviewAudioSidecarGeneration`: invalidates old readiness, error, and play-promise callbacks.
- one bounded drift timer while sidecar mode is playing; no timer otherwise.

- [ ] **Step 1: Add RED structural contracts**

Add `audio_sidecar_transport_*` UI contracts that require:

- hidden sidecars are created with `new Audio()`, `preload = "auto"`, `convertFileSrc(path)`, and
  muted during preparation;
- one cleanup helper pauses, removes `src`, calls `load`, clears listeners/timers through generation
  invalidation, and empties the active set;
- one synchronization helper consumes `PlayerCore.audioSidecarSyncDecision` and never writes the
  video's time;
- `play`, `pause`, `timeupdate`, `seeked`, and `ratechange` events synchronize sidecars, with seeked
  using forced alignment;
- lifecycle helpers for file-handle release, suspend, clip close, open, and rename clear sidecars;
- volume and mute controls update the logical audio state and apply
  `PlayerCore.reviewAudioOutputDecision` instead of treating transport-level `video.muted` as the
  user's mute choice.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p clipline-app --test ui_contract audio_sidecar_transport -- --nocapture
```

Expected: FAIL because browser sidecar lifecycle and logical audio output state are absent.

- [ ] **Step 3: Add preparation and disposal helpers**

Create sidecars dynamically; do not add permanent audio elements to `index.html`. A preparation
helper accepts the native `{audioTrackId, path}` list and request generation, waits for every local
audio element to load metadata, aligns each muted element to the latest video time, and rejects as
one operation on any media error. It returns a prepared set without making it audible.

Dispose failed, stale, and superseded prepared sets immediately. Cleanup must release asset file
handles so rename/delete behavior remains reliable on Windows.

- [ ] **Step 4: Add video-authoritative synchronization**

Mirror play/pause/rate and apply pure drift decisions to active sidecars. Force alignment on
activation and `seeked`; use ordinary tolerance on `timeupdate` and a bounded periodic check while
playing. Catch sidecar `play()` rejections and route current-generation failures through the review
audio failure path rather than leaving original audio muted.

- [ ] **Step 5: Separate user audio controls from transport muting**

Make `syncVolume`, `toggleMute`, and the volume-slider handler use `reviewAudioMuted` and
`reviewAudioVolume`. One output application helper sets video and sidecar volume/muting according
to the pure output decision. With no active sidecars, direct playback must behave exactly as before.

This commit adds and wires the transport primitives but does not yet change the preview queue to
activate them; `activeReviewAudioSidecars` remains empty during ordinary use until Task 5.

- [ ] **Step 6: Verify Task 4**

```powershell
cargo test -p clipline-app --test ui_contract audio_sidecar_transport -- --nocapture
cargo test -p clipline-app --test player_core audio_sidecar -- --nocapture
cargo test -p clipline-app --test ui_contract review_player_owns_all_controls -- --nocapture
```

- [ ] **Step 7: Commit**

```powershell
git add apps/clipline-app/ui/app-core.js apps/clipline-app/ui/review-player.js apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(player): add synchronized audio sidecar transport"
```

---

### Task 5: Activate Sidecars Through the Latest-Request Queue

**Files:**

- Modify: `apps/clipline-app/ui/review-player.js`
- Modify: `apps/clipline-app/ui/main.js` only if an event hook needs a final adjustment
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `apps/clipline-app/tests/player_core.rs` only if queue coverage needs the protected-path
  request field added to its fixture

- [ ] **Step 1: Replace source-swap contracts with RED sidecar activation contracts**

Update the old `valid_preview_swap_*` and related structural tests. Require that:

- exactly one browser call invokes `prepare_clip_audio_sidecars`;
- its request sends current selected IDs and every active sidecar path as
  `protectedPreviewPaths`;
- the preview request/finish path contains no `setReviewVideoSource`, `assignReviewVideoSource`,
  `video.src`, or preview-path assignment;
- the current original video remains audible while extraction and sidecar readiness are pending;
- the queue is not finished until preparation succeeds or fails;
- only `transition.apply` plus the current clip/selection/source-generation gates may activate a
  prepared set;
- post-await activation rereads `reviewPlayheadTime()`, paused/ended state, and playback rate;
- stale prepared elements are disposed and a coalesced `transition.start` launches only the newest
  desired request;
- fallback clears sidecars and restores direct video audio without changing `video.src`;
- empty selection clears sidecars and selects muted output without changing `video.src`;
- failure leaves the active set/video untouched and restores checkboxes to
  `currentReviewAudioTrackIds`.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p clipline-app --test ui_contract audio_sidecar_activation -- --nocapture
cargo test -p clipline-app --test ui_contract explicit_audio_preview -- --nocapture
```

Expected: FAIL because the queue still invokes the legacy command and swaps the video source.

- [ ] **Step 3: Change request orchestration without weakening queue ownership**

Keep `emptyAudioPreviewQueue`, `queueAudioPreviewRequest`, `cancelAudioPreviewRequest`, and
`finishAudioPreviewRequest` unchanged unless a RED pure test demonstrates a missing transition.

At request start, snapshot active sidecar paths only for cache protection. Await the native result.
If the request is still the current desired selection, prepare its returned sidecars while muted;
otherwise skip loading. Finish the queue only after native extraction and any necessary preparation
complete. A newer desired selection that arrives during preparation must suppress apply, dispose the
prepared set, and start only that newest request.

- [ ] **Step 4: Activate all-or-nothing audio**

For a valid `transition.apply`, reread current video transport state, force-align the complete
prepared set, and, when the video is playing, start every prepared element while it is still muted
and await all play promises. Install the set and switch output mode to `sidecars` only after those
promises succeed. Do not silence original video audio until every sidecar is ready and the request
passes the final generation/selection gate. Update `currentReviewAudioTrackIds`,
`currentReviewAudioKey`, controls, and resolved status only after activation succeeds.

- [ ] **Step 5: Implement direct fallback, muted, error, and lifecycle behavior**

Fallback/muted selections cancel desired queue work, invalidate pending preparation, dispose active
sidecars, and select `direct` or `muted` output respectively. They never assign the video source.

Current-request extraction/load/play failure keeps the previously audible media set, restores the
checkbox selection, and reports a transient nonfatal error. Open/close/suspend/release/rename clear
or rebuild sidecars consistently; rename may request the still-selected tracks only after the
renamed original video source is current.

- [ ] **Step 6: Verify Task 5**

```powershell
cargo test -p clipline-app --test ui_contract audio_sidecar -- --nocapture
cargo test -p clipline-app --test ui_contract explicit_audio_preview -- --nocapture
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
cargo test -p clipline-app
```

- [ ] **Step 7: Commit**

```powershell
git add apps/clipline-app/ui/review-player.js apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs apps/clipline-app/tests/player_core.rs
git commit -m "fix(player): switch audio without reloading video"
```

Stage only files actually changed.

---

### Task 6: Remove Whole-Video Preview Code and Verify the Product

**Files:**

- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `handoff.md`

- [ ] **Step 1: Add RED legacy-absence contracts**

Add or replace structural assertions requiring that the shipped app has:

- no registered or invoked `preview_clip_audio_tracks` command;
- no legacy single `protectedPreviewPath` request field;
- no full-preview `audio-preview-mix-v4` key;
- no review preview path passed to `setReviewVideoSource`;
- no preview-only whole-source `std::fs::read(&source)` path;
- no preview-only FFmpeg command that maps `0:v:0` or uses `amix`.

Do not prohibit full-source reads or mixing used by export/share features; scope assertions to the
legacy preview functions and names so they cannot false-positive on valid export code.

- [ ] **Step 2: Confirm RED**

```powershell
cargo test -p clipline-app --test ui_contract legacy_audio_preview -- --nocapture
```

Expected: FAIL while the parallel legacy command and generator still exist.

- [ ] **Step 3: Remove only superseded preview code**

Delete the legacy request type, Tauri command registration, full-preview cache-key function,
whole-file preview builder, preview-only write helper/tests, and preview-only FFmpeg `amix` function.
Keep shared cache pruning, temp guards, MP4 remux/export helpers, share audio mixing, and source
validation intact.

Old `audio-preview-mix-v4` files remain recognizable cache candidates and are naturally evicted;
do not add a destructive one-time deletion path.

- [ ] **Step 4: Run focused regression gates**

```powershell
cargo test -p clipline-mp4 media_track_counts -- --nocapture
cargo test -p clipline-app audio_sidecar -- --nocapture
cargo test -p clipline-app audio_preview_cache -- --nocapture
cargo test -p clipline-app --test player_core audio_sidecar -- --nocapture
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
cargo test -p clipline-app --test ui_contract audio_sidecar -- --nocapture
cargo test -p clipline-app --test ui_contract legacy_audio_preview -- --nocapture
```

- [ ] **Step 5: Run fresh full verification**

Stop only the worktree `clipline-app.exe` before rebuilding. Then run:

```powershell
cargo test --workspace
cargo clean -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
```

All tests must pass and both Clippy commands must emit zero warnings.

- [ ] **Step 6: Update the handoff**

Document the measured old path (1.88 GiB whole-video previews and 15-second pre-mix), the new
per-track sidecar architecture, 100 ms drift policy, cache protection set, tests, verification
counts, and the remaining non-cancellable active FFmpeg limitation.

- [ ] **Step 7: Commit cleanup and handoff**

```powershell
git add apps/clipline-app/src/library.rs apps/clipline-app/src/app.rs apps/clipline-app/tests/ui_contract.rs handoff.md
git commit -m "refactor(player): remove whole-video audio previews"
```

- [ ] **Step 8: Launch the worktree app for manual acceptance**

For the development build, point Clipline at the known installed FFmpeg if automatic location does
not find it:

```powershell
$env:CLIPLINE_FFMPEG='C:\Users\dain\AppData\Local\Clipline\ffmpeg\ffmpeg.exe'
cargo run -p clipline-app
```

Using the reproduced 31-minute clip, ask the user to verify:

1. first uncached one-track and multi-track switches become audible in approximately 0.5 to 2
   seconds;
2. repeated cached switches are nearly immediate;
3. video never reloads, stutters, or jumps to zero during switching and rapid seeking;
4. only the newest rapid selection becomes audible;
5. play, pause, scrub, playback rate, fallback, mute, clip changes, and rename keep audio aligned;
6. extraction failure leaves the previously audible selection playing;
7. restart pruning keeps total preview-cache bytes within policy except for active protected files.

- [ ] **Step 9: Independent final review**

Review the complete implementation diff from `f4a0877` through `HEAD`, resolve every Critical or
Important finding with another RED/GREEN commit, rerun affected focused tests, and then repeat the
workspace verification gates before declaring the branch ready.
