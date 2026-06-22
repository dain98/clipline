# Clipline Bug Scan

**Date:** 2026-06-21
**Scope:** Full workspace — ~29K lines Rust (`apps/clipline-app`, `crates/*`) + ~5.5K lines JS frontend (`apps/clipline-app/ui`).
**Method:** 8 parallel deep-review passes, one per subsystem, focused on correctness/safety bugs (not style). The top-priority items were spot-verified against source.

**Overall:** The codebase is carefully written — extensive bounds-checking, checked arithmetic, disciplined `unsafe`, and many candidate issues were confirmed as *non*-bugs. The findings below are the genuine ones, prioritized by impact on realistic inputs.

Severity legend: 🔴 Critical · 🟠 High · 🟡 Medium · ⚪ Lower-confidence / worth a look.

---

## 🔴 Top priority — silent data loss / corruption / bricking

### 1. Recorder can be permanently bricked by the game-detection thread
- **Where:** `apps/clipline-app/src/app.rs:351-357` (`set_detected_game`); same pattern at `app.rs:267-276` (`restart`).
- **Status:** Verified against source.
- **What:** `inner.tx.take()` removes the recorder sender *before* the fallible `Self::options(&inner)?` runs. If `options()` returns `Err` (it can — `to_service_options` → replay-cache-dir normalization can fail, e.g. a drive briefly unavailable), `?` returns early leaving `inner.tx == None`. The old recorder is never restarted and the new one is never spawned.
- **Why it's real:** `set_detected_game` is driven by the 2-second game-detection thread. Recording silently dies with only a one-shot toast and stays dead — the next same-window tick takes the `(None, None, false)` branch — until the app restarts or the user manually toggles recording.
- **Fix:** Build `next_options` *before* `inner.tx.take()`; only take the sender once options is `Ok`. (Or restore `inner.tx` on the error path.)

### 2. Settings saved non-atomically → total settings loss on a crash
- **Where:** `apps/clipline-app/src/settings.rs:876` (`save_to`).
- **Status:** Verified against source.
- **What:** `std::fs::write(path, json)` truncates then writes in place — no temp-file + rename, no backup.
- **Why it's real:** `update_cloud` calls `save()` on **every upload-progress step**, so writes are frequent and concurrent with long uploads — exactly when a crash/power-loss is likely. A partial write makes `load_from` fail to parse, and `load_or_default()` silently resets *all* settings (media dir, hotkey, cloud connection, custom games) to defaults.
- **Fix:** Write `settings.json.tmp`, `sync_all`, then atomic `rename`. The repo already does temp+rename for posters (`poster.rs:93`) and audio previews (`library.rs:455`) — apply the same here.

### 3. MP4 duration truncated to `u32` → broken files past ~13 h
- **Where:** `crates/clipline-mp4/src/init.rs:157,242,269`; `crates/clipline-mp4/src/writer.rs:297,484`.
- **What:** `mvhd`/`tkhd`/`mdhd` durations are computed as `u64` but written `as u32` (version-0 boxes). A 90 kHz video track overflows `u32` at ~13.25 h; audio (timescale = sample rate) at ~24.8 h. On overflow the declared duration wraps — players show the wrong length and seeking breaks. Separately, `duration_movie_ts()` multiplies before dividing and truncates, so video/audio track durations can disagree by a tick.
- **Fix:** Emit version-1 (64-bit) box layouts when the value exceeds `u32::MAX`; use a `u128` intermediate for the movie-duration rescale.

### 4. `update_cloud` holds a std `Mutex` across blocking disk I/O on the async runtime
- **Where:** `apps/clipline-app/src/app.rs:241-254` (called from async upload commands, e.g. `cloud.rs:560` `persist_record`).
- **What:** Takes `self.0.lock()` then calls `next.save()` (serialize + `create_dir_all` + `fs::write`) while holding it, on a Tauri async thread.
- **Why it's real:** `persist_record` is called repeatedly *during* uploads; `cloud_status`/`active_shortcut_matches` also lock `self.0`. A slow disk blocks every other state access and starves the runtime.
- **Fix:** Clone under the lock, drop it, then `save()` — or `spawn_blocking`.

---

## 🟠 High

### Capture — video
- **`crates/clipline-capture/src/windows/wgc.rs:278-279` — `?` inside the FrameArrived handler can permanently stall capture.** A transient COM failure (`SystemRelativeTime`/`ContentSize`/`GetInterface`, or pool `Recreate` during a resize) returns `Err` from the WinRT event callback, which is then not re-armed; frames stop and `next_frame` times out forever. Other per-frame failures are correctly swallowed with `let Ok(..) else { return Ok(()) }` — these four aren't. **Fix:** log + `return Ok(())` instead of `?`.
- **`crates/clipline-capture/src/windows/nv12.rs:327-363` — `read_nv12` UV-plane read isn't bounds-validated against the mapped buffer.** `nv12_layout_fits` validates only the Y plane against `DepthPitch`; the UV span (assumed at `pitch*height`) is computed but never checked against the real mapped extent, then read via `from_raw_parts`. If a driver places UV elsewhere or the allocation is smaller, this is an OOB read (FFmpeg readback path). **Fix:** validate the full UV span against `depth_pitch`.

### Buffer
- **`crates/clipline-buffer/src/segment.rs:22-26,124-128` — `sample_slices()` panics on corrupt disk-reloaded segments.** `DiskSegment::load()` validates only aggregate length, not that per-sample sizes still tile the reloaded bytes; the convenience `sample_slices()` then `.expect()`s and panics the capture thread on a truncated/corrupt cache file. A checked variant (`sample_slices_checked`) exists but isn't used on the hot path. **Fix:** use the checked variant on save/load and propagate as `InvalidData`.

### App backend — security
- **`apps/clipline-app/src/cloud.rs:633` — cloud bearer token stored plaintext with `CRED_PERSIST_LOCAL_MACHINE` + fixed target name**, readable by any process running as the same user; never re-validated/refreshed. **Fix:** reconsider persistence scope; treat as a per-user secret.

### LoL markers
- **`apps/clipline-app/src/markers.rs:75` + `crates/clipline-lol/src/tracker.rs:13-23` — a transient API error mid-match is misclassified as game-end, then re-emits every event as duplicate markers.** `Err(_) => break` collapses timeouts/404s/oversized-body into "game ended"; the outer loop then rebuilds a fresh `EventTracker` (watermark reset to `None`) so `fresh()` re-surfaces the entire backlog. The LoL Live Client API is known for sporadic mid-game errors. **Fix:** distinguish transient errors (retry in place) from real game-end; persist the tracker across reconnects.

### Frontend
- **`apps/clipline-app/ui/main.js:2413-2434` + `player-core.js:414-418` — split-output clips open with playback that does not match the selected-track UI.** `defaultAudioTrackIds()` intentionally omits the mixed `output` fallback when process-output tracks are present, but `openClip()` still points the video at the original MP4 and never applies that effective selection. A split-output clip can therefore play the browser's default/all source audio while the panel says only process/mic tracks are selected. **Fix:** keep normal clips lazy, but eagerly build/switch to an audio preview when the default effective selection excludes the mixed fallback.
- **`apps/clipline-app/ui/main.js:2409-2434` — `openClip` doesn't cancel the previous clip's RAF or `pendingSeek`.** Clicking clip B while A is mid-scrub can apply A's pending seek to B / leave A's playhead loop running. Only `closeReview` tears these down. **Fix:** `cancelAnimationFrame(rafId); pendingSeek = null;` at the top of `openClip`.
- **`apps/clipline-app/ui/player-core.js:500-505` — `clipKind` classifies by mutable filename.** A user-renamed clip containing `session_`/`_trim_` gets the wrong gallery filter/icon/title (and can trigger KDA-title substitution). **Fix:** use a backend-provided `kind` field.
- **`apps/clipline-app/ui/main.js:2801` and `player-core.js:362-376` — `seekTo`/`nextMarker`/`prevMarker` can feed `NaN` to `video.currentTime`** from a `.markers.json` marker missing `t_s`, stalling seeking; `renderMarkers` (`main.js:2730`, `m.t_s.toFixed(1)`) throws on the same input and aborts the whole marker render. **Fix:** guard `Number.isFinite(t)`.
- **`apps/clipline-app/ui/main.js:3695-3705` — audio-track switch can shift the trim out-point.** The persistent `loadedmetadata` handler unconditionally re-runs `setTrim(0, video.duration)` on the re-muxed track file (whose duration can differ by re-encode rounding), racing the `{once:true}` trim-restore. **Fix:** suppress the reset path during an audio-preview swap.

---

## 🟡 Medium

### Audio path
- **`crates/clipline-capture/src/pcm.rs:31-43`** — `LoopbackAssembler` ignores **negative** gaps (backward QPC after a discontinuity), concatenating audio that should overlap → audio drifts ahead of video over long recordings. Pair with **`windows/wasapi.rs:413`** ignoring `AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY`.
- **`crates/clipline-capture/src/windows/wasapi.rs:204-222`** — per-process loopback `Initialize` with `buffer_duration_100ns = 0` → ~10 ms device buffer; at 30 fps poll cadence (~33 ms) it can overflow and drop process audio. (The endpoint path correctly passes 1 s.)
- **`crates/clipline-capture/src/pcm.rs:70-79`** — `extract_mono_centered` divides by total channel count, so a single-sided stereo mic comes out −6 dB.
- **`crates/clipline-capture/src/pcm.rs:210-244`** — `pop_mixed_frames` can emit silence for a briefly-stalled source, then **discard** that source's real (late) frame for the same grid slot → permanent gap, not just delay.
- **`crates/clipline-capture/src/ffmpeg_encoder.rs:209-218`** — synthesized-pts fallback can go non-monotonic vs. a later real pts → garbage frame durations.

### MP4
- **`crates/clipline-mp4/src/trim.rs:561-603`** — `parse_avcc`/`parse_hvcc` keep only the *first* SPS/PPS (and VPS) even when the stream declares multiple; frames referencing other parameter sets fail to decode after trim. `init.rs` `avcc`/`hvcc` builders hardcode one of each too.
- **`crates/clipline-mp4/src/trim.rs:83-123`** — audio selected by `start >= aligned_start && start < aligned_end` drops the packet straddling the cut and resets `base_decode_time = 0`, so audio/video aren't co-started at the cut (A/V offset drift). Range selection is done in `f64` seconds rather than integer ticks (`trim.rs:331-336`).
- **`crates/clipline-mp4/src/trim.rs:684-686`** — fixed-size `stsz` path allows a crafted small file to force a large `vec![sample_size; count]` pre-allocation (memory amplification).

### App backend
- **`apps/clipline-app/src/service.rs:1464`** — marker sidecar written with plain `fs::write` (non-atomic); the library scan can read a half-written `.markers.json`. (Poster/preview already use temp+rename.)
- **`apps/clipline-app/src/app.rs:312,327`** — `stop_recording` returns without joining the recorder thread; a fast Stop→Start can race for the WGC/D3D11/WASAPI device → transient `init:` errors.
- **`apps/clipline-app/src/settings.rs:897-900`** — replay-cache/media-folder overlap validation is case-sensitive on Windows. `same_normalized_path()` handles case-insensitive equality, but `same_or_nested_path()` uses raw `PathBuf::starts_with`. A cache path like `c:\videos\clipline\cache` can be accepted under media `C:\Videos\Clipline`. **Fix:** normalize Windows path comparisons case-insensitively for both equality and nesting.
- **`apps/clipline-app/src/cloud_upload.rs:36`** — `reqwest::Client::new()` has **no timeouts**; a half-open connection hangs an upload forever (the 3-attempt retry only fires on returned errors, not hangs).
- **`apps/clipline-app/src/cloud.rs:200` and `cloud_upload.rs:194,213,370`** — whole clip read into RAM then `.to_vec()`-copied per part; a multi-GB session clip causes 2-3× memory spikes / possible OOM (defeats the memory-pressure monitor). **Fix:** stream from disk.
- **`apps/clipline-app/src/cloud.rs:284-335` + `main.js:2200,2230`** — uploads can remain locally stuck in `processing` after the fixed ready-poll window expires. `wait_for_ready_clip()` returns `Ok(None)` after 30 seconds, but the persisted record is left as `processing`; the UI treats that status as busy and disables retry/upload controls. **Fix:** persist a retryable non-busy timeout state or a failed state with the remote URL preserved.
- **`apps/clipline-app/src/cloud.rs:329-331`** — `delete_local_after_upload` removes the clip and marker sidecar but leaves the cached poster. Normal `delete_clip()` removes the poster too. **Fix:** remove `poster_path(&target)` on cloud-delete cleanup.
- **`apps/clipline-app/src/memory.rs:30-34,111-122`** — memory monitor silently undercounts when a child process can't be `OpenProcess`'d (returns `None` → omitted), so pressure decisions act on understated totals.
- **`apps/clipline-app/src/cloud.rs:518-536`** — `local_clip_id` derived from *post-remux* (audio-selection-dependent) checksum, so it's unstable per source clip; dedup/retry leans on a path fallback.

### Buffer / storage / events
- **`crates/clipline-buffer/src/disk.rs:48-68`** — `push` orphans `seg_*.tmp` on write failure (e.g. disk full); nothing ever sweeps them, and the storage GC only knows about `.mp4` files → unbounded growth.
- **`crates/clipline-buffer/src/disk.rs:82-90`** — an eviction `remove_file` error (AV lock / EACCES, common on Windows) makes a *successful* insert report `Err` and aborts before getting back under budget.
- **`crates/clipline-events/src/markers.rs:81-100`** — `clip_markers` computes `duration_s = end - start` with no `end >= start` guard → negative duration serialized to the sidecar on inverted/zero windows.
- **`crates/clipline-storage/src/lib.rs:298-303`** — `same_path` falls back to raw `PathBuf` equality when `canonicalize` fails; on Windows the two `Ok` arms can also disagree (`\\?\` form vs `read_dir` form), risking GC-deleting the just-saved clip. **Fix:** fail-safe (treat as protected) on canonicalize failure.
- **`crates/clipline-buffer/src/ring.rs:74-90` / `disk.rs:126-140`** — smart-mode `save_window` GOP-boundary edge can drop unsaved footage or re-include saved footage at the `exclude_before_s` boundary when a GOP straddles the exclusion point (forward-skip to next keyframe instead of realigning to GOP start). *Lower confidence — add a targeted test with a multi-segment GOP straddling the cut.*

---

## ⚪ Lower-confidence / worth a look
- **`crates/clipline-capture/src/windows/mft.rs:296-312`** — `MF_E_TRANSFORM_STREAM_CHANGE` retry loop has no iteration cap (theoretical infinite loop). **`mft.rs:331`** — `track_config()` can ship empty SPS/PPS → undecodable MP4 instead of an error.
- **`crates/clipline-capture/src/windows/d3d11.rs`** — immediate-context used cross-thread; only sound because `SetMultithreadProtected(true)` is set on device creation. Worth an assertion if a device is ever supplied externally.
- **`crates/clipline-capture/src/clock.rs:26`** — `qpc_to_ticks_100ns` truncates instead of rounding (monotone drift bias on non-10 MHz QPC).
- **`apps/clipline-app/ui/main.js:3907`** — `memory_status` polled every 2 s with no `visibilitychange` gating (fires while in tray).
- **`crates/clipline-lol/src/normalize.rs:54-58`** — `Multikill`/`Ace` involves-local-player gate fails when the payload omits `KillerName` → highest-value clip moments lose their priority boost.
- **`crates/clipline-lol/src/client.rs:48`** — 2 s request timeout is aggressive under recording load and amplifies the false-game-end bug above.
- **`crates/clipline-mp4/src/init.rs:322`** — `sample_rate << 16` truncates the Opus sample-rate 16.16 field for rates ≥ 65536 (cosmetic; `dOps` is authoritative).
- **`apps/clipline-app/src/hotkeys.rs:51-63`** — save hotkey uses `try_lock` and silently drops the press on contention (rare; acceptable for an LL hook).

---

## Suggested order of attack
1. **#1 (recorder bricking)** and **#2 (atomic settings write)** — small, surgical fixes for silent failures that hit real users.
2. **#3 (u32 duration)** — if long recordings are a supported use case.
3. **wgc.rs `?`-in-handler** and **LoL transient-error misclassification** — small classification fixes preventing stalls and duplicate-marker storms.
4. The frontend cluster (`openClip` teardown, `clipKind`, `NaN` seek guard) — cheap and user-visible.

---

## Verified non-bugs (checked, not issues)
A sampling of things explicitly reviewed and cleared, to avoid re-litigating:
- MP4 box readers bounds-check via `.get()` and `bounded_table_count`; `box_end`/`read_box_at` validate against `input.len()`.
- `read_credential` reads the freed pointer *before* `CredFree` (named guard) — not a use-after-free (`cloud.rs:651-659`).
- No `std::sync::Mutex` is held across `.await` in async Tauri commands (except the `update_cloud` blocking-I/O case, #4).
- Settings loading is repair-or-default per field; unknown enums fall back rather than failing the whole parse.
- `game_plugins::plugin_icon_cache_path` rejects `/`, `\`, `.` in the id (no path traversal).
- `gallerySmartGroups` "Today" boundary math is correct across timezones; clipboard handle ownership is correct on all error paths.
- Marker windows are consistently half-open `[start, end)` across backend and frontend.
