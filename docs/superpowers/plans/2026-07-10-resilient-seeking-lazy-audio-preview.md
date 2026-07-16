# Resilient Seeking and Lazy Audio Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep rapid seek targets authoritative across media events and source swaps while making multi-track audio previews explicit, serialized, and bounded to a 2 GiB reusable cache.

**Architecture:** Pure state-transition helpers in `player-core.js` own logical seeking, review-audio selection, and preview request coalescing. `review-player.js` is the only browser/Tauri integration layer that applies those decisions to the media element, while `library.rs` retains media generation and gains atomic, protected-path-aware LRU cleanup.

**Tech Stack:** Rust workspace, vanilla JavaScript, Tauri 2, WebView2 HTML media element, Boa-hosted JavaScript tests, Rust filesystem tests.

## Global Constraints

- Execute from an isolated git worktree created from the commit containing this plan (whose implementation diff base is `b049255c`); do not stage or modify the original checkout's uncommitted shortcut or `.gsi-spike` changes.
- Keep player state transitions and time calculations in the DOM-free `apps/clipline-app/ui/player-core.js` layer.
- Preserve keyboard, button, scrubber, marker, trim-range, playback-rate, and play/pause behavior.
- Do not change recording, export, or source media files.
- Do not add a frontend framework, database, background service, dependency, or media format.
- Do not build a streaming or cancellable mixer; preview work becomes lazy, serialized, and coalesced.
- Use failing tests before production changes and leave every checkbox in this plan unticked.
- Use conventional commits, one logical commit per task.

## File Structure

- `apps/clipline-app/ui/player-core.js` — pure logical-seek state, review-audio selection rules, and preview queue transitions.
- `apps/clipline-app/ui/app-core.js` — shared review audio state and audio-panel rendering adapters.
- `apps/clipline-app/ui/review-player.js` — media source generations, DOM seek application, preview command orchestration, and source swaps.
- `apps/clipline-app/ui/main.js` — media event consumers that must render the logical playhead.
- `apps/clipline-app/tests/player_core.rs` — behavioral tests for all pure JavaScript transitions through Boa.
- `apps/clipline-app/tests/ui_contract.rs` — structural contracts proving the browser layer uses the pure state and never previews on open.
- `apps/clipline-app/src/library.rs` — preview request schema, generation, atomic publication, cache touch, and LRU pruning.
- `apps/clipline-app/src/app.rs` — best-effort preview-cache cleanup during startup.
- `handoff.md` — root cause, final behavior, verification evidence, and remaining non-cancellable-job limitation.

---

### Task 1: Pure Logical Seek State

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs:398-464`
- Modify: `apps/clipline-app/ui/player-core.js:251-281`
- Modify: `apps/clipline-app/ui/player-core.js:1847-1851`

**Interfaces:**
- Consumes: existing `clampTime(value, duration)`.
- Produces: `createLogicalSeekState()`, `requestLogicalSeek(state, time, duration)`, `beginSourceAssignment(state, generation, resumeTime, duration)`, `metadataSeekDecision(state, generation, duration)`, `seekedDecision(state, generation, currentTime, duration)`, and `logicalPlaybackTime(state, currentTime, duration)`.
- State shape: `{ targetTime: number|null, sourceGeneration: number, metadataGeneration: number|null }`.
- Decision shape: `{ state, applyTime: number|null, confirmed: boolean }`.

- [ ] **Step 1: Write the failing logical-seek regression tests**

Replace the narrow source-swap tests with these tests while retaining the existing `relativeSeekTarget` coverage:

```rust
#[test]
fn logical_seek_survives_early_seeked_and_source_swap() {
    let mut ctx = player_core_context();
    let result = eval_json(
        &mut ctx,
        r#"
        (() => {
          let state = PlayerCore.createLogicalSeekState();
          state = PlayerCore.beginSourceAssignment(state, 1, 10, 60);
          let decision = PlayerCore.metadataSeekDecision(state, 1, 60);
          state = decision.state;
          state = PlayerCore.seekedDecision(state, 1, 10, 60).state;
          for (const delta of [5, 5, 5, 5, 5]) {
            const target = PlayerCore.relativeSeekTarget(10, state.targetTime, delta, 60);
            state = PlayerCore.requestLogicalSeek(state, target, 60);
          }
          state = PlayerCore.beginSourceAssignment(state, 2, 0, 60);
          const early = PlayerCore.seekedDecision(state, 2, 0, 60);
          const metadata = PlayerCore.metadataSeekDecision(early.state, 2, 60);
          const prior = PlayerCore.seekedDecision(metadata.state, 2, 30, 60);
          const arrived = PlayerCore.seekedDecision(prior.state, 2, 35, 60);
          return {
            targetAfterEarlyEvent: early.state.targetTime,
            earlyApply: early.applyTime,
            metadataApply: metadata.applyTime,
            priorApply: prior.applyTime,
            confirmed: arrived.confirmed,
            finalTarget: arrived.state.targetTime,
          };
        })()
        "#,
    );

    assert_eq!(
        result,
        r#"{"targetAfterEarlyEvent":35,"earlyApply":null,"metadataApply":35,"priorApply":35,"confirmed":true,"finalTarget":null}"#
    );
}

#[test]
fn logical_seek_ignores_invalid_requests_and_clamps_when_metadata_arrives() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(
            &mut ctx,
            r#"
            (() => {
              let state = PlayerCore.createLogicalSeekState();
              state = PlayerCore.requestLogicalSeek(state, 75, 0);
              state = PlayerCore.requestLogicalSeek(state, NaN, 0);
              const metadata = PlayerCore.metadataSeekDecision(
                PlayerCore.beginSourceAssignment(state, 7, 0, 60),
                7,
                60,
              );
              return {
                applyTime: metadata.applyTime,
                logical: PlayerCore.logicalPlaybackTime(metadata.state, 0, 60),
              };
            })()
            "#,
        ),
        r#"{"applyTime":60,"logical":60}"#
    );
}
```

- [ ] **Step 2: Run the focused tests and confirm the expected failure**

Run:

```powershell
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
```

Expected: FAIL because `PlayerCore.createLogicalSeekState` is not defined.

- [ ] **Step 3: Add the minimal pure seek coordinator**

Add beside `clampTime` in `player-core.js`:

```javascript
  const SEEK_CONFIRM_TOLERANCE_S = 0.1;

  const createLogicalSeekState = () => ({
    targetTime: null,
    sourceGeneration: 0,
    metadataGeneration: null,
  });

  const requestLogicalSeek = (state, time, duration) => {
    if (!Number.isFinite(time)) return state;
    return { ...state, targetTime: clampTime(time, duration) };
  };

  const beginSourceAssignment = (state, sourceGeneration, resumeTime, duration) => {
    const requested = Number.isFinite(state && state.targetTime)
      ? state.targetTime
      : Number.isFinite(resumeTime) ? resumeTime : 0;
    return {
      targetTime: clampTime(requested, duration),
      sourceGeneration,
      metadataGeneration: null,
    };
  };

  const metadataSeekDecision = (state, sourceGeneration, duration) => {
    if (!state || state.sourceGeneration !== sourceGeneration) {
      return { state, applyTime: null, confirmed: false };
    }
    const targetTime = Number.isFinite(state.targetTime)
      ? clampTime(state.targetTime, duration)
      : null;
    const next = { ...state, targetTime, metadataGeneration: sourceGeneration };
    return { state: next, applyTime: targetTime, confirmed: false };
  };

  const seekedDecision = (state, sourceGeneration, currentTime, duration) => {
    if (!state
        || state.sourceGeneration !== sourceGeneration
        || state.metadataGeneration !== sourceGeneration
        || !Number.isFinite(state.targetTime)
        || !Number.isFinite(currentTime)) {
      return { state, applyTime: null, confirmed: false };
    }
    const targetTime = clampTime(state.targetTime, duration);
    if (Math.abs(currentTime - targetTime) <= SEEK_CONFIRM_TOLERANCE_S) {
      return {
        state: { ...state, targetTime: null },
        applyTime: null,
        confirmed: true,
      };
    }
    return {
      state: { ...state, targetTime },
      applyTime: targetTime,
      confirmed: false,
    };
  };

  const logicalPlaybackTime = (state, currentTime, duration) => {
    const time = Number.isFinite(state && state.targetTime)
      ? state.targetTime
      : Number.isFinite(currentTime) ? currentTime : 0;
    return clampTime(time, duration);
  };
```

Export all six functions from the `PlayerCore` return object. Keep the old source-swap helpers temporarily; Task 2 removes them only after browser integration no longer consumes them.

- [ ] **Step 4: Run the focused and complete pure-player suites**

```powershell
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
cargo test -p clipline-app --test player_core
```

Expected: both commands PASS.

- [ ] **Step 5: Commit the pure state machine**

```powershell
git add apps/clipline-app/tests/player_core.rs apps/clipline-app/ui/player-core.js
git commit -m "fix(player): make seek targets authoritative"
```

---

### Task 2: Browser Seek and Source-Swap Integration

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs:1685-1834`
- Modify: `apps/clipline-app/tests/player_core.rs:398-464`
- Modify: `apps/clipline-app/ui/player-core.js:257-281,1848-1851`
- Modify: `apps/clipline-app/ui/review-player.js:101-152,389-401,681-729,916-968`
- Modify: `apps/clipline-app/ui/main.js:298-329`

**Interfaces:**
- Consumes: Task 1 logical-seek functions and state shape.
- Produces: `reviewPlayheadTime()`, a generation-gated `assignReviewVideoSource(path, options)`, and media event handlers that never clear an unconfirmed target.
- Removes: `pendingSeek`, `reviewSeekRevision`, `consumeSourceSwapResumeTime`, `sourceSwapResumeTime`, and `sourceRestoreDecision`.

- [ ] **Step 1: Replace the structural tests with authoritative-state contracts**

```rust
#[test]
fn review_player_applies_logical_seek_only_for_current_metadata() {
    let review = read_ui_js("review-player.js");
    let assign = js_function_body(&review, "assignReviewVideoSource");
    assert!(assign.contains("PlayerCore.beginSourceAssignment("));
    assert!(assign.contains("PlayerCore.metadataSeekDecision("));
    assert!(assign.contains("assignment.sourceGeneration !== reviewSourceGeneration"));

    let seek_to = js_function_body(&review, "seekTo");
    assert!(seek_to.contains("PlayerCore.requestLogicalSeek("));
    assert!(seek_to.contains("reviewSeekState.metadataGeneration === reviewSourceGeneration"));

    assert!(review.contains("PlayerCore.seekedDecision("));
    assert!(review.contains("function reportReviewSourceError(assignment)"));
    assert!(assign.contains("video.addEventListener(\"error\""));
    assert!(review.contains("function reviewPlayheadTime()"));
    assert!(!review.contains("pendingSeek"));
    assert!(!review.contains("reviewSeekRevision"));
}

#[test]
fn timeline_and_media_events_render_the_logical_playhead() {
    let review = read_ui_js("review-player.js");
    let main = read_ui_js("main.js");
    assert!(js_function_body(&review, "paintTimeline").contains("reviewPlayheadTime()"));
    assert!(js_function_body(&review, "paintOverview").contains("reviewPlayheadTime()"));
    assert!(js_function_body(&review, "seekBy").contains("reviewSeekState.targetTime"));
    assert!(main.contains("const current = reviewPlayheadTime();"));
}

#[test]
fn opening_a_clip_clears_only_the_previous_clips_seek_state() {
    let review = read_ui_js("review-player.js");
    let open_clip = js_function_body(&review, "openClip");
    assert!(open_clip.contains("reviewSeekState = PlayerCore.createLogicalSeekState();"));
    assert!(open_clip.contains("assignReviewVideoSource(clip.path, { resumeTime: 0 })"));
}
```

- [ ] **Step 2: Run the UI contracts and confirm they fail**

```powershell
cargo test -p clipline-app --test ui_contract review_player_applies_logical_seek -- --nocapture
cargo test -p clipline-app --test ui_contract timeline_and_media_events_render -- --nocapture
```

Expected: FAIL because `reviewSeekState` and `reviewPlayheadTime` are absent and `pendingSeek` remains.

- [ ] **Step 3: Centralize source metadata through the seek coordinator**

Replace the source globals and assignment helper in `review-player.js` with:

```javascript
var reviewSourceGeneration = 0;
var reviewSeekState = PlayerCore.createLogicalSeekState();

function reviewPlayheadTime() {
  return PlayerCore.logicalPlaybackTime(reviewSeekState, video.currentTime, clipDuration());
}

function assignReviewVideoSource(path, options = {}) {
  const { resumeTime = 0, onLoadedMetadata = null } = options;
  const assignment = { sourceGeneration: ++reviewSourceGeneration };
  reviewSeekState = PlayerCore.beginSourceAssignment(
    reviewSeekState,
    assignment.sourceGeneration,
    resumeTime,
    clipDuration(),
  );
  video.addEventListener("loadedmetadata", () => {
    const decision = PlayerCore.metadataSeekDecision(
      reviewSeekState,
      assignment.sourceGeneration,
      video.duration,
    );
    reviewSeekState = decision.state;
    if (assignment.sourceGeneration !== reviewSourceGeneration) return;
    if (decision.applyTime != null) video.currentTime = decision.applyTime;
    if (typeof onLoadedMetadata === "function") onLoadedMetadata(assignment);
  }, { once: true });
  video.addEventListener("error", () => reportReviewSourceError(assignment), { once: true });
  currentReviewMediaPath = path;
  video.src = convertFileSrc(path);
  return assignment;
}
```

Move the direct media-error display out of `main.js` and add this generation guard in
`review-player.js` so a superseded assignment cannot publish an error:

```javascript
function reportReviewSourceError(assignment) {
  if (assignment.sourceGeneration !== reviewSourceGeneration) return;
  const error = video.error;
  $("stage-note").textContent = `load error ${error ? error.code : "?"}`;
}
```

Make `releaseReviewVideoSource()` increment the generation and call `beginSourceAssignment` with `reviewPlayheadTime()` before removing `src`; this marks metadata unavailable without discarding an active target.

Change `setReviewVideoSource` so its metadata callback checks only source ownership, restores trim and play/pause state, and lets `assignReviewVideoSource` apply position:

```javascript
  const restore = (assignment) => {
    if (assignment.sourceGeneration !== reviewSourceGeneration) return;
    if (trimRange) setTrim(trimRange.start, trimRange.end);
    if (shouldResume) video.play().catch(() => syncPlayState());
    else syncPlayState();
  };
  assignReviewVideoSource(path, { resumeTime, onLoadedMetadata: restore });
```

Delete `reviewSeekRevision`, `sourceRestoreDecision`, and their old tests/exports.

- [ ] **Step 4: Replace pending-seek ownership with confirmation-based handling**

```javascript
function seekTo(time, options = {}) {
  if (!currentClip || !Number.isFinite(time)) return;
  if (!options.keepGameEventSelection) clearGameEventSelection();
  if (!options.keepGamePlaySelection) clearGamePlaySelection();
  reviewSeekState = PlayerCore.requestLogicalSeek(reviewSeekState, time, clipDuration());
  const target = reviewSeekState.targetTime;
  if (reviewSeekState.metadataGeneration === reviewSourceGeneration && !video.seeking) {
    video.currentTime = target;
  }
  maybeFollow(target);
  paintTimeline();
  syncGameEventRail(target);
  syncGamePlayRail(target, { keepGamePlaySelection: options.keepGamePlaySelection });
}

video.addEventListener("seeked", () => {
  const decision = PlayerCore.seekedDecision(
    reviewSeekState,
    reviewSourceGeneration,
    video.currentTime,
    clipDuration(),
  );
  reviewSeekState = decision.state;
  if (decision.applyTime != null) video.currentTime = decision.applyTime;
  const current = reviewPlayheadTime();
  maybeFollow(current);
  paintTimeline();
  syncGameEventRail(current);
  syncGamePlayRail(current);
});

function seekBy(delta) {
  seekTo(PlayerCore.relativeSeekTarget(
    video.currentTime,
    reviewSeekState.targetTime,
    delta,
    clipDuration(),
  ));
}
```

Use `reviewPlayheadTime()` in `jumpEdit`, `paintTimeline`, and `paintOverview`. Replace both preview completion calls to `consumeSourceSwapResumeTime(resumeTime)` with `reviewPlayheadTime()`, then delete `consumeSourceSwapResumeTime`, `pendingSeek`, and `sourceSwapResumeTime`.

Reset `reviewSeekState` before assigning a newly opened clip and after closing/suspending review playback. Source swaps within the same clip must not reset it.

- [ ] **Step 5: Make general media events consume logical time**

In `main.js`:

```javascript
video.addEventListener("play", () => {
  const current = reviewPlayheadTime();
  syncPlayState();
  syncGameEventRail(current);
  syncGamePlayRail(current);
  paintTimeline();
  scheduleOverlayIdleCheck();
});

video.addEventListener("timeupdate", () => {
  const current = reviewPlayheadTime();
  maybeFollow(current);
  paintTimeline();
  syncGameEventRail(current);
  syncGamePlayRail(current);
});
```

Keep the existing `pause`, `volumechange`, metadata display, and error behavior.

- [ ] **Step 6: Run focused and full app test suites**

```powershell
cargo test -p clipline-app --test player_core
cargo test -p clipline-app --test ui_contract
cargo test -p clipline-app
```

Expected: all commands PASS, including the exact rapid-seek/source-swap regression.

- [ ] **Step 7: Commit the browser integration**

```powershell
git add apps/clipline-app/tests/player_core.rs apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/player-core.js apps/clipline-app/ui/review-player.js apps/clipline-app/ui/main.js
git commit -m "fix(player): preserve logical seeks across source events"
```

---

### Task 3: Direct Fallback Audio on Clip Open

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs:947-1043`
- Modify: `apps/clipline-app/tests/ui_contract.rs:1615-1683`
- Modify: `apps/clipline-app/ui/player-core.js:711-795,1880-1885`
- Modify: `apps/clipline-app/ui/app-core.js:138-142,256-343`
- Modify: `apps/clipline-app/ui/review-player.js:228-252,389-460`

**Interfaces:**
- Consumes: clip marker `audio_tracks` in embedded order.
- Produces: `directPlaybackAudioTrackIds(tracks)`, `selectedReviewAudioTrackIds(tracks, selectedIds)`, `reviewSelectionNeedsPreview(tracks, selectedIds)`, `reviewAudioTrackRowState(track, tracks, selectedIds)`, `applyReviewAudioTrackToggle(tracks, selectedIds, trackId, checked)`, and `reviewAudioTrackSelectedRowCount(tracks, selectedIds)`.
- Preserves: existing `defaultAudioTrackIds` for the upload dialog's default composition. Review-aware row/toggle rules are also used by the upload dialog so an aggregate fallback ID is never displayed as unchecked while selected.

- [ ] **Step 1: Write failing fallback-selection tests**

```rust
#[test]
fn review_audio_defaults_to_direct_fallback_and_explicit_tracks_need_preview() {
    let mut ctx = player_core_context();
    let model = eval_json(
        &mut ctx,
        r#"
        (() => {
          const tracks = [
            { id: 'output', kind: 'output', label: 'Output Audio' },
            { id: 'process:1', kind: 'process_output', label: 'Game' },
            { id: 'microphone', kind: 'microphone', label: 'Microphone' },
          ];
          const fallback = PlayerCore.directPlaybackAudioTrackIds(tracks);
          const process = PlayerCore.applyReviewAudioTrackToggle(
            tracks, fallback, 'process:1', true,
          );
          const withMic = PlayerCore.applyReviewAudioTrackToggle(
            tracks, process, 'microphone', true,
          );
          const restored = PlayerCore.applyReviewAudioTrackToggle(
            tracks, withMic, 'output', true,
          );
          const muted = PlayerCore.applyReviewAudioTrackToggle(
            tracks, fallback, 'output', false,
          );
          return {
            fallback,
            fallbackRow: PlayerCore.reviewAudioTrackRowState(tracks[0], tracks, fallback),
            fallbackNeedsPreview: PlayerCore.reviewSelectionNeedsPreview(tracks, fallback),
            process,
            withMic,
            restored,
            muted,
            expandedNeedsPreview: PlayerCore.reviewSelectionNeedsPreview(tracks, withMic),
          };
        })()
        "#,
    );

    assert_eq!(
        model,
        r#"{"fallback":["output"],"fallbackRow":{"checked":true,"indeterminate":false},"fallbackNeedsPreview":false,"process":["process:1"],"withMic":["process:1","microphone"],"restored":["output","microphone"],"muted":[],"expandedNeedsPreview":true}"#
    );
}
```

Replace the old default-preview UI contract with:

```rust
#[test]
fn opening_multitrack_clip_plays_original_source_without_preview() {
    let review = read_ui_js("review-player.js");
    let open_clip = js_function_body(&review, "openClip");
    assert!(open_clip.contains("resetSelectedAudioTracks(clip);"));
    assert!(open_clip.contains("assignReviewVideoSource(clip.path, { resumeTime: 0 })"));
    assert!(open_clip.contains("video.play().catch(() => syncPlayState());"));
    assert!(!open_clip.contains("applySelectedAudioTracksToPlayback"));
    assert!(!main_js().contains("function applyDefaultAudioSelectionIfNeeded"));
}

#[test]
fn review_and_upload_audio_controls_render_exact_selected_ids() {
    let app_core = read_ui_js("app-core.js");
    let review_panel = js_function_body(&app_core, "renderAudioTrackPanel");
    let upload_panel = js_function_body(&app_core, "renderUploadAudioTracks");
    assert!(review_panel.contains("PlayerCore.reviewAudioTrackRowState"));
    assert!(review_panel.contains("PlayerCore.applyReviewAudioTrackToggle"));
    assert!(upload_panel.contains("PlayerCore.reviewAudioTrackRowState"));
    assert!(upload_panel.contains("PlayerCore.applyReviewAudioTrackToggle"));
}
```

- [ ] **Step 2: Run the focused tests and confirm they fail**

```powershell
cargo test -p clipline-app --test player_core review_audio_defaults -- --nocapture
cargo test -p clipline-app --test ui_contract opening_multitrack_clip -- --nocapture
cargo test -p clipline-app --test ui_contract review_and_upload_audio_controls -- --nocapture
```

Expected: FAIL because the review-specific helpers are absent and `openClip` still invokes default preview generation.

- [ ] **Step 3: Add review-only selection semantics**

Add beside the existing audio helpers:

```javascript
  const directPlaybackAudioTrackIds = (tracks) => {
    const first = normalizedAudioTracks(tracks).map(audioTrackId).find(Boolean);
    return first ? [first] : [];
  };

  const selectedReviewAudioTrackIds = (tracks, selectedIds) => {
    const selected = audioIdSet(selectedIds);
    const valid = normalizedAudioTracks(tracks).map(audioTrackId).filter(Boolean);
    return valid.filter((id) => selected.has(id));
  };

  const reviewSelectionNeedsPreview = (tracks, selectedIds) => {
    const selected = selectedReviewAudioTrackIds(tracks, selectedIds);
    const direct = directPlaybackAudioTrackIds(tracks);
    return selected.length !== direct.length
      || selected.some((id, index) => id !== direct[index]);
  };

  const reviewAudioTrackRowState = (track, tracks, selectedIds) => ({
    checked: selectedReviewAudioTrackIds(tracks, selectedIds).includes(audioTrackId(track)),
    indeterminate: false,
  });

  const applyReviewAudioTrackToggle = (tracks, selectedIds, trackId, checked) => {
    const allTracks = normalizedAudioTracks(tracks);
    const selected = new Set(selectedReviewAudioTrackIds(allTracks, selectedIds));
    const track = allTracks.find((candidate) => audioTrackId(candidate) === String(trackId));
    if (!track) return [...selected];
    if (checked && isMixedOutputTrack(track) && hasSplitOutputTracks(allTracks)) {
      for (const processTrack of processOutputTracks(allTracks)) {
        selected.delete(audioTrackId(processTrack));
      }
    }
    if (checked && isProcessOutputTrack(track) && hasSplitOutputTracks(allTracks)) {
      for (const candidate of allTracks) {
        if (isMixedOutputTrack(candidate)) selected.delete(audioTrackId(candidate));
      }
    }
    if (checked) selected.add(audioTrackId(track));
    else selected.delete(audioTrackId(track));
    return selectedReviewAudioTrackIds(allTracks, [...selected]);
  };

  const reviewAudioTrackSelectedRowCount = (tracks, selectedIds) =>
    normalizedAudioTracks(tracks).filter((track) =>
      reviewAudioTrackRowState(track, tracks, selectedIds).checked
    ).length;
```

Export these six helpers. Keep the existing upload/export helpers unchanged.

- [ ] **Step 4: Use fallback state in the review panel**

In `app-core.js`:

```javascript
var currentReviewAudioTrackIds = [];

function resetSelectedAudioTracks(clip = currentClip) {
  selectedAudioTrackIds = new Set(
    PlayerCore.directPlaybackAudioTrackIds(clipAudioTracks(clip)),
  );
}

function selectedAudioTrackIdsForClip(clip = currentClip, selected = selectedAudioTrackIds) {
  return PlayerCore.selectedReviewAudioTrackIds(clipAudioTracks(clip), [...selected]);
}
```

Delete `applyDefaultAudioSelectionIfNeeded`. Add a `rowState` option to `renderAudioTrackRows` and pass `PlayerCore.reviewAudioTrackRowState` from both `renderAudioTrackPanel` and `renderUploadAudioTracks`. Use `reviewAudioTrackSelectedRowCount` in the review summary and `audioSelectionLabel`. Use `applyReviewAudioTrackToggle` in both checkbox callbacks. `openUploadDialog` may still initialize non-current clips from `defaultAudioTrackIds`; the row state now represents those exact IDs rather than treating the aggregate output as a process-track master.

- [ ] **Step 5: Open original media immediately**

In `openClip`, after resetting selection:

```javascript
  currentReviewAudioTrackIds = selectedAudioTrackIdsForClip(clip);
  currentReviewAudioKey = audioSelectionKey(clip, currentReviewAudioTrackIds);
```

Assign with `assignReviewVideoSource(clip.path, { resumeTime: 0 })` and always call `video.play().catch(() => syncPlayState())`. Remove the default-preview branch.

Reset `currentReviewAudioTrackIds` alongside `currentReviewAudioKey` in `suspendReviewPlayback` and `closeReview`.

- [ ] **Step 6: Run the focused and full frontend suites**

```powershell
cargo test -p clipline-app --test player_core review_audio -- --nocapture
cargo test -p clipline-app --test ui_contract opening_multitrack_clip -- --nocapture
cargo test -p clipline-app --test player_core
cargo test -p clipline-app --test ui_contract
```

Expected: all commands PASS; opening code contains no preview invocation.

- [ ] **Step 7: Commit direct playback behavior**

```powershell
git add apps/clipline-app/tests/player_core.rs apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/player-core.js apps/clipline-app/ui/app-core.js apps/clipline-app/ui/review-player.js
git commit -m "fix(player): open clips with fallback audio immediately"
```

---

### Task 4: Pure Serialized Preview Queue

**Files:**
- Modify: `apps/clipline-app/tests/player_core.rs` after the review-audio tests
- Modify: `apps/clipline-app/ui/player-core.js` beside the audio selection helpers and in the export object

**Interfaces:**
- Consumes: request objects `{ clipPath, trackIds, selectionKey, sourceGeneration }`.
- Produces: `emptyAudioPreviewQueue()`, `queueAudioPreviewRequest(state, request)`, `cancelAudioPreviewRequest(state)`, and `finishAudioPreviewRequest(state, revision, succeeded)`.
- Queue state: `{ active: request|null, desired: request|null, revision: number }`; queued requests receive the next integer `revision`.
- Transition result: `{ state, start: request|null, apply: request|null }`.

- [ ] **Step 1: Write failing serialization and cancellation tests**

```rust
#[test]
fn audio_preview_queue_serializes_and_coalesces_to_latest_request() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(
            &mut ctx,
            r#"
            (() => {
              const one = { clipPath: 'a.mp4', trackIds: ['mic'], selectionKey: 'one', sourceGeneration: 4 };
              const two = { clipPath: 'a.mp4', trackIds: ['game'], selectionKey: 'two', sourceGeneration: 4 };
              const three = { clipPath: 'b.mp4', trackIds: ['output'], selectionKey: 'three', sourceGeneration: 5 };
              let state = PlayerCore.emptyAudioPreviewQueue();
              const first = PlayerCore.queueAudioPreviewRequest(state, one);
              state = first.state;
              state = PlayerCore.queueAudioPreviewRequest(state, two).state;
              state = PlayerCore.queueAudioPreviewRequest(state, three).state;
              const finished = PlayerCore.finishAudioPreviewRequest(state, first.start.revision, true);
              return {
                firstStart: first.start.selectionKey,
                firstApply: finished.apply,
                nextStart: finished.start.selectionKey,
                active: finished.state.active.selectionKey,
              };
            })()
            "#,
        ),
        r#"{"firstStart":"one","firstApply":null,"nextStart":"three","active":"three"}"#
    );
}

#[test]
fn audio_preview_queue_applies_only_current_success_and_cancel_keeps_worker_slot() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(
            &mut ctx,
            r#"
            (() => {
              const request = { clipPath: 'a.mp4', trackIds: ['mic'], selectionKey: 'mic', sourceGeneration: 2 };
              const queued = PlayerCore.queueAudioPreviewRequest(PlayerCore.emptyAudioPreviewQueue(), request);
              const applied = PlayerCore.finishAudioPreviewRequest(queued.state, queued.start.revision, true);
              const second = PlayerCore.queueAudioPreviewRequest(PlayerCore.emptyAudioPreviewQueue(), request);
              const cancelled = PlayerCore.cancelAudioPreviewRequest(second.state);
              const ignored = PlayerCore.finishAudioPreviewRequest(cancelled, second.start.revision, true);
              return {
                apply: applied.apply.selectionKey,
                activeAfterApply: applied.state.active,
                cancelledStillActive: cancelled.active.selectionKey,
                applyAfterCancel: ignored.apply,
              };
            })()
            "#,
        ),
        r#"{"apply":"mic","activeAfterApply":null,"cancelledStillActive":"mic","applyAfterCancel":null}"#
    );
}
```

- [ ] **Step 2: Run the queue tests and confirm they fail**

```powershell
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
```

Expected: FAIL because `emptyAudioPreviewQueue` is not defined.

- [ ] **Step 3: Implement the pure queue transitions**

```javascript
  const emptyAudioPreviewQueue = () => ({ active: null, desired: null, revision: 0 });

  const queueAudioPreviewRequest = (state, request) => {
    const revision = Number(state && state.revision || 0) + 1;
    const next = { ...request, revision };
    const active = state && state.active ? state.active : null;
    return {
      state: { active: active || next, desired: next, revision },
      start: active ? null : next,
      apply: null,
    };
  };

  const cancelAudioPreviewRequest = (state) => ({
    active: state && state.active ? state.active : null,
    desired: null,
    revision: Number(state && state.revision || 0) + 1,
  });

  const finishAudioPreviewRequest = (state, revision, succeeded) => {
    if (!state || !state.active || state.active.revision !== revision) {
      return { state, start: null, apply: null };
    }
    const desired = state.desired;
    const apply = succeeded && desired && desired.revision === revision ? state.active : null;
    const start = !apply && desired && desired.revision !== revision ? desired : null;
    return {
      state: { active: start, desired: start, revision: state.revision },
      start,
      apply,
    };
  };
```

Export all four functions.

- [ ] **Step 4: Run all pure-player tests**

```powershell
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test player_core
```

Expected: PASS.

- [ ] **Step 5: Commit the queue state machine**

```powershell
git add apps/clipline-app/tests/player_core.rs apps/clipline-app/ui/player-core.js
git commit -m "fix(player): serialize audio preview requests"
```

---

### Task 5: Preview Queue Browser Integration and Failure Recovery

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs:1685-1740,1848-1874,2746-2754`
- Modify: `apps/clipline-app/ui/app-core.js:138-143,279-281`
- Modify: `apps/clipline-app/ui/review-player.js:154-252,336-460`

**Interfaces:**
- Consumes: Task 4 queue transitions, Task 3 review-audio helpers, Task 2 logical source swaps.
- Produces: `runAudioPreviewRequest(request)`, `requestSelectedAudioPreview()`, `cancelDesiredAudioPreview()`, and `restoreAudibleAudioSelection(message)`.
- Tauri request adds `protectedPreviewPath`, containing `currentReviewMediaPath` when a preview is currently loaded.

- [ ] **Step 1: Write failing UI orchestration contracts**

```rust
#[test]
fn explicit_audio_preview_uses_one_pure_coalescing_queue() {
    let review = read_ui_js("review-player.js");
    assert!(review.contains("var audioPreviewQueue = PlayerCore.emptyAudioPreviewQueue();"));
    assert!(review.contains("PlayerCore.queueAudioPreviewRequest("));
    assert!(review.contains("PlayerCore.finishAudioPreviewRequest("));
    assert_eq!(review.matches("await invoke(\"preview_clip_audio_tracks\"").count(), 1);
    assert!(review.contains("protectedPreviewPath: currentReviewMediaPath"));
    assert!(!review.contains("audioPreviewSeq"));
}

#[test]
fn preview_failure_keeps_source_and_reverts_controls_to_audible_selection() {
    let review = read_ui_js("review-player.js");
    let restore = js_function_body(&review, "restoreAudibleAudioSelection");
    assert!(restore.contains("selectedAudioTrackIds = new Set(currentReviewAudioTrackIds);"));
    assert!(restore.contains("renderAudioTrackPanel();"));
    assert!(restore.contains("setDeckStatus(message, { transient: true });"));
    assert!(!restore.contains("setReviewVideoSource"));
}

#[test]
fn valid_preview_swap_reads_latest_player_state_after_await() {
    let review = read_ui_js("review-player.js");
    let run = js_function_body(&review, "runAudioPreviewRequest");
    let await_preview = run.find("await invoke(\"preview_clip_audio_tracks\"").unwrap();
    let latest_time = run[await_preview..].find("reviewPlayheadTime()").unwrap();
    let latest_pause = run[await_preview..].find("!video.paused && !video.ended").unwrap();
    let swap = run[await_preview..].find("setReviewVideoSource(path, {").unwrap();
    assert!(latest_time < swap);
    assert!(latest_pause < swap);
}
```

Update the rename contract to require `requestSelectedAudioPreview()` rather than awaiting the old monolithic preview function. Update suspend/close contracts to require `cancelDesiredAudioPreview()`.

- [ ] **Step 2: Run the contracts and confirm they fail**

```powershell
cargo test -p clipline-app --test ui_contract explicit_audio_preview -- --nocapture
cargo test -p clipline-app --test ui_contract preview_failure_keeps_source -- --nocapture
cargo test -p clipline-app --test ui_contract valid_preview_swap -- --nocapture
```

Expected: FAIL because the old sequence-number implementation starts each request immediately.

- [ ] **Step 3: Add queue state and audible-selection recovery**

Replace `audioPreviewSeq` with:

```javascript
var audioPreviewQueue = PlayerCore.emptyAudioPreviewQueue();
```

Add to `review-player.js`:

```javascript
function cancelDesiredAudioPreview() {
  audioPreviewQueue = PlayerCore.cancelAudioPreviewRequest(audioPreviewQueue);
}

function restoreAudibleAudioSelection(message) {
  selectedAudioTrackIds = new Set(currentReviewAudioTrackIds);
  renderAudioTrackPanel();
  setDeckStatus(message, { transient: true });
}

function previewRequestStillCurrent(request) {
  return Boolean(currentClip)
    && currentClip.path === request.clipPath
    && request.selectionKey === audioSelectionKey(currentClip)
    && request.sourceGeneration === reviewSourceGeneration;
}
```

On open, close, suspend, rename source release, and direct-fallback selection, call `cancelDesiredAudioPreview()` rather than discarding the active worker slot. This lets an already-running command finish but prevents it from applying.

- [ ] **Step 4: Implement one async preview runner**

```javascript
async function runAudioPreviewRequest(request) {
  let path = null;
  let error = null;
  try {
    path = await invoke("preview_clip_audio_tracks", {
      request: {
        path: request.clipPath,
        audioTrackIds: request.trackIds,
        protectedPreviewPath: currentReviewMediaPath,
      },
    });
  } catch (e) {
    error = String(e);
  }

  const transition = PlayerCore.finishAudioPreviewRequest(
    audioPreviewQueue,
    request.revision,
    error == null,
  );
  audioPreviewQueue = transition.state;

  if (transition.apply && previewRequestStillCurrent(transition.apply)) {
    const resumeTime = reviewPlayheadTime();
    const shouldResume = !video.paused && !video.ended;
    const rate = video.playbackRate;
    const trimRange = { start: trimStart, end: trimEnd };
    setReviewVideoSource(path, { resumeTime, shouldResume, rate, trimRange });
    currentReviewAudioTrackIds = [...transition.apply.trackIds];
    currentReviewAudioKey = transition.apply.selectionKey;
    setDeckStatus(audioSelectionLabel(currentClip), { transient: true });
  } else if (error && !transition.start && previewRequestStillCurrent(request)) {
    restoreAudibleAudioSelection(`audio preview failed: ${error}`);
  }

  if (transition.start) void runAudioPreviewRequest(transition.start);
}
```

The latest play/pause, rate, trim, and logical time must be read after `await`; do not retain request-start values.

- [ ] **Step 5: Dispatch explicit selection or restore direct playback**

```javascript
function requestSelectedAudioPreview() {
  const clip = currentClip;
  if (!clip) return;
  const tracks = clipAudioTracks(clip);
  const selected = selectedAudioTrackIdsForClip(clip);
  const selectionKey = audioSelectionKey(clip, selected);
  if (!PlayerCore.reviewSelectionNeedsPreview(tracks, selected)) {
    cancelDesiredAudioPreview();
    if (currentReviewMediaPath !== clip.path) {
      setReviewVideoSource(clip.path, {
        resumeTime: reviewPlayheadTime(),
        shouldResume: !video.paused && !video.ended,
        rate: video.playbackRate,
        trimRange: { start: trimStart, end: trimEnd },
      });
    }
    currentReviewAudioTrackIds = [...selected];
    currentReviewAudioKey = selectionKey;
    return;
  }
  if (selectionKey === currentReviewAudioKey) return;
  const queued = PlayerCore.queueAudioPreviewRequest(audioPreviewQueue, {
    clipPath: clip.path,
    trackIds: [...selected],
    selectionKey,
    sourceGeneration: reviewSourceGeneration,
  });
  audioPreviewQueue = queued.state;
  setDeckStatus("switching audio tracks...");
  if (queued.start) void runAudioPreviewRequest(queued.start);
}
```

Call this dispatcher from the review audio checkbox callback and after a current-file rename. Do not call it from `openClip`.

- [ ] **Step 6: Run frontend and app tests**

```powershell
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test ui_contract audio_preview -- --nocapture
cargo test -p clipline-app --test ui_contract preview_failure -- --nocapture
cargo test -p clipline-app
```

Expected: PASS. The structural test confirms only one invocation site exists and all repeated changes enter the pure queue.

- [ ] **Step 7: Commit preview orchestration**

```powershell
git add apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/app-core.js apps/clipline-app/ui/review-player.js
git commit -m "fix(player): coalesce explicit audio previews"
```

---

### Task 6: Bounded Atomic Preview Cache

**Files:**
- Modify: `apps/clipline-app/src/library.rs:146-151,672-795,1090-1110,1650-end`
- Modify: `apps/clipline-app/src/app.rs:1633-1636,1724-1769`
- Modify: `apps/clipline-app/tests/ui_contract.rs:395-404,808-816`

**Interfaces:**
- Consumes: `AudioPreviewRequest.protected_preview_path: Option<String>` from Task 5.
- Produces: `pub(crate) fn prune_audio_preview_cache_on_startup() -> Result<AudioPreviewPruneReport, String>` and private `prune_audio_preview_cache(dir, protected, max_bytes)`.
- Constant: `AUDIO_PREVIEW_CACHE_MAX_BYTES = 2 * 1024 * 1024 * 1024`.
- Report shape: `{ removed_files: usize, removed_bytes: u64, reusable_bytes: u64 }`.

- [ ] **Step 1: Write failing filesystem policy tests**

```rust
#[test]
fn audio_preview_cache_prunes_lru_and_partials_but_preserves_protected_file() {
    let dir = TestDir::new("clipline-library", "audio-preview-cache-lru");
    let oldest = dir.path().join("audio-preview-0001.mp4");
    let newest = dir.path().join("audio-preview-0002.mp4");
    let protected = dir.path().join("audio-preview-0003.mp4");
    let partial = dir.path().join("audio-preview-0004.mp4.1.2.tmp");
    std::fs::write(&oldest, [0_u8; 6]).unwrap();
    std::fs::write(&newest, [0_u8; 6]).unwrap();
    std::fs::write(&protected, [0_u8; 20]).unwrap();
    std::fs::write(&partial, [0_u8; 3]).unwrap();
    std::fs::File::options().write(true).open(&oldest).unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)).unwrap();
    std::fs::File::options().write(true).open(&newest).unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(2)).unwrap();

    let report = prune_audio_preview_cache(
        dir.path(),
        std::slice::from_ref(&protected),
        6,
    ).unwrap();

    assert!(!oldest.exists());
    assert!(newest.exists());
    assert!(protected.exists());
    assert!(!partial.exists());
    assert_eq!(report.reusable_bytes, 6);
}

#[test]
fn audio_preview_write_is_atomic_and_leaves_no_partial() {
    let dir = TestDir::new("clipline-library", "audio-preview-atomic");
    let target = dir.path().join("audio-preview-abcd.mp4");
    write_audio_preview(&target, vec![1, 2, 3, 4]).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), vec![1, 2, 3, 4]);
    assert!(std::fs::read_dir(dir.path()).unwrap().flatten().all(|entry| {
        !entry.file_name().to_string_lossy().ends_with(".tmp")
    }));
}

#[test]
fn audio_preview_cache_hit_refreshes_recency() {
    let dir = TestDir::new("clipline-library", "audio-preview-cache-touch");
    let preview = dir.path().join("audio-preview-abcd.mp4");
    std::fs::write(&preview, b"preview").unwrap();
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
    std::fs::File::options().write(true).open(&preview).unwrap()
        .set_modified(old).unwrap();

    touch_audio_preview(&preview).unwrap();

    assert!(std::fs::metadata(&preview).unwrap().modified().unwrap() > old);
}
```

Add the structural contract:

```rust
#[test]
fn audio_preview_command_protects_active_media_and_prunes_cache_on_startup() {
    let library = library_rs();
    let app = app_rs();
    assert!(library.contains("pub protected_preview_path: Option<String>"));
    assert!(library.contains("prune_audio_preview_cache("));
    assert!(library.contains("touch_audio_preview(&preview)"));
    assert!(app.contains("crate::library::prune_audio_preview_cache_on_startup()"));
}
```

- [ ] **Step 2: Run focused tests and confirm the expected failure**

```powershell
cargo test -p clipline-app audio_preview_cache_prunes -- --nocapture
cargo test -p clipline-app --test ui_contract audio_preview_command -- --nocapture
```

Expected: FAIL because `prune_audio_preview_cache` and `protected_preview_path` do not exist.

- [ ] **Step 3: Add the request field, limit, and cache report**

```rust
const AUDIO_PREVIEW_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioPreviewPruneReport {
    removed_files: usize,
    removed_bytes: u64,
    reusable_bytes: u64,
}

#[derive(serde::Deserialize)]
pub struct AudioPreviewRequest {
    pub path: String,
    #[serde(default, rename = "audioTrackIds")]
    pub audio_track_ids: Vec<String>,
    #[serde(default, rename = "protectedPreviewPath")]
    pub protected_preview_path: Option<String>,
}
```

Extract `protected_preview_path` before moving the request into `spawn_blocking` and pass it through `preview_clip_audio_tracks_file` to the mixer-aware helper.

- [ ] **Step 4: Implement protected-path-aware LRU pruning**

```rust
fn is_audio_preview_mp4(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
        name.starts_with("audio-preview-") && name.ends_with(".mp4")
    })
}

fn is_audio_preview_partial(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
        name.starts_with("audio-preview-") && name.ends_with(".tmp")
    })
}

#[derive(Debug)]
struct CachedAudioPreview {
    path: PathBuf,
    len: u64,
    modified: std::time::SystemTime,
}

fn audio_preview_path_is_protected(path: &Path, protected: &[PathBuf]) -> bool {
    protected.iter().any(|candidate| {
        path == candidate
            || std::fs::canonicalize(path).ok().zip(std::fs::canonicalize(candidate).ok())
                .is_some_and(|(left, right)| left == right)
    })
}

fn prune_audio_preview_cache(
    dir: &Path,
    protected: &[PathBuf],
    max_bytes: u64,
) -> Result<AudioPreviewPruneReport, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(format!("read audio preview cache {dir:?}: {error}")),
    };
    let mut report = AudioPreviewPruneReport::default();
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("read audio preview cache entry: {error}"))?;
        let path = entry.path();
        if is_audio_preview_partial(&path) {
            let len = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            if std::fs::remove_file(&path).is_ok() {
                report.removed_files += 1;
                report.removed_bytes = report.removed_bytes.saturating_add(len);
            }
            continue;
        }
        if !is_audio_preview_mp4(&path) || audio_preview_path_is_protected(&path, protected) {
            continue;
        }
        let metadata = entry.metadata()
            .map_err(|error| format!("read audio preview metadata {path:?}: {error}"))?;
        let len = metadata.len();
        report.reusable_bytes = report.reusable_bytes.saturating_add(len);
        candidates.push(CachedAudioPreview {
            path,
            len,
            modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
        });
    }
    candidates.sort_by(|left, right| {
        left.modified.cmp(&right.modified).then_with(|| left.path.cmp(&right.path))
    });
    for candidate in candidates {
        if report.reusable_bytes <= max_bytes {
            break;
        }
        if std::fs::remove_file(&candidate.path).is_ok() {
            report.removed_files += 1;
            report.removed_bytes = report.removed_bytes.saturating_add(candidate.len);
            report.reusable_bytes = report.reusable_bytes.saturating_sub(candidate.len);
        }
    }
    Ok(report)
}
```

The pruning function must:

1. return an empty report when the directory does not exist;
2. delete every matching partial file best-effort;
3. gather matching MP4 length and modified time;
4. exclude protected paths from `reusable_bytes` and eviction candidates;
5. sort eviction candidates by modified time and then path;
6. remove oldest candidates until `reusable_bytes <= max_bytes`;
7. return filesystem enumeration or metadata errors with path context.

Add this cache-hit helper. The preview remains usable when it fails: log the error and continue
playback.

```rust
fn touch_audio_preview(path: &Path) -> Result<(), String> {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()))
        .map_err(|error| format!("refresh audio preview recency {path:?}: {error}"))
}
```

- [ ] **Step 5: Prune before and after generation without deleting active files**

Before cache lookup, call pruning with the request's current protected preview. On cache hit, touch the result and prune with both the current protected path and the hit protected. After native or FFmpeg generation, prune with both the previously active preview and the generated result protected.

Keep `write_audio_preview` on `cached_export_tmp_path`, which already produces unique sibling names and atomically renames. Ensure all error branches remove their own temporary output.

Delete `prune_old_audio_previews`; keep age-based share-export pruning unchanged.

- [ ] **Step 6: Add best-effort startup cleanup**

```rust
pub(crate) fn prune_audio_preview_cache_on_startup() -> Result<AudioPreviewPruneReport, String> {
    let dir = crate::settings::audio_preview_cache_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create audio preview cache {dir:?}: {e}"))?;
    prune_audio_preview_cache(&dir, &[], AUDIO_PREVIEW_CACHE_MAX_BYTES)
}
```

Call near the existing audio-preview directory setup in `app.rs`:

```rust
if let Err(e) = crate::library::prune_audio_preview_cache_on_startup() {
    eprintln!("could not prune audio preview cache on startup: {e}");
}
```

The failure is logged and startup continues.

- [ ] **Step 7: Run cache, app, and contract tests**

```powershell
cargo test -p clipline-app audio_preview_cache -- --nocapture
cargo test -p clipline-app audio_preview_write_is_atomic -- --nocapture
cargo test -p clipline-app audio_preview_cache_hit_refreshes_recency -- --nocapture
cargo test -p clipline-app --test ui_contract audio_preview_command -- --nocapture
cargo test -p clipline-app
```

Expected: PASS. Tests use byte-sized fixtures; no large media file is created.

- [ ] **Step 8: Commit bounded cache behavior**

```powershell
git add apps/clipline-app/src/library.rs apps/clipline-app/src/app.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "fix(player): bound the audio preview cache"
```

---

### Task 7: Full Verification, Handoff, and Manual Reproduction

**Files:**
- Modify: `handoff.md`

**Interfaces:**
- Consumes: all prior tasks.
- Produces: workspace-wide quality evidence and a concise manual test handoff.

- [ ] **Step 1: Run focused regression suites together**

```powershell
cargo test -p clipline-app --test player_core logical_seek -- --nocapture
cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture
cargo test -p clipline-app --test player_core review_audio -- --nocapture
cargo test -p clipline-app --test ui_contract audio_preview -- --nocapture
cargo test -p clipline-app audio_preview_cache -- --nocapture
```

Expected: every focused regression PASS.

- [ ] **Step 2: Run workspace tests**

```powershell
cargo test --workspace
```

Expected: PASS with zero failed tests; hardware tests may self-skip normally.

- [ ] **Step 3: Run a fresh-cache clippy check for the changed app crate**

```powershell
cargo clean -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 4: Run the workspace lint gate**

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with zero warnings.

- [ ] **Step 5: Stop the existing development app, launch the changed build, and reproduce manually**

Stop only Clipline processes whose executable path resolves under
`C:\Users\dain\Projects\clipline` or the active Paseo worktree, then run:

```powershell
cargo run -p clipline-app
```

With a large multi-track clip:

1. open and switch clips and verify no preview cache file appears automatically;
2. spam the five-second forward button and the configured five-second keyboard shortcut across several timeline events;
3. verify neither the visible playhead nor actual playback lands at zero unless zero is requested;
4. explicitly select microphone/process audio and verify original playback continues during preview generation;
5. keep seeking while the preview finishes and verify the swap lands at the latest logical target;
6. rapidly change audio selections and verify only the newest selection becomes audible;
7. restart once and verify retained preview data is at or below 2 GiB unless one active preview alone is larger.

Expected: all seven observations match the design, and general playback remains responsive when browsing clips.

- [ ] **Step 6: Update the handoff document**

Add a dated entry to `handoff.md` containing these facts:

```markdown
- Rapid relative seeks now retain an authoritative logical target until the current source confirms arrival within 100 ms; early/stale `seeked` events cannot clear it.
- Multi-track clips open directly with their embedded fallback audio. Audio preview generation begins only after an explicit non-fallback selection and remains serialized/coalesced.
- Reusable audio previews are LRU-capped at 2 GiB; the active preview is protected and may temporarily exceed the cap when it alone is oversized.
- Preview jobs already executing in `spawn_blocking` remain non-cancellable, but automatic and concurrent fan-out has been removed.
```

Include the final workspace test count, clippy result, and Step 5 manual test outcome.

- [ ] **Step 7: Commit the handoff update**

```powershell
git add handoff.md
git commit -m "docs: update resilient playback handoff"
```

- [ ] **Step 8: Inspect final scope and history**

```powershell
git status --short
git log --oneline --decorate -8
git diff b049255c..HEAD --stat
```

Expected: the implementation worktree is clean; history contains one logical commit per task; the diff contains only the mapped implementation, tests, and handoff files.
