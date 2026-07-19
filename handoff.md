# Clipline — Development Handoff

> For a fresh Claude Code session (or human) continuing this project.
> **`ddoc.md` is the single source of truth** for product/architecture decisions. This file is
> the bridge: where the project stands, how it's built, what bit us, and what's next.

## Checkpoint (2026-07-18): elevated-game Save Replay hotkeys

An Arknights: Endfield report said Save Replay worked only after tabbing out. The reporter's UAC
prompt identifies the boundary: Endfield runs elevated while Clipline normally runs at medium
integrity, so Windows UIPI prevents Clipline's low-level keyboard hook from observing input aimed
at the focused game. Running Clipline as administrator was confirmed as the user workaround.

Clipline remains `asInvoker` by default. Game-detection events now query the detected process token
through safe Win32 wrappers and flag the blocked state only when the game is elevated above
Clipline. The frontend shows one in-app explanation per game PID and offers an explicit Restart as
Administrator action, warning that the rolling buffer resets. Acceptance launches the same
executable through the `runas` verb with the current PID; the elevated child waits for the normal
instance to exit before starting Tauri, avoiding overlapping recorders and the single-instance
race. Clipline exits only after Windows successfully creates the replacement, so a denied or
cancelled UAC request leaves it running normally. Future launches remain non-elevated.

Focused elevation/Win32/UI tests, CI-mode `cargo test --workspace`, fresh-cache workspace clippy
with warnings denied, formatting, and diff checks pass. Computer Use could not attach because its
native pipe returned OS error 2. A live UAC attempt timed out without approval and verified the
normal PID remained alive with no replacement; accepting UAC and visually confirming the elevated
replacement/dialog remain the final native checks.

PR #87 review hardened the handoff further: only a confirmed-gone parent may skip the wait,
handoff failures abort before Tauri starts, protected-process token query failures warn
conservatively, and the frontend retries queued warnings while closing stale ones. Later
passes keep the elevation dialog open after UAC cancellation, block dismiss/Escape while the
restart is in flight, restore the warned PID if the dialog closed during that wait, reconcile
the dialog after in-flight clears (so a game that exited during UAC cannot leave a stale
modal), and re-enable controls when restart returns false.

## Checkpoint (2026-07-18): Nightly 0.1.35

Nightly 0.1.35 contains PR #86. It ships the Proxmox/Windows VM software H.264 fallback,
active-encoder status, safer Discord/output-audio defaults, long-session capture-cadence fixes,
and mixed-output selection preservation. The previous public nightly was 0.1.34, so the app and
Tauri versions were bumped for updater delivery. The standalone installer also advances its
pinned Microsoft WebView2 Fixed Version Runtime patch from 150.0.4078.48 to 150.0.4078.83.

## Checkpoint (2026-07-18): long-session burst timestamp fix

A 0.1.34 user report described long VOD playback occasionally jumping to 00:00 after an
arbitrary seek. The supplied `session_1783827199.markers.json` is internally consistent: 91
ordered, unique, in-range markers over 2022.944 seconds with a constant recording offset. The
matching 2,103,075,867-byte MP4 downloaded with SHA-256
`4A1DB0A25A8435443F7238D9985090D764407694C5BA52EA361F2412D2F68BAA`. FFprobe accepts its H.264
video and two Opus tracks, every video packet timestamp is strictly increasing, all sampled seeks
from 60 through 2000 seconds land on the expected preceding keyframe, the maximum keyframe gap is
0.65 seconds, and a full 33:43 video/audio decode completes without codec errors. Markers,
keyframes, sample indexes, and bitstream corruption are therefore ruled out for this artifact.

The artifact did expose a reproducible recorder defect. It contains 1,265 consecutive video-frame
gaps below one millisecond, all exactly 0.1 ms; several cluster around the reported 15-minute area.
`CadencedCapture` emitted a scheduled duplicate when WGC timed out, then accepted a real frame
whose presentation timestamp still belonged to that filled cadence slot and forced it to
`last_pts + 0.0001`. This produced extra near-zero-duration samples and an average frame rate above
the configured 60 FPS. `CadencedCapture` now retains an early real frame as the latest texture and
yields a bounded timeout to the service loop before reading again, so save/stop handling stays
responsive while a stale WGC queue drains. Its retry budget preserves the existing wall-clock
deadline; successful real frames advance the same wall anchor by their PTS delta; and overloaded
conversion/encoding skips missed cadence slots instead of letting video PTS drift behind wall time
and audio. Six focused tests cover idle duplication, stale-frame yielding/data reuse, delayed WGC
delivery, and time spent in the encoder between capture calls.

This timing defect is a plausible WebView2 stressor, especially because the supplied file has a
1.48 MB tail `moov` and Clipline plays it through Tauri's range-based asset protocol, but the exact
seek-to-zero chain is not yet proven. Computer Use could not attach in the final reproduction pass
because this thread's native pipe returned OS error 2. Do not claim the player reset itself was
visually reproduced or fully fixed until a fresh native session exercises this artifact. The
validated file is hard-linked without an extra 2 GB copy at
`C:\Users\dain9\Videos\Clipline\Imported seek repro 1783827199\session_1783827199.mp4`.

The bounded PR #86 review stopped cleanly after pass 3. It also fixed the split-audio helper that
normalized the new `output + microphone` default into microphone-only output. Review-fix commits:
`56f2339 docs: plan PR 86 review fixes`, `97dbd79 fix(capture): yield while dropping stale frames`,
`42a2744 fix(player): preserve mixed output selection`, and
`12201c3 fix(capture): keep cadence aligned with wall clock`.

Focused tests, the CI-mode full workspace suite, fresh-cache workspace clippy with warnings denied,
formatting, and diff checks pass. The unchanged live
`captures_monotonic_gpu_frames_from_primary_monitor` device test timed out twice waiting for a
desktop update after the app was stopped; other live WGC tests passed. Treat that as an environment
signal to rerun with an actively changing desktop, not as validation of this cadence patch.

## Checkpoint (2026-07-17): Discord audio safety-track default

A user report that Discord stopped recording after a recent update was reproduced as a playback-
selection regression, not loss from the mixed speaker capture. With Experimental app audio tracks
enabled, Clipline enumerates process audio sessions only when the recorder starts. A native
`ffplay` process started afterward was absent from the per-process marker metadata but remained
audible in the mixed Output Audio safety track. In the final five seconds of
`C:\Users\dain9\Videos\Clipline\2026-07-17 15-52\clip_1784329112.mp4`, mixed output measured
-33.1 dB mean/-30.0 dB peak while the stale startup Media Player track measured -91.0 dB
mean/-84.3 dB peak.

Nightly 0.1.34 commit `dc7250e` changed clip opening to prepare every default audio track. The
existing split-track default excluded mixed Output Audio whenever any startup process track
existed, so the review player could switch from audible stream zero to stale process tracks and
make late-start Discord appear unrecorded. Split-track clips now default to mixed Output Audio plus
non-process inputs such as the microphone; selecting individual app tracks remains available and
mutually exclusive with mixed output. Runtime process discovery is still a separate, larger
enhancement. The focused `player_core` regression test covers the safe default.

## Checkpoint (2026-07-17): Proxmox VM software H.264 fallback

Clipline can now record in Windows VMs that support WGC but expose neither a D3D11 video
processor nor a hardware video encoder. The existing hardware paths are unchanged and preferred.
The fallback reads WGC BGRA textures through a staging resource, performs deterministic limited-
range Rec.709 BGRA-to-NV12 crop/scale conversion in neutral Rust, and pipes NV12 to the LGPL
FFmpeg `h264_mf` encoder with `-hw_encoding 0`. `h264_mf` must pass a real one-frame probe before
the candidate is offered.

Verified live in this Proxmox Windows 11 VM on Microsoft Basic Display Adapter: Clipline ran at
1280×800/60 FPS, spawned `h264_mf` in forced software mode, saved three replays, populated their
Library thumbnails, and produced a validated 60.6-second H.264 MP4 with limited-range BT.709
metadata. The FFmpeg mux round-trip integration test exercised both SVT-AV1 and Media Foundation
software H.264. No Proxmox PCI passthrough, IOMMU, or virtual-GPU flag is required for this path;
its tradeoff is CPU usage, so reducing FPS/resolution is the first tuning lever.

Native Computer Use acceptance then saved and reviewed a fresh fourth replay at
`C:\Users\dain9\Videos\Clipline\2026-07-17 15-08\clip_1784326197.mp4`. Play/pause, click-seek,
playhead dragging, and post-scrub playback all worked without visible corruption. The 60.36-second
file is H.264 1280×800 limited-range BT.709 with two stereo Opus tracks and decodes cleanly; both
audio inputs were silent in this run. A five-second steady-state sample measured Clipline plus its
FFmpeg child at roughly 120% of one logical core (about 15% of this eight-logical-processor VM),
confirming the expected CPU cost rather than iGPU acceleration. Acceptance also caught that the
frontend discarded the backend's active encoder label, so Automatic mode could not identify the
selected fallback. The UI now retains the status event's encoder and exposes
`Stop recording · Software · H.264` on the active recorder control.

Implementation commits on `build-run-app` begin at
`5f354ab docs(capture): plan software VM encoder fallback`. The local ignored
`apps/clipline-app/ffmpeg/` directory contains the 2026-07-17 BtbN LGPL shared build used for live
acceptance. Keep distributing FFmpeg as a separate process and never add GPL encoders.

## Checkpoint (2026-07-16): repository simplification pass

Nightly 0.1.34 contains PRs #83 through #85. It ships the transactional reliability and long-MP4
fixes, resilient seeking with fast audio-only sidecar switching, continuous quiet-audio capture,
the dead-code/public-surface reduction, and the accepted arrow/J/L review-navigation remap. The
previous public nightly was 0.1.33, so the app and Tauri versions were bumped for updater delivery.

The primary checkout is on `main` at the same commit as `origin/main`. A conservative cleanup
removed unused preview readback, mixed-loopback audio, PCM mixing, MP4/buffer wrappers, generated
browser snapshots, and completed scratch notes. Internal buffer, event, League, and storage crates
now expose one root API instead of duplicate public module paths. No runtime behavior, dependency,
configuration, or persistence changes are intended.

Review-player navigation now uses left/right arrows for five-second seeks (Shift for one second)
and J/L for frame-aligned ten-frame steps. Automated contracts and manual acceptance pass. Local
capture data under `.gsi-spike/` remains untracked and must not be cleaned. `cargo test
--workspace`, fresh-cache workspace clippy with warnings denied, formatting, and diff validation
all pass on Windows.

## Checkpoint (2026-07-15): fast audio sidecar switching implemented

The whole-video review preview path has been replaced end to end. The original `<video>` now stays
loaded while selected audio tracks are extracted to reusable audio-only MP4 sidecars and played by
synchronized hidden audio elements. Manual acceptance on the reproduced 31-minute clip remains.

### Workspace and preservation constraints

- Active branch: `sidecar-sync-policy`
- Active worktree:
  `C:\Users\dain\.paseo\worktrees\1qv1k36q\friendly-sheep`
- The original checkout at `C:\Users\dain\Projects\clipline` has user-owned uncommitted changes in
  `apps/clipline-app/tests/player_core.rs`, `apps/clipline-app/tests/ui_contract.rs`,
  `apps/clipline-app/ui/index.html`, `apps/clipline-app/ui/player-core.js`, and
  `apps/clipline-app/ui/review-player.js`, plus untracked `.gsi-spike/`. Never overwrite, stage, or
  clean those changes. Continue only in the isolated worktree.

### User-visible state

- The rapid right-arrow/forward-seek reset was fixed by making the logical seek target
  authoritative across media events and source generations. The user manually confirmed this item
  appears fixed.
- Quiet WASAPI endpoints now synthesize timeline-continuous silence with one 20 ms capture-latency
  allowance. The real hardware sync test passed with approximately 11.7 ms maximum skew.
- Explicit audio switches are serialized/coalesced and no longer assign a preview to `video.src`.
  The directly playable first track stays on the original video; other non-empty selections use
  synchronized sidecars, and an empty selection is muted output.
- Every audible sidecar path is protected from the total 2 GiB LRU cache while active. The only
  known orchestration limitation is that an already-running FFmpeg extraction is not cancelled;
  its stale result may populate cache but cannot activate.

### Diagnosis and approved architecture

The reproduced 31:31, 1.88 GiB clip exposed the root cause: each uncached selection read the whole
source, rebuilt another full MP4 containing copied video, wrote roughly 1.9 GiB, and reloaded the
video element. That creates about 3.8 GiB of disk traffic, several GiB of live buffers, and cache
thrashing.

Live measurements with the packaged FFmpeg:

- one audio track copied to audio-only MP4: 1.87 s, 23.9 MB;
- two tracks copied in one FFmpeg process: 0.50 s, 47.7 MB total;
- two tracks decoded/mixed/re-encoded to one audio-only MP4: 15.0 s.

The user approved an approximately 0.5-to-2-second first uncached switch and near-instant cached
switches. The approved design keeps the original `<video>` loaded, caches one stream-copied
audio-only MP4 per embedded track, and plays selected tracks through synchronized hidden audio
elements. The video remains the authoritative clock with a 100 ms drift threshold.

Read these documents completely before continuing:

- `docs/superpowers/specs/2026-07-15-audio-sidecar-switching-design.md`
- `docs/superpowers/plans/2026-07-15-audio-sidecar-switching.md`

### Completed sidecar work

The design and all six implementation tasks are committed or ready in the current cleanup commit:

- `f4a08779` — `docs(player): design fast audio sidecar switching`
- `a53a83c8` — `docs(player): plan fast audio sidecar switching`
- `e1a947bf` — `feat(mp4): expose media track counts`
- `311dc21a` — `feat(player): prepare cached audio sidecars`
- `516aef21` — `fix(player): harden audio sidecar preparation`
- `7050c29b` — `fix(player): close audio sidecar publication boundaries`
- `4dd47e1` — `feat(player): define audio sidecar transport policy`
- `5a99b13` — `feat(player): add synchronized audio sidecar transport`
- `585553d` — `fix(player): switch audio without reloading video`

Completed behavior:

- `prepare_clip_audio_sidecars` accepts `{ path, audioTrackIds, protectedPreviewPaths }` and
  returns ordered `{ audioTrackId, path }` records.
- Per-track `audio-track-sidecar-v1` cache keys reuse a track across selection combinations.
- One FFmpeg process extracts all missing selected streams with explicit `0:a:N`, `-vn`, and
  `-c:a copy`; the new path never copies or maps video.
- Existing requested hits are protected before pruning, validated, touched, and reused.
- Outputs validate as exactly zero video tracks and one audio track before publication.
- Publication ownership remains armed across the blocking task and Tauri asset-scope calls. A
  failure removes only invocation-owned finals; collision winners and prior hits are never owned.
- Legacy clips without audio marker metadata use a bounded `Read + Seek` MP4 metadata reader that
  skips `mdat`. Finalized `moov` allocation is capped at 64 MiB, with malformed size/header/EOF
  coverage.
- The video is the authoritative clock. Sidecars force-align on activation and seek, mirror
  play/pause/rate, and correct ordinary drift only above 100 ms using one 500 ms timer while
  playing.
- User mute and volume are logical state independent of transport-level video muting. Original
  video audio is not silenced until every current-generation sidecar is playable and its play
  promise succeeds.
- Opening a clip selects every default review track, including the microphone, while the first
  embedded track starts immediately; the complete selection activates atomically after its
  sidecars are ready without reloading the video.
- Direct source playback follows audio stream index zero even when marker rows are reordered, and
  each source assignment keeps one removable error listener for its full lifetime.
- Validated sidecar cache hits retain their ordered result without a redundant second validation;
  validation/publication owns temporary-file cleanup on every failure path.
- Clip open/close, suspend, source release, replacement, and rename invalidate callbacks, stop the
  drift timer, pause sidecars, remove their sources, call `load()`, and release Windows file
  handles.
- The legacy `preview_clip_audio_tracks` command, whole-source reader/remuxer, combination cache
  key, preview-only writer, and FFmpeg video-copy/`amix` path have been removed. Old
  `audio-preview-*.mp4` files remain ordinary LRU eviction candidates.

Verification reported green at this checkpoint:

- `cargo test -p clipline-mp4 media_track_counts -- --nocapture`
- `cargo test -p clipline-mp4`
- `cargo test -p clipline-app audio_sidecar -- --nocapture`
- `cargo test -p clipline-app audio_preview_cache -- --nocapture`
- `cargo test -p clipline-app --test player_core audio_preview_queue -- --nocapture`
- `cargo test -p clipline-app --test player_core logical_seek -- --nocapture`
- `cargo test -p clipline-app --test ui_contract legacy_audio_preview -- --nocapture`
- `cargo test --workspace` — 775 listed tests, all green
- `cargo clean -p clipline-app`
- `cargo clippy -p clipline-app --all-targets -- -D warnings`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --check`

### Exact next steps

1. Launch this worktree with
   `CLIPLINE_FFMPEG=C:\Users\dain\AppData\Local\Clipline\ffmpeg\ffmpeg.exe`.
2. On the reproduced 31-minute clip, verify uncached one/multi-track switches take approximately
   0.5–2 seconds, cached switches are nearly immediate, and rapid selection changes apply only the
   newest selection.
3. While sidecars are active, verify seeking/right-arrow spam never reloads or resets the video;
   also exercise play, pause, scrub, playback rate, mute, direct fallback, empty selection, clip
   changes, and rename.
4. Force an extraction/load failure and verify the previously audible selection continues, then
   restart once to confirm total preview-cache pruning still respects active protected files.

## What this project is

Clipline is an open-source, lightweight, ad-free game recorder for Windows (see `ddoc.md`):
ShadowPlay-style replay buffer, **no DLL injection ever** (anti-cheat safety is the core
architectural bet), automatic timeline event markers via the League of Legends Live Client
Data API, Hybrid MP4 output, Rust core + Tauri UI.

## Current state (2026-07-09): a working tray recorder with a first-party review player

Thirty-five milestones executed (plans in `docs/superpowers/plans/*.md` — plan docs are kept there, all
completed task-by-task with strict TDD; read any of them to see the conventions in action):

1. **WGC capture** — monitor + window, GPU-side frames, QPC-anchored pts
2. **MFT H.264 encoder** — async hardware MFT (AMF on the dev box), GPU NV12 path, AVCC out
3. **WASAPI loopback audio** — system audio → real Opus (audiopus), silence gap fill
4. **A/V sync hardening** — stamp-derived MP4 timeline, one shared clock, `avsync` validator
   (real-engine test: −8.3 ms total drift)
5. **Tauri shell** — `apps/clipline-app`: tray app, replay-buffer service thread, **Alt+F10**
   global hotkey → `Videos\Clipline\clip_<unix>.mp4`, smart no-overlap saves
6. **Event markers** — League poller (1 Hz, quiet retry outside matches) → `MarkerLog` →
   `<clip>.markers.json` sidecars re-based to clip time; mock-server verified end-to-end
7. **Library + marker timeline** — clip list (duration/size/age/marker badge), in-app playback
   (H.264+Opus `<video>` works in WebView2 via the asset protocol), marker ticks with
   click-to-seek, path-validated delete
8. **Disk quota + auto-GC** — neutral storage manager scans `Videos\Clipline`, counts MP4s plus
   marker sidecars, enforces a default 10 GiB oldest-first quota after saves, protects the
   just-saved clip, and surfaces usage/quota/clip count in the UI. `--disk-quota-gb 0` disables
   GC; any positive number sets the GiB cap.
9. **Settings** — `%APPDATA%\Clipline\settings.json` persists capture target, buffer/replay
   seconds, bitrate, FPS, disk quota, and save hotkey. The in-app Settings panel validates and
   saves changes, restarts the recorder service with new recording options, rebinds the global
   hotkey, updates the tray label, and keeps the storage row on the active quota.
10. **Trim/export editor** — the player overlay now has in/out controls and exports a sibling MP4
    without touching the source clip. `clipline-mp4::trim_keyframe_aligned` parses Clipline's
    finalized H.264/Opus MP4 tables, aligns start backward and end forward to video keyframes,
    stream-copies selected samples into a fresh finalized MP4, and crops marker sidecars.
11. **Review player v2** — clips open in a two-pane review player with no native video chrome:
    dimmed-outside-trim timeline with draggable in/out edges and amber marker ticks,
    transport row (marker prev/next, ±5 s, play/pause, tenths readout, rate, volume),
    keyboard-first review (`Space`/`K`, `←→`/`J`/`L` 5 s / `Shift` 1 s, `,`/`.` 0.1 s,
    `I`/`O` trim at playhead, `M`/`Shift+M` markers, `Esc`), and an export row that shows the
    kept range live. There are deliberately no trim number inputs — position the playhead,
    then mark. The UI is split into `index.html` / `styles.css` / `player-core.js` (pure,
    DOM-free logic) / `main.js` (wiring); `player-core.js` is unit-tested **from Rust** via
    `boa_engine` (`tests/player_core.rs`), and `tests/ui_contract.rs` guards the DOM contract.
    (An earlier externally-authored workspace, `bd1c84f`, was reverted and redone this way.)
12. **Review player polish** (Outplayed comparison-driven) — typed marker chips
    (kill ✕ / spree ★ / objective ◆ / structure ▣ / info •, kind-colored, unknown kinds
    degrade to info), labeled time ruler with nice-step gradations, transport reordered to
    sit under the stage, human-first library labels ("Jun 11 · 10:25 PM" + marker digest,
    filename in the tooltip), focus mode (`F` hides the sidebar), live scrubbing
    (seek-throttled via the `seeked` event so WebView2 keeps painting; trim-handle drags
    ride the playhead and pause/resume playback).
13. **Session folders** — saves land in `Videos\Clipline\<session>\`: one folder per recorder
    run (label `YYYY-MM-DD HH-MM`, local time, fixed at service start) plus a dedicated
    `… league` folder per detected LoL match (the poller now sends
    `MatchStarted`/`MatchEnded`; `GameEnd` events also end the match session). Folders are
    created lazily at save time; exports stay siblings so they inherit the folder; the
    library groups by session with legacy root clips under "Earlier"; `reveal_clip` opens
    Explorer with the clip selected; storage status/GC scan root + one level and delete
    emptied session folders. assetProtocol needed a second glob
    (`**/Videos/Clipline/**/*.mp4`) for subfolder playback.
14. **Stage overlay transport** — the transport row moved onto the video as a translucent
    hover bar (gradient scrim, hand-authored inline SVG icons, no icon font/npm): pins while
    paused, fades after 2 s idle while playing (`PlayerCore.overlayVisible`, evaluated from
    the playhead rAF loop — no timers), hides on pointer-leave, wakes on pointer/keyboard.
    Volume is an icon + hover-expanding slider. `ui_contract` now requires `<svg` inside
    every transport button.
15. **Sidebar rail + header cleanup** — the hamburger collapses the sidebar to a 52 px
    icon rail (status dot, save, gear; `F` toggles; rail state survives clip open/close)
    instead of the old full-collapse focus mode. Header is two icon buttons (folder reveal,
    trash delete); Copy Path is gone (the path in `#pmeta` is selectable text) and Close is
    gone (click the active library row again, or `Esc`). Export is a scissors-"Clip" primary
    button. Delete confirmation is an in-app `<dialog>` (Delete left / Cancel right, user
    preference) — `ui_contract` bans native `confirm()`/`alert()` and the removed header ids
    outright.
16. **Settings page** — settings left the sidebar fold for a full-bleed tabbed page in the
    main pane (Capture / Recording / Storage / Hotkeys; name + description rows; one Save
    footer). Reached via the sidebar Settings row or the rail gear; exits via ✕, `Esc`
    (priority over closing the clip; player shortcuts are inert behind the page), or opening
    a clip. The open clip pauses and survives the round-trip. Field ids and the
    validate/save/restart wiring are unchanged from milestone 9.
17. **Display-region capture** — Capture settings now include `display_region`, persisted as
    `{ display_id, x, y, width, height }`. The settings page renders a virtual desktop map with
    draggable/resizable region box, numeric pixel fields, and right-click menu actions
    (Align: left/right/top/bottom/center; Set to Display: enumerated Win32 displays). The
    recorder enumerates monitors with `EnumDisplayMonitors`, captures the selected monitor with
    WGC, derives a safe in-frame crop from virtual-desktop coordinates, and crops GPU-side in the
    D3D11 video processor before MFT encode. This is intentionally a single-display region crop;
    stitched regions spanning multiple monitors are still out of scope. Verified locally with
    `CARGO_TARGET_DIR=target\codex-test cargo test --workspace`,
    `CARGO_TARGET_DIR=target\codex-test cargo clippy --workspace --all-targets -- -D warnings`,
    and a static Chrome screenshot harness for the settings UI.
18. **Hotkey recorder** — Settings > Hotkeys no longer asks users to type shortcut strings.
    `#set-hotkey` is a read-only recorder: focus/click it, press F1-F11/F13-F24 with optional
    Ctrl/Alt/Shift, and the UI writes the normalized shortcut (`F10`, `Ctrl+Alt+F9`, etc.)
    through the same validate/save/rebind path. Modifier-only input prompts for an F-key,
    `Escape` cancels, F12 is rejected as debugger-reserved on Windows, and invalid keys stay in
    recorder mode with inline status. The pure formatter lives in `ui/player-core.js` and is
    covered by `tests/player_core.rs`; `ui_contract` requires the read-only recorder/status
    markup.
19. **Settings UX cleanup** — the display-region map no longer has its own internal scrollbars;
    it computes a static height from the virtual desktop shape and lets the settings page own any
    scrolling. Recording settings now read in user terms: replay history, save length, video
    quality, and smoothness. Recording controls use sliders with human summaries and visible scale
    markers, and quality snaps to Compact/Balanced/Sharp/Maximum preset stops. The underlying ids
    and persisted settings values are unchanged.
20. **Recording controls cleanup** — the user-facing Replay history control is gone; Clipline keeps
    the internal rolling buffer at two minutes and exposes only Save length, capped at 5 sec-2 min
    with 30 sec / 1 min / 2 min presets. Smoothness now has 30/60/90/120 FPS stops. The Settings
    page no longer has the top-right X button, so the bottom-left Settings control is the close
    affordance. The sidebar now shows a clickable capture status (`Capturing Desktop`, window, or
    display region), storage/quota/clip count, and Save Replay; it no longer shows buffered seconds,
    MB, or GOP diagnostics. The new `set_recording` Tauri command stops/starts the recorder from
    that status control. Stopping intentionally clears the rolling replay buffer, and internal
    settings restarts do not emit a stale stopped status.
21. **Audio device controls + mic capture** — Capture settings now include Audio output and
    Microphone controls. Users can keep system/output audio on or off, select default or explicit
    render/capture endpoints, set output and mic gain from 0-200%, enable microphone capture, and
    choose Mono mic handling with a checkbox. When output and mic are both enabled, the recorder
    mixes them into one normal Opus track so the in-app player and regular video players hear both;
    single-source output-only or mic-only captures still use the normal WASAPI Opus source. The mic
    path accepts common WASAPI float/PCM formats and resamples to Opus' 48 kHz timeline. Capture
    also has a live Test mic monitor: the button toggles to Stop testing, plays the selected mic
    back through Web Audio, and shows a live level meter. Output audio remains enabled by default;
    mic capture is opt-in for privacy.
22. **Media folder settings + Explorer fixes** — Storage settings now has a Media folder path.
    The recorder service, library listing, delete/export validation, storage quota/status, and
    folder-opening commands all use the same persisted root instead of independently assuming
    `Videos\Clipline`. The default is still `Videos\Clipline`; changing it restarts the recorder
    and creates the folder before saving settings. The review header's folder button opens the
    containing folder directly, and the Storage tab uses a native Choose Folder picker to set the
    media root.
23. **FFmpeg encoder matrix** (ddoc §4) — recording is no longer MFT-H.264-only. `clipline-mp4`
    is codec-aware (`VideoTrackConfig::{h264,hevc,av1}` → `avc1`/avcC, `hvc1`/hvcC, `av01`/av1C;
    HEVC PTL parsed from the SPS, AV1 profile/level/tier from the sequence-header OBU; trim is
    codec-agnostic). `clipline-capture` gained neutral `hevc`/`av1` bitstream modules and an
    FFmpeg **subprocess** encoder: `FfmpegVideoEncoder` spawns a bundled `ffmpeg.exe`, pipes NV12
    in (GPU frames are converted BGRA→NV12 on the GPU via the existing `VideoConverter` then read
    back through a staging texture), and a reader thread frames the elementary stream into access
    units (`framing.rs`: Annex B by VCL NAL for H.264/HEVC, IVF temporal units for AV1). The probe
    (`ffmpeg.rs`) locates `ffmpeg.exe` and reports `{h264,hevc,av1}_{nvenc,amf,qsv}` + `libsvtav1`
    by parsing `-encoders` and test-encoding each hardware encoder. `probe.rs` now carries an
    `EncoderApi` axis (Mft vs Ffmpeg) and `rank_encoders(caps, decodable, preference)` — backend
    merit, MFT preferred over FFmpeg for the same combo, Auto restricted to player-decodable codecs
    and now H.264-first for playback compatibility. The recorder walks the ranked candidates until one opens (behind
    `Box<dyn Encoder>`), reports the active encoder in the sidebar status, and warns on explicit
    fallback. Settings has one Encoder dropdown listing the machine's real backend×codec combos;
    the UI probes WebView2 (`canPlayType`) for HEVC/AV1, marks undecodable codecs "(limited
    playback)", and reports the decodable set so Automatic never records an unplayable clip.
    **The subprocess approach was chosen over linking libavcodec** (deliberate revision of the
    plan): zero unsafe FFI, version-robust, cleanest LGPL boundary. Decisions, sharp edges, and
    the not-yet-done parts are below.
24. **Custom game detection foundation** — Settings now has a Games tab with built-in profile
    placeholders and a custom game workflow: Add Custom Game scans visible top-level windows,
    records process path/exe/title metadata, and saves enabled custom rules under
    `%APPDATA%\Clipline\settings.json`. A background detector enumerates visible windows every
    2 seconds and, when a saved custom game is running, restarts the recorder onto that concrete
    WGC window handle; when it disappears, Clipline falls back to the normal Capture target. This
    remains no-injection/no-memory-read: only Win32 window/process metadata plus WGC window capture.
    The sidebar/status surface reports `Capturing Game: <name>` while a custom game override is
    active. Windowed game capture uses the HWND client rect, so title bars/borders are excluded
    from saved replays. The WGC frame pool now respects per-frame `ContentSize` and recreates on
    capture-item resize; the NV12 converter rebuilds its video processor when the client texture
    size changes, scaling resized windows into the fixed MP4 track instead of artifacting or
    clipping to the first size. The review player also renders clips inside an aspect-locked
    `#stage-frame`, so WebView's `<video>` element cannot add top/bottom letterboxing when the
    available stage area is slightly off from the clip's aspect ratio. Custom game detection now
    owns per-window capture selection in the UI, so the old manual "Window title" capture target
    was removed from Settings > Capture while backend/CLI compatibility remains. The fallback
    Capture target dropdown lists available displays first and keeps the editable `SET REGION`
    option at the bottom; display selections persist as full-monitor display-region captures.
    - Settings > Games now has a manual Detect Games workflow beside Add Custom Game. Both flows
      open modal dialogs instead of inline panels; Detect Games scans Steam manifests only, shows
      unchecked candidates, dedupes existing custom games, and appends selected rows as normal
      Custom games using the existing save-to-apply flow. Saved custom games render in a compact
      scrollable list with each row's recording-mode toggle on the right.
25. **Full-session game recording** — Each saved custom game persists its own recording-mode
    preference (`replays_only` default, `full_session` selectable). Games set to full session start
    a shared-encoder Hybrid MP4 sink when the detected window becomes the active capture target,
    while continuing to feed the replay ring so Save Replay still works. The session sink now runs
    on a dedicated writer thread: sealed GOPs are cloned once and queued after the replay ring push,
    so disk stalls or secondary file-write failures cannot abort primary replay capture. The MP4
    writer is initialized lazily on the first queued GOP so codec parameter sets discovered from
    the first HEVC/AV1/H.264 packets land in the final `hvcC`/`av1C`/`avcC`, and segment muxing uses
    borrowed sample slices instead of per-sample `Vec` copies. Full sessions finalize
    `session_<unix>.mp4` in the run's session folder on game disappearance, target switch, service
    stop, capture end, or clean shutdown; if encoder finish fails, the temp session is discarded
    with a warning rather than emitted as a complete recording. The on-disk file uses a temporary
    `.mp4.recording` suffix until finalized so the Library cannot open an in-progress fragmented
    recording. Non-empty orphaned `.mp4.recording` files are recovered to `.mp4` once per app
    process on launch, empty ones are removed, active recording bytes count toward storage usage,
    and GC avoids deleting the rest of the library when a protected full session alone exceeds
    quota. Recovery deliberately does not run on every recorder restart; custom-game target
    switches can overlap old/new service threads, and a repeated sweep can rename the active temp
    file before the old thread finalizes it. Finalization also treats "temp missing but final file
    already exists" as success so any session caught by that race is still emitted into the
    Library. Full sessions use the same marker sidecar, quota cleanup, library refresh, and
    saved-event path as manual replays, and the library labels them as "Full session".
26. **Game plugins + League auto-recording** — Game-specific behavior now sits behind a built-in
    plugin registry (`apps/clipline-app/src/game_plugins.rs`) instead of hardcoded UI/settings
    branches. Settings persist generic plugin state under `games.plugins.<plugin_id>` with
    enabled + recording-mode fields, and the frontend renders Settings > Games from the backend
    `list_game_plugins` catalog. The first plugin is `league_of_legends`: it matches only the
    real in-game `League of Legends.exe` top-level window, not `LeagueClientUx.exe` or Riot
    launcher windows, so champion select/client activity does not start full-session recording.
    League is enabled by default and defaults to `full_session`; when the match window appears,
    Clipline switches capture to that window and starts a shared-encoder session recording, then
    finalizes it when the window disappears. Custom games remain as the generic fallback layer
    beneath plugins.
27. **Plugin event sources + in-game hotkey fallback** — Built-in game plugins can now expose an
    optional event-source spawner in addition to their window matcher. The recorder carries the
    active built-in plugin id in `ServiceOptions` and asks that plugin for markers; League owns the
    Live Client Data API poller, while custom games record with no marker source unless a future
    plugin adds one. Save Replay now also has a Windows `WH_KEYBOARD_LL` fallback hook, kept in sync
    with the Settings > Hotkeys shortcut, so games that suppress Tauri/Win32 registered global
    shortcuts still reach the recorder. All save triggers share a short debounce to avoid double
    saves when both hotkey paths fire.
28. **Explicit SDR color metadata** — Desktop/game captures are no longer left to driver,
     encoder, or player color-range inference. The WGC BGRA path is treated as full-range RGB
     Rec.709 and the D3D11 video processor converts to limited-range NV12 Rec.709; MFT and FFmpeg
     encoders receive matching color attrs/flags, and `clipline-mp4` writes `colr`/`nclx` sample
     entry metadata. A real smoke recording now probes as `color_range=tv`,
     `color_space=bt709`, `color_transfer=bt709`, and `color_primaries=bt709`.
29. **Startup on Windows login** — Settings now has a General tab with an "Open on startup"
     toggle. When enabled, Clipline registers itself in the Windows Run registry key
     (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) via `tauri-plugin-autostart`,
     passing `--autostart` so launches from the registry start minimized to the tray instead
     of opening the main window.
30. **Audio track splitting v1** — Output audio is split by current Windows render-session
     process using process-loopback capture, so game/Discord/Spotify/browser audio can land in
     separate Opus tracks. Clipline keeps a mixed Output Audio track first as a playback/export
     safety track, then app/process tracks, then microphone when enabled; when the experimental
     "app audio tracks" Capture setting is off, only the mixed Output Audio track is recorded.
     That setting defaults off.
      Electron-style apps that emit
      multiple child-process audio sessions are grouped by same-executable root process before
      process-loopback capture, so Discord should appear once instead of as renderer/audio-service
      duplicates. Launcher parent sessions (for example Steam) are dropped when a child process
      also has its own audio session, because process-loopback captures the target process tree and
      otherwise records the game twice with a small offset. Clipline also filters its own
      `clipline-app` process out of split app-audio tracks so replay-save notification sounds are
      not selected as a separate default source. Mid-stream buffered replays advertise a one-frame
      (20 ms) Opus pre-skip so cold decoders discard the first-frame startup artifact instead of
      playing it as a short burst at clip start. The
      process-loopback activation path uses an agile completion handler and an owned `VT_BLOB`
      activation payload; the dev machine reproduced heap corruption when that blob pointed at
      stack memory. Saved
     replays and full-session recordings write `audio_tracks` metadata into marker sidecars, the
     review deck exposes an expandable track checklist, and the upload dialog lets users choose
     which tracks to include. Single-track and muted selections are stream-copy remuxed through
     `clipline-mp4::remux_with_selected_audio_tracks`; multi-track share/upload selections are
     exported through the native Opus mixer so external players receive one audio stream. New audio
     sessions that appear after recording starts are not discovered dynamically yet.
31. **Mouse hotkeys + selected-track uploads** — Settings > Hotkeys accepts middle mouse,
     Mouse4, and Mouse5 when combined with Ctrl/Alt/Shift, in addition to F1-F11/F13-F24.
     Keyboard F-key shortcuts still use Tauri's OS global-shortcut registration plus the
     low-level fallback; mouse-button shortcuts are hook-only through an on-demand Windows
     low-level mouse hook. The rail now shows the active Save Replay hotkey below RAM. Single-track
     and muted cloud uploads use lightweight selected-track remuxing; multi-track cloud/share
     exports now use native Opus mixing so external players hear one normal audio stream.
32. **Library multi-select + bulk actions** — the local gallery supports selecting multiple
     clips and acting on them in bulk. A filter-toolbar `#gallery-select-toggle` button labeled
     `Select multiple` flips the whole grid into selectable mode where clicking a tile toggles
     selection instead of opening it; the normal per-card trash affordance is hidden while this
     mode is active so selection and one-off deletion do not compete. A `#gallery-bulk-bar` appears
     inside the filter toolbar with `Select all` / `Clear` / `Delete` / `Cancel` and a live count.
     `Delete` runs the new
     `delete_clips` Tauri command (one round trip, validates every path up front via
     `validate_clip_path`, deletes mp4 + `markers.json` sidecar + cached poster, returns a
     `DeletedClipsReport { deleted, failed }` so partial success is surfaced rather than swallowed).
     `Esc` clears the selection then exits select mode; `Ctrl+A`
     (in select mode) selects all visible. Selection is keyed on `clip.path` (survives
     filter/sort/group/re-render), is **local-only** — the Cloud tab hides the Select toggle and
     clears/exits selection on entry. Backend work is
     split into a testable `delete_clips_impl` (no `tauri::State`) so the partial-success +
     sidecar/poster cleanup behavior is covered by a unit test; `tests/ui_contract.rs` gains
     `gallery_supports_multi_select_bulk_actions`.
33. **First-party supported game presentation** — the installable plugin direction was replaced
     with built-in supported game profiles. League remains the first profile, with declarative
     presentation data for marker styling, gallery cards, a playback-synced, pull-tab-collapsible
     right-side event rail, and a bottom metadata strip. Event ingestion stays core-owned behind
     the built-in `league_live_client` capability; game integration updates ship with normal
     Clipline releases instead of external plugin zips or Settings-driven package installs.
     `EventKind`, `GameId`, `is_review_event()`, and `is_timeline_marker()` remain core-owned:
     profiles style the closed marker vocabulary but cannot add event kinds or change persistence
     policy. The review player
     threads presentation into pure `player-core.js` marker helpers and `main.js` renders
     profile-driven gallery summaries, marker styling, the event rail, and metadata. League's Live
     Client summary keeps optional participant/team roster data so the event rail can render
     kill-feed-style actor/victim champion portraits from Data Dragon, actor/objective rows for
     turret/dragon/baron events, blue/red row treatment, restored first-party timeline marker
     icons, and a separate event-rail icon map using first-party kill/death silhouettes plus
     CommunityDragon objective icons. Gallery cards use the profile `gallery.card` policy for title
     and icon behavior; League keeps full-session cards titled by K/D/A plus CS/min when fresh
     sidecars have creep-score data, while replacing the generic League logo with the local
     champion portrait. League's metadata strip resolves the local champion portrait through the
     Riot Data Dragon champion-square provider, renders summoner spells beside the portrait, shows
     value-first K/D/A plus ratio, and appends a compact item-build row from fresh Live Client
     sidecar data; older clips fall back to whatever summary fields they already have. Settings >
     Games remains backend-driven for supported game rows but no longer exposes check/update/
     reinstall/reset package actions.
34. **osu! play-block foundation** — the desktop side now has a first-party `osu!` supported-game
     profile (`osu!.exe`, full-session focused), an Account/Plays settings dialog that plainly
     collects a user-provided osu! OAuth app Client ID, Client Secret, and user id/username, plus
     a question-mark setup guide that opens a local walkthrough. The client secret is stored in
     Windows Credential Manager, not `settings.json`; the desktop uses the client-credentials
     grant directly and sends `x-api-version: 20220705` when fetching recent scores so failed plays
     have real ids and `ended_at`. `ClipMarkers.plays` sidecars support interval play blocks.
     Full-session saves from osu!-tagged sessions write durable
     `.osu-enrichment.json` pending records; startup/library refresh retries are idempotent, and
     storage/delete cleanup tracks those pending sidecars with marker/poster files. The pure
     mapper accepts normalized osu! scores, keeps fails, requires `ended_at`, prefers
     `started_at`, derives estimated starts from beatmap length with DT/HT adjustment, clamps
     derived failed starts against the previous play, dedupes score ids, applies UTC/skew
     overlap, and reports when the 500-score fetch ceiling may leave plays missing.
     The review UI can render osu! intervals as timeline blocks, a right-side "Set plays" rail,
     hover/focus details, seek/highlight behavior, and osu! gallery summaries. A real spike
     confirmed client credentials with `public` scope can fetch Dain's recent osu!standard scores,
     including submitted failed plays, so there is no Clipline Cloud broker dependency.
35. **Reliability and playback hardening** — Full-session finalization now retains non-empty
    `.mp4.recording` files for startup recovery when writer finalization or the final rename fails.
    Settings changes plan recorder options without taking the active command sender and commit the
    restart only after persistence/tray/hook work succeeds. Cloud-library loads are account-scoped
    and generation-guarded, forced refreshes supersede in-flight requests, renamed clips carry and
    rewrite pending osu! enrichment, and all deletion/quota paths include markers, clip metadata,
    pending enrichment, and posters. Finalized MP4s switch `mvhd`/`tkhd`/`mdhd` to version 1 above
    `u32::MAX`, with `u128` duration rescaling. Multi-audio preview swaps resolve the playhead after
    generation completes, consume the latest queued seek, and rapid relative seeks accumulate.

Verification (2026-07-09): formatting, workspace Clippy, and fresh-cache Clippy for the three
changed crates passed. The first non-CI workspace test run had one transient real-clock device-test
failure; its exact rerun, a subsequent complete non-CI workspace rerun, and the CI-mode full
workspace test run passed. App launch and manual playback verification are deferred until this
branch is integrated.

> Claude handoff: the library clip-icon/labeling thread was paused at the user's request. If you
> resume it, the user wants no monitor/desktop icon and no tiny checkbox/corner badge. The desired
> shape is a full-size clapper icon on the left, only for videos that are actually user-created
> clips, likely after finishing a clearer labeling model.

Recent fixes (2026-07-06):
- Nightly 0.1.33 contains the profile-category review filter work from PR #80 and the library
  launch-surface fixes from PR #81. The previous public nightly metadata was 0.1.32, so the app
  and Tauri package versions were bumped to 0.1.33 for updater delivery. Review timeline and match
  event filters now key off profile-declared marker categories instead of League-only kind names;
  `InhibKilled` appears under Structures and `FirstBlood` is no longer double-counted as a kill.
  Library badges keep SESSION/TRIM/CLOUD text optically centered, fresh installs bundle the LGPL
  FFmpeg resource used for gallery posters, and the launch-time update dialog is draggable while
  leaving its action buttons clickable.

Recent fixes (2026-07-04):
- Settings > Recording now has an Advanced toggle for exact recording overrides. When enabled,
  `advanced_recording` supplies custom max output bounds (aspect-preserving, never stretching),
  exact bitrate Mbps, and exact FPS to the recorder while the normal preset controls remain the
  default path. Video-quality summaries now include the preset bitrate (for example,
  `Sharp quality - more detail. 24 Mbps.`), and the disk replay estimate follows the exact
  bitrate when Advanced is enabled.
  Verified with focused settings/UI/player-core tests, `cargo test --workspace`, and
  `cargo clean -p clipline-app; cargo clippy --workspace --all-targets -- -D warnings`.

Recent fixes (2026-07-03):
- Settings now opens as a popup over the current Library/Review view instead of replacing the
  main pane. Unsaved edits change `Close` to `Discard Changes`; the first discard attempt
  shakes the popup, shows `Careful--your changes aren't saved.` in red beside `Discard Changes`,
  and makes `Save Settings` glow. A second discard button press closes and restores the last
  saved settings. Backdrop clicks close only when the form is clean; with unsaved edits they
  warn/shake/glow repeatedly until the user presses `Save Settings` or `Discard Changes`.
  Rows with unsaved changes now get a blue glow, and tabs containing changed rows show a pip;
  indicators clear when edits are saved, discarded, or reverted.
  Verified with `cargo test --workspace` and
  `cargo clean -p clipline-app; cargo clippy --workspace --all-targets -- -D warnings`.

Recent fixes (2026-07-02):
- Nightly 0.1.28 contains the custom game detection workflow and review follow-ups from PRs
  #72 and #73. The previous public nightly metadata was 0.1.27, so the app and Tauri package
  versions were bumped to 0.1.28 for updater delivery. Custom games can now be added from a
  Steam-based detected-games modal with checkbox selection, the custom games list is compact and
  scroll-contained, and visible non-game windows are no longer added as standalone detection
  results.
- Nightly 0.1.27 contains the osu! play-block polish and CI review fixes from PR #71. The
  previous public nightly metadata was 0.1.26, so the app and Tauri package versions were bumped
  to 0.1.27 for updater delivery. osu! timeline bars now handle overlapping intervals cleanly,
  incomplete plays use their purple treatment, exported play clips keep the song title without
  intrusive marker metadata, account settings preserve saved API credentials, and the cross-platform
  UI contract tests declare their serde_json dependency explicitly.
- Nightly 0.1.26 contains the gallery hover/enrichment refresh-loop hotfix from PR #70. The
  previous public nightly metadata was 0.1.25, so the app and Tauri package versions were bumped
  to 0.1.26 for updater delivery. Library card hover no longer flickers from repeated refreshes,
  and osu! pending enrichment only emits a UI refresh when visible play metadata changed.
- Nightly 0.1.25 contains the osu! play-block release from PR #69. osu! is now a real
  supported-game profile with stable/cutting-edge detection, title-change play timing, optional
  direct API enrichment, Set plays metadata cards, interval blocks, and right-click play export
  without marker metadata in the exported clip.
- The osu! profile now detects the stable idle title `osu!`, stable map titles such as
  `osu!  - ginkiha - EOS [Lycoris]`, and cutting-edge build titles such as
  `osu!cuttingedge b20260624`, while explicitly rejecting updater-like titles from `osu!.exe`.
  osu!-tagged full sessions shorter than ten seconds are discarded as boot/update transients.
  Its empty Set plays rail copy now points users to the osu! API settings credentials instead of
  implying enrichment completed with no submitted plays.
- Added the osu! play-block implementation plan at
  `docs/superpowers/plans/2026-06-30-osu-play-blocks.md`, plus the desktop schema/UI/enrichment
  scaffolding and reusable API spike script. The shipped auth path is direct desktop
  client-credentials with a local setup guide, not the earlier Cloud broker/proxy.
- Supported-game rows now persist a nested `review` settings block. Each supported row has a
  Settings button that opens a grouped tabbed dialog: General controls Replays only vs Full session
  and whether to show League match details, Match events filters the right-side rail by your events,
  team fights, and map events, and Timeline markers filters your markers vs map markers. Fresh
  recordings keep broader review events (`is_review_event`) in marker sidecars so those filters can
  show ally/enemy events; older recordings only contain whatever marker data existed when they were
  captured.
- League local-player assists now normalize as `ChampionAssist`, survive the timeline-marker
  filter, and render with the new assist icon/category; the refreshed sword kill icon is used by
  both timeline markers and the right-side match events rail.
- Nightly 0.1.24 is a hotfix for the review timeline action row and League minion turret-kill
  presentation. The previous public nightly metadata was 0.1.23, so the app and Tauri package
  versions were bumped to 0.1.24 for updater eligibility.
- The review player's snip action now lives as an icon-only control at the far right of the
  below-timeline metadata row instead of taking its own row or appearing inside the timeline.
- League event rail rows using `actor_event` layout now map non-participant minion actor ids
  like `Minion_T200...` to CommunityDragon minion portraits, so minion turret kills render as a
  compact icon row instead of exposing the raw minion id text.
- Legacy/no-sidecar multi-audio MP4s now infer their audio track list from the finalized MP4 tables
  and use the same native preview mixer/upload selection paths as fresh split-audio clips. The
  inferred metadata is playback-only, so clip duration still comes only from real sidecar markers.
- The review player no longer has a session-wide "audio preview unavailable" latch; failed preview
  generation falls back for that attempt without blocking later multi-track preview retries.

Recent fixes (2026-06-29):
- Nightly 0.1.22 is a hotfix for local review playback of output+mic clips. The previous
  public nightly metadata was 0.1.21, so the app and Tauri package versions were bumped to
  0.1.22 for updater eligibility.
- Local review audio previews now use the native `clipline-mp4` Opus mixer before falling back
  to FFmpeg, so Clipline-authored multi-track output+mic recordings play back as one audible
  stream in WebView2 even when external FFmpeg is missing.
- Nightly 0.1.21 contains the simple timeline editor from PR #66. The previous public nightly
  metadata was 0.1.20, so the app and Tauri package versions were bumped to 0.1.21 for updater
  eligibility.
- The review deck now defaults to a simple Outplayed-style timeline: whole-clip browse view first,
  a scissors button enters local trim mode around the playhead, and `Create Clip` uses the existing
  keyframe-aligned export path. The previous navigator/zoom/snap editor is still available via the
  General setting `Legacy timeline editor` (`legacy_timeline_editor` in settings JSON). The simple
  timeline now keeps the scissors control above the track, layers event markers on the timeline band,
  and attaches a denser time ruler below it.
- Nightly 0.1.20 contains the League replay playback performance fix from PR #65. The previous
  public nightly metadata was 0.1.19, so the app and Tauri package versions were bumped to
  0.1.20 for updater eligibility.
- League review playback now avoids recomputing the event rail, marker metadata, and overlay
  digest work on every video time tick. The player throttles overlay detail refreshes while the
  video is running and keeps the event rail's active-row updates on a lighter schedule, reducing
  the frame stutter observed after the richer League presentation shipped.
- Nightly 0.1.19 contains the first-party supported game profile pivot and League presentation
  upgrade from PR #62. The previous public nightly metadata was 0.1.18, so the app and Tauri
  package versions were bumped to 0.1.19 for updater eligibility.
- League clips now have built-in supported-game presentation data for marker styling, gallery
  cards, a playback-synced right-side event rail, and richer bottom metadata driven by the
  first-party profile. The old standalone installable plugin package path is intentionally not
  part of this release; game presentation updates now ship through normal Clipline nightlies.

Recent fixes (2026-06-27):
- Nightly 0.1.18 contains the default multitrack playback fix and gallery thumbnail hardening
  from PR #63. The previous public nightly metadata was 0.1.17, so the app and Tauri package
  versions were bumped to 0.1.18 for updater eligibility.
- Review playback now mixes default output+mic multi-track captures for WebView2/share targets
  that only play the first audio stream, but falls back to source playback without a persistent
  error when ffmpeg audio mixing is unavailable. Local poster failures are cached for the app
  session and stay on the gradient placeholder instead of using per-card video elements that can
  keep Windows file handles open.
- Nightly 0.1.17 contains the local clip-library multi-select/bulk-delete workflow and the
  replay-audio fixes from PR #61. The previous public nightly metadata was 0.1.16, so the
  app and Tauri package versions were bumped to 0.1.17 for updater eligibility.
- Replay muxing now avoids carrying non-zero Opus pre-skip into freshly cut replay clips and
  selects the intended WASAPI loopback process tree, fixing the start-of-clip audio burst and
  the Steam-track tunnel/phase artifact observed in newly recorded clips.
- Nightly 0.1.16 contains the memory/duplicate-instance guard, close-to-tray playback suspension,
  settings-draft preservation, replay Opus pre-skip fix, and rustfmt drift cleanup. The previous
  public nightly metadata was 0.1.15, so the app and Tauri package versions were bumped to 0.1.16
  for updater eligibility.
- Close-to-tray now emits a frontend playback-suspend event before hiding the WebView, so review
  audio/video and pending preview work stop instead of continuing behind the tray session.
- Settings now keep an explicit unsaved draft while the settings page is open. Tab switches and
  async device/display/encoder refreshes read from that draft, so saving at the end preserves edits
  made across multiple settings tabs.
- Replay clips cut from the middle of an Opus stream now write audio tracks with zero `dOps`
  pre-skip, avoiding the tiny start-of-clip audio drop that only belongs at the original stream
  beginning.
- Runtime memory/duplicate-instance guard: Task Manager reports of many Clipline rows were partly
  WebView2 child process labeling, but duplicate top-level `clipline-app.exe` processes were also
  allowed. The Tauri shell now registers `tauri-plugin-single-instance` before autostart so normal
  duplicate launches reveal the existing window and `--autostart` duplicates stay quiet. The
  recorder also byte-budgets the pending GOP before ring insertion (capped at 64 MiB), drops
  leading non-keyframes until the first keyframe, and errors clearly if an encoder stops producing
  keyframes instead of accumulating packets indefinitely. Verified with focused `ui_contract` and
  `pipeline` regressions, `cargo test --workspace`, fresh-cache clippy, and a debug runtime
  duplicate-launch probe.

Recent fixes (2026-06-25):
- Nightly 0.1.15 contains the Cloud library tab/profile rail work, relaxed hotkey rules, and the
  PR #53 review follow-ups below. The previous public nightly metadata was 0.1.14, so the app and
  Tauri package versions were bumped to 0.1.15 for updater eligibility.
- Connected cloud identity in the rail: when `settings.cloud` has a stored credential target/user,
  the bottom-left rail shows a compact profile button above Settings. It refreshes the account from
  `/api/v1/auth/me`, prefers `display_name` over username, fetches `GET /api/v1/me/avatar` with the
  stored bearer token via the native `cloud_user_avatar` command, and opens the user's cloud profile
  at `/u/{username}`. A small in-process ETag cache handles avatar 304 responses; 404 or fetch errors
  keep an initials fallback and disconnect hides the rail identity entirely.
- Library cloud source tab: the Library header now has Local/Cloud tabs. The desktop pins
  `clipline-cloud-api` to Clipline Cloud `v1.2.18` and uses `CloudClient::list_clips` to fetch the
  authoritative server library (`GET /api/v1/clips`, paged newest-first). Cloud cards still merge
  local upload records by `client_clip_id` so they can show whether a local copy is present, and
  fall back to persisted `settings.cloud.uploads` rows while the server list is unavailable. Rows
  with a matching local file now render as normal playable local clip cards. Cloud-only rows fetch
  authenticated thumbnails and media through native commands, cache them under
  `%APPDATA%\Clipline\cloud-cache`, and play the cached MP4 through the existing review player;
  `Open page` still opens the owned cloud page externally. PR #53 review follow-up: disconnected
  Cloud tab rendering no longer recurses, fallback upload rows keep `remote_clip_id` so cloud-only
  history can play in-app, thumbnails lazy-load through the shared poster observer, transient list
  errors stay visible without latching the tab permanently loaded, cloud-cache files are
  account-namespaced/pruned/bounded by size, and cloud-only review playback hides local-file
  actions while rerouting the header cloud button to copy the cloud link. The Cloud list command
  still fetches every page before first render; convert it to first-page render + lazy pagination if
  large cloud libraries become sluggish.
- Recorder startup display recovery: startup primary-monitor capture now resolves the primary
  display through the same `EnumDisplayMonitors` path used by Settings instead of
  `MonitorFromPoint(0,0)`, which could bind to a ghost/wrong monitor on some Windows layouts.
  Display-region capture also recovers from a missing saved display id or stale region geometry by
  warning the user and falling back to the full current primary display when the saved display is
  gone. If the saved display still exists but the region only partially fits, the crop clamps to
  the visible part instead of silently recording the whole display. Full-display region selections
  are recognized by display size and re-based to the current monitor origin so Windows virtual
  desktop coordinate churn across reboot does not require opening Settings and saving again.
- Share/export audio compatibility follow-up: the 0.1.12/0.1.14 remux-only upload behavior could
  hand cloud/Discord a multi-audio-track MP4 where only the first stream was played, producing
  silent uploads or missing mic audio. Cloud uploads now replace two-or-more selected audio tracks
  with one native mixed Opus track while stream-copying video, and clipboard copy uses the same
  selected-audio compatibility export under `%APPDATA%\Clipline\share-exports` before setting
  CF_HDROP. This is native `audiopus` decode/mix/re-encode inside `clipline-mp4`; users do not
  need FFmpeg installed for multi-track upload/share audio. The mixer preserves the source Opus
  pre-skip, averages overlapping tracks to avoid hard clipping, and streams slot-by-slot instead of
  buffering all decoded PCM. Share-preview/export cache writes use unique sibling temp files and
  prune orphaned `.mp4.tmp` files.
- WebView2 compatibility follow-up for the Windows 10 tester whose Edge/WebView2 registry state
  was missing: Nightly 0.1.14 switches the normal NSIS installer from Tauri's WebView2
  `offlineInstaller` to the small embedded Evergreen bootstrapper, while keeping
  `minimumWebview2Version = 120.0.2210.55`. Fresh installs and updates can now fetch/repair the
  runtime from Microsoft during install instead of carrying the large offline runtime in every
  Clipline installer. This is not an air-gapped compatibility claim: offline or Microsoft-blocked
  machines may still need the WebView2 Runtime installed manually.
- The app now has a native already-broken-install recovery signal. `main.js` invokes
  `frontend_ready` once JavaScript boots and IPC works; the Rust shell logs `frontend_ready
  received`. When `open_main_window` reveals the UI, it also probes `is_visible()` explicitly and
  classifies Tauri's typed `Runtime(FailedToReceiveMessage)` as a dead WebView2 signal. If that
  getter probe fails or the frontend-ready watchdog expires, Clipline shows one native `rfd`
  repair dialog per process from a worker thread. This matters because a dead WebView2 frontend
  cannot trigger the in-app updater; already-broken users need reinstall/manual WebView2 repair.

Recent fixes (2026-06-24):
- Windows 10 follow-up from Nate's 0.1.12 logs: the recovery-window build also produced
  immediate `failed to receive message from webview` state calls, while Windows 11 works
  normally. Treat this as WebView2/runtime creation trouble, not a hidden-window bug. Nightly
  0.1.13 removed the `main-recovery-*` churn, kept revealing the existing `main` handle when
  getters fail, logged Microsoft Edge WebView2 runtime registry `pv` values at startup, and set
  `minimumWebview2Version = 120.0.2210.55` so Windows 10 installs repair/update stale runtimes.
- Published Nightly 0.1.12 with the mouse-hotkey, selected-audio-track upload remux, release
  diagnostics, and dead-window recovery work from PR #51.
- Added release-build diagnostics for the tray/open-window path. Clipline now appends
  single-line entries to `%APPDATA%\Clipline\clipline.log`, including startup args,
  tray menu/icon events, close-to-tray handling, window event summaries, WebView labels,
  and before/after window state around `Open Clipline` (`visible`, `minimized`, `focused`,
  position, and size). The log rotates to `clipline.old.log` after 1 MiB.
- Tray close now hides the app window instead of destroying it. A destroyed Tauri window can leave
  a `main` webview label behind whose state calls fail with `failed to receive message from
  webview`; 0.1.12 briefly tried recovery labels, but Windows 10 logs showed new recovery
  webviews failing the same way, so the recovery path was removed again in favor of WebView2
  runtime diagnostics and installer enforcement.
- Save Replay hotkeys now support middle mouse, Mouse4, and Mouse5 when combined with
  Ctrl/Alt/Shift. Mouse hotkeys skip the OS global-shortcut registration path and are handled by
  an on-demand low-level mouse hook; switching between keyboard and mouse hotkeys
  unregisters/registers only the keyboard shortcut side. The rail shows the current save hotkey
  below RAM.
- Cloud upload briefly remuxed explicit selected audio tracks instead of mixing multiple selections
  through FFmpeg, avoiding the old "ffmpeg is not available for audio track mixing" failure but
  exposing first-audio-stream playback problems in external players. The 2026-06-25 native-mix
  follow-up above supersedes that behavior for multi-track selections.

Recent fixes (2026-06-22):
- Tray "Open Clipline" now uses the same reveal path as a normal foreground launch:
  show the hidden WebView window, restore it if it is minimized, then focus it. This fixes
  tray-only sessions where recording/capture kept running but the interface did not come
  back from the tray.
- Startup now treats OS global-hotkey registration as best-effort. If `Alt+F10`
  is already owned by another recorder/overlay, Clipline continues launching,
  keeps the tray/menu path available, and still installs the low-level in-game
  hotkey fallback instead of aborting during Tauri setup with no visible UI.
  Settings rebinds now skip unregistering stale, never-registered shortcuts and
  retry an unchanged missing shortcut without blocking unrelated settings saves.
- Opening a cloud-uploaded clip now rechecks its remote Clipline Cloud state in the background:
  visibility/link changes refresh the local upload record, finalized remote deletions clear the
  local cloud badge/link, and temporary 404s for `uploaded_processing` records keep the local
  processing record.
- Cloud uploads briefly mixed multiple selected audio tracks into one Opus stream, this was
  replaced on 2026-06-24 with selected-track remuxing for every explicit upload selection, and the
  2026-06-25 native-mix follow-up restored single-stream multi-track uploads without requiring
  FFmpeg.
- Debug/Cargo builds now keep Windows startup registration disabled and clear stale debug Run-key
  entries on launch/status checks; installed release builds keep normal startup behavior.

Recent fixes (2026-06-21):
- Bug-scan app reliability slice: recorder restarts now build replacement service options before
  dropping the old command sender, settings saves go through a synced sibling temp file and atomic
  replace, cloud ready-poll timeouts preserve an `uploaded_processing` record with its remote link
  instead of stuck `processing`, cloud auto-delete removes poster sidecars, disk replay cache/media
  overlap checks are case-insensitive on Windows, split-output clips apply the default selected-track
  preview on open, and opening a new clip clears the previous playhead RAF/pending seek.
- Split-audio review/upload semantics: when per-process output tracks exist, the "Output Audio"
  checklist row is a master toggle for those process output tracks, not an extra mixed track to
  include alongside them. The mixed Output Audio stream remains in the file as a fallback/safety
  track, but selected previews omit it while process tracks are active to avoid doubled audio.
  Exact all-physical-track preview requests return the original clip path instead of generating a
  mixed preview.

Recent fixes (2026-06-19):
- Library rows now keep full title/context text visible, then fade the right edge on hover/focus
  to reveal a borderless trash affordance. League clip metadata intentionally wraps onto its own
  line, and the death skull marker is mask-scaled to visually match kill markers.
- Deleting a clip updates the local library cache and storage summary instead of doing a full app
  refresh, avoiding the visible lag spike after delete.
- Custom game detection treats saved process path/exe identity as authoritative. Legacy
  title-only custom rules ignore browser processes, so YouTube tabs with a game title do not start
  game recording or trigger save-on-return behavior.
- The native WebView/Chromium context menu is suppressed. Library rows own a small right-click
  menu with Upload, Rename, Rename file, and Delete actions.
- Library rows and the review header rename clips by saving a metadata-backed display title without
  moving the MP4. The secondary Rename file action still validates Windows-safe MP4 names, moves
  marker/poster/metadata sidecars with the source file, preserves the clip kind, and keeps matching
  cloud upload records pointed at the new local path.
- Upload buttons now open an in-app dialog for title, description, and visibility before upload.
  Nonblank descriptions are trimmed and sent on `POST /api/v1/uploads`; blank descriptions are
  omitted. New cloud uploads no longer include deprecated marker payloads in the create request.
- Rename/export no longer run heavy filesystem/media work on the UI path. Rename first tries to
  move the file without unloading the player, only releasing the video handle on a Windows lock
  retry; export returns enough metadata for the UI to insert the new clip row locally instead of
  rescanning every clip.
- Startup avoids the old library/probe burst: `list_clips` and `storage_status` run on the blocking
  pool, library listing uses marker-sidecar duration instead of reading whole MP4s, and display /
  audio / encoder probes are deferred until after first paint or Settings opens. Plain clips without
  a marker sidecar may have unknown duration in the library list; the UI now omits that value rather
  than showing `?`.
- Audio splitting v1 records output audio as per-process MP4 audio tracks when Windows process
  loopback is available, keeps microphone as a separate track, carries track labels in sidecars,
  shows review/upload checklists, and remuxes only selected tracks for cloud upload. It falls back
  to a mixed Output Audio track if no process tracks start or the experimental Capture setting is
  turned off; the setting defaults off. Duplicate child sessions from apps like Discord are grouped
  by same-executable root process before capture. The Windows process-loopback path was fixed after reproducing
  `STATUS_HEAP_CORRUPTION`: keep the activation payload as an owned
  `VT_BLOB`, keep it alive until `GetActivateResult`, and make the completion handler agile.
- Review audio-track checkboxes now affect playback as well as upload: WebView-native track toggles
  are used when available, otherwise Clipline stream-copies a temporary selected-audio preview MP4
  under `%APPDATA%\Clipline\audio-previews` and reloads the player at the same timestamp.
- PR review follow-ups: opening a multi-track clip no longer eagerly creates a full-length audio
  preview; preview generation starts only after the user changes track selection. Multi-track
  preview mixing now surfaces FFmpeg failures instead of falling through to an unmixed MP4, and
  the preview cache key was bumped to avoid reusing old fallback artifacts. If some process-loopback
  tracks start but others fail, Clipline appends the mixed Output Audio fallback so game/system
  audio is still preserved. Cloud upload records now supersede older records for the same clip
  path, so retrying with a different audio-track selection does not leave stale failed state in
  the library.
- Review playback now treats any source MP4 with more than one audio track as needing the selected
  audio preview/mix, even when every track is selected. This keeps default output+mic captures
  audible in WebView2 and common share targets that only play the first track; if ffmpeg-based
  mixing is unavailable, the app falls back to source playback without pinning a persistent error.
  Local gallery poster failures are cached for the app session and stay on the gradient placeholder
  instead of attaching per-card video elements that can hold Windows file locks.
- Review audio previews now try the native `clipline-mp4` Opus mixer before FFmpeg, so
  Clipline-authored output+mic clips get a one-stream local preview even when external FFmpeg is
  missing. The FFmpeg mixer remains a fallback for legacy/non-Opus files the native mixer cannot
  parse.

Run it: `cargo run -p clipline-app` (settings persist under `%APPDATA%\Clipline\settings.json`;
options still override startup behavior: `--window <title substring>` to capture one window
instead of the primary monitor, `--lol-url <url>` to point the marker poller at a mock, and
`--disk-quota-gb <n>` to override the saved quota for that launch). The media folder is now a
saved Storage setting; changing it affects future library scans, saves, exports, and quota checks.
Useful examples: `record_smoke -- --seconds 5 --window <w> --audio` (full pipeline + sync
report + ffprobe), `wgc_smoke` (capture only). Everything is verified live on this machine —
real clips with matching A/V durations, real marker sidecars, real in-app playback.

| Crate | What it does | Verified by |
|---|---|---|
| `clipline-events` | Event schema (ddoc §5), game-clock→recording anchor math, `MarkerLog`/`ClipMarkers` sidecars | unit tests |
| `clipline-lol` | League Live Client adapter: client, dedupe, normalization, `poll_once` | httpmock integration + `markers_e2e` |
| `clipline-buffer` | Replay ring of GOP segments (video + N audio tracks), byte eviction, `save_window` smart mode | unit tests |
| `clipline-storage` | Saved-clip inventory, sidecar-aware size accounting, oldest-first quota GC with protected fresh saves | unit tests |
| `clipline-mp4` | Hybrid MP4 muxer (frag→finalized in place), **codec-aware** (H.264/HEVC/AV1: avc1/hvc1/av01 + avcC/hvcC/av1C), Rec.709 limited `colr` metadata, multi-track + Opus, box walker, `movie_duration_s`, codec-agnostic keyframe-aligned stream-copy trim | ffprobe + unit tests |
| `clipline-capture` | Traits + mocks + `Recorder` (steppable, save-while-recording) + **all real Windows engines** under `src/windows/` (`wgc`, `mft`, `nv12`, `wasapi`, `mft_probe`, `d3d11`, `window`) + the **FFmpeg subprocess encoder** (`ffmpeg`, `ffmpeg_encoder`, `framing`) + explicit SDR Rec.709 limited-range conversion/encoder metadata + neutral `annexb`/`hevc`/`av1`/`opus`/`pcm`/`clock`/`avsync`/`probe`; WASAPI covers selectable mixed output loopback, per-process output loopback, mic capture, mic level testing, PCM decode, and resampling to 48 kHz; window helpers enumerate visible HWND/process metadata for custom game detection | mocks on CI; CI-skipped device + ffmpeg tests run real on the dev machine |
| `apps/clipline-app` | Tauri 2 shell: service thread, configurable hotkey, tray, status/library/settings plus the first-party review player; Settings > Games persists custom game rules and auto-switches capture to detected game windows | live e2e (screenshots in the session logs) + `player_core` (Boa) + `ui_contract` |

## Machine setup (already done on this machine; for a fresh clone elsewhere)

1. **Git identity** (repo-local, doesn't travel): `git config user.email "dain98@gmail.com"`,
   `git config user.name "Dain"` — commits are authored by the personal account.
2. **Remote/auth:** repo is `https://github.com/dain98/clipline.git` over **HTTPS** with gh as
   credential helper (`gh auth setup-git`, account `dain98`). Don't switch to SSH — the
   machine's agent key belongs to a different GitHub account.
3. **Rust** stable + clippy. `cargo test --workspace` must be green before starting.
4. **ffmpeg/ffprobe** (winget `Gyan.FFmpeg`) — the ffprobe e2e tests self-skip without it.
   On this machine the binaries live under
   `%LOCALAPPDATA%\Microsoft\WinGet\Packages\Gyan.FFmpeg_...\ffmpeg-8.1.1-full_build\bin`
   (fresh shells get them on PATH; long-lived shells may need the full path).

## Development conventions (unchanged since day one — keep them)

- **Plan-driven TDD.** Each milestone gets `docs/superpowers/plans/YYYY-MM-DD-<name>.md` with
  complete code and bite-sized steps; execute strictly failing-test-first. Plans are committed
  before execution; checkboxes stay unticked (repo convention).
- **Commits:** conventional style (`feat(capture): …`), one logical change, trailer
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` when Claude authors.
- **Quality gates per milestone:** workspace tests green, `cargo clippy --workspace
  --all-targets` zero warnings, push, **CI green on ubuntu + windows**, handoff updated.
- **Platform discipline:** neutral logic stays neutral (testable on both CI OSes); Windows
  code behind `#[cfg(windows)]`; trait changes happen neutral-side first with tests; all
  `unsafe` confined to `windows/` modules behind safe wrappers.

## Sharp edges (each of these cost real debugging time — read before touching)

**CI / testing**
- Device tests (WGC, MFT, WASAPI, real-clock sync) are **hard-skipped under `CI`**:
  windows-2025 runners report `IsSupported()==true` for WGC then access-violate inside the
  capture component; they have no hardware encoder or audio endpoint. Local runs exercise
  them for real — the dev machine (RX 6700 XT, 5120x1440 primary) is the test rig.
- CI clippy can fail on lints a **warm local cache hides** — `cargo clean -p <crate>` before
  trusting a local clippy pass on changed crates.
- `clipline-app` keeps ubuntu CI webkit-free by gating *all* Tauri deps under
  `[target.'cfg(windows)'.dependencies]` with a stub `main` elsewhere; `build.rs` gates
  `tauri_build::build()` on `CARGO_CFG_WINDOWS`.

**Media pipeline**
- `clipline-mp4` wants **4-byte length-prefixed NALs**; MFTs emit Annex B — `annexb.rs`
  converts (and strips AUD/SPS/PPS). B-frames must stay **disabled** (no ctts in the muxer).
- **Async audio previews replace the video source:** never restore a playhead captured before the
  preview await. Resolve and consume `pendingSeek` immediately before `video.src` changes, and base
  repeated relative seeks on the queued target rather than stale `video.currentTime`.
- **Long finalized MP4s need version-1 duration boxes:** `mvhd`, `tkhd`, and each `mdhd` must switch
  independently when its duration exceeds `u32::MAX`; use a `u128` intermediate when rescaling.
- The MP4 timeline is **duration-cumulative**: video durations are re-derived from capture
  stamps at GOP seal; audio gaps become silence (`pcm.rs`); audio recorded before the first
  video packet is dropped (engine-init lead-in shifted video ~63 ms early before the fix —
  `avsync::validate_timeline` caught it on its first real run).
- WASAPI loopback requires a **48 kHz float mix format** (resampler is a follow-up); loopback
  goes quiet when nothing renders — that's why the gap fill exists.
- One D3D device and one `RelativeClock` must be shared across capture/encode/audio —
  the constructors force it (`WgcCapture::new_clock()`, `*_on(device, …, clock)`).
- H.264 hardware encoders cap near 4096 wide; the 5120-wide monitor scales to ≤2560
  (`even_dimensions` + scale in service/smokes).
- SDR color is explicit end-to-end: WGC BGRA is treated as full-range RGB Rec.709, the D3D11
  video processor outputs limited-range NV12 Rec.709, MFT/FFmpeg are given matching metadata,
  and MP4 sample entries write `colr`/`nclx`. If recordings look dark or oversaturated again,
  check this path before assuming a blue-light filter or player issue. HDR capture/display
  management remains separate future work.

**FFmpeg encoder tier (milestone 23)**
- It's a **subprocess**, never linked. `FfmpegVideoEncoder` spawns `ffmpeg.exe`; killing the
  recorder drops the child (Drop closes stdin + joins the reader). CI has no bundled ffmpeg, so
  `ffmpeg::probe()` returns empty and the live encoder test (`tests/ffmpeg_encode.rs`) self-skips;
  everything stays MFT-only there. The neutral bits (probe parsing, `framing.rs`, codec boxes)
  are fully unit-tested on both CI OSes.
- Ship an **lgpl-shared** build (BtbN) under `%APPDATA%\Clipline\ffmpeg` — it has SVT-AV1 + GPU
  encoders but **no libx264/libx265**, so no software H.264/HEVC. The dev box has it installed
  there; release builds stage that bundle into `apps/clipline-app/ffmpeg/` so the installer ships
  it as a Tauri resource for gallery poster generation and the optional encoder tier. The search
  order (`CLIPLINE_FFMPEG` override → bundled resource → exe dir → `%APPDATA%\Clipline\ffmpeg` →
  PATH) means the packaged LGPL build wins over any GPL PATH ffmpeg. Attribution:
  `THIRD-PARTY-NOTICES.md`.
- AMF **rejects tiny resolutions** (`Init() failed with error 5` at 128×72) — the probe
  test-encodes at 640×360. SVT-AV1 **errors on `-maxrate`/`-bufsize`** (exit -22): CBR capping is
  hardware-only; SVT-AV1 gets `-b:v` + `-preset 8` (VBR-ish; the ring evicts by bytes anyway).
- Access-unit framing assumes **one slice per picture** (the hardware-encoder default at our
  resolutions). H.264/HEVC keyframes are detected from the bitstream (IDR / IRAP); **AV1 keyframes
  are positional** (`frame_index % gop_frames == 0`) because IVF carries no keyframe flag — so
  scene-cut keyframes must stay disabled (they are: fixed `-g`, no scenecut flags).
- `EncoderBackend::MfSoftware` is modeled by the probe but **not instantiable** — `MftH264Encoder`
  only enumerates hardware MFTs. The candidate walk skips it; wiring the sync software MFT (CPU
  input, no D3D manager) is a follow-up. With no hardware H.264 and no ffmpeg, recording errors
  (same as before this milestone).

**Tauri (v2)**
- The webview **silently no-ops** (no events, no invoke) without
  `capabilities/default.json` granting `core:default`.
- The assetProtocol scope **does not resolve `$VIDEO`** — use plain globs. With configurable
  media folders the scope is currently `**/*.mp4`; diagnose media errors via a `video.onerror`
  handler because error code 4 usually means the scope rejected the request, not a codec problem.
- H.264+Opus MP4 plays natively in WebView2 — no native decode path needed until AV1/HEVC.
- `tauri-build` requires `icons/icon.ico` (ours is ffmpeg-generated).

**Misc**
- League Live Client testing without a match: `--lol-url` + the httpmock pattern in
  `crates/clipline-lol/tests/markers_e2e.rs`; a tiny local mock server works against the
  real app (see plan 2026-06-11-clipline-event-markers.md).
- Storage GC is save-time only for now. Default cap is 10 GiB; `--disk-quota-gb <n>` overrides
  it and `0` disables it. GC deletes MP4s oldest-first with matching `.markers.json` sidecars,
  but intentionally refuses to delete the clip that was just saved even if that leaves the
  directory over budget.
- Settings saves restart the recorder service immediately. Bad window-capture titles pass
  validation if non-empty, then surface as service init errors. Hotkey support is intentionally
  limited to modifiers plus F-keys (`Alt+F10`, `Ctrl+Alt+F10`, `Ctrl+Shift+F9`, etc.). The Tauri
  global shortcut path remains registered, and a low-level Windows keyboard hook is installed as a
  fallback for focused games that do not deliver the registered shortcut.
- Trim/export is intentionally v1: finalized Clipline-authored MP4s only, H.264 video with optional
  Opus audio, one sample description per track, no frame-accurate boundary re-encode yet. Exports
  are keyframe-aligned: in snaps backward to the previous sync sample and out snaps forward to the
  next sync sample/EOF, so the exported range can be wider than the numeric in/out request.
- The main pane stacks `#review-empty` / `#review-viewer` / `#settings-page` on one grid cell.
  Any `display:` rule on those views **defeats the `[hidden]` attribute** — every stacked view
  needs an explicit `[hidden] { display: none }` restatement and an opaque background (the
  empty state once bled through the settings page).
- UI automation: occluded windows swallow synthesized clicks while `PrintWindow`
  (PW_RENDERFULLCONTENT) still captures the window content — reposition/topmost before
  clicking; `CopyFromScreen` shows black for accelerated webviews. If someone is at the
  machine, their live mouse/window-drags race synthesized input — coordinate with them
  instead of fighting for the cursor.
- Frontend logic is testable without Node: `ui/player-core.js` is pure (no DOM, no Tauri,
  exposed via `globalThis`) and `tests/player_core.rs` evaluates it in `boa_engine`
  (dev-dependency). Keep player math/formatting there, not in `main.js`, or it falls out of
  test coverage. `tests/ui_contract.rs` fails if anyone re-inlines styles/scripts into
  `index.html` or puts `controls` back on the video element.
- osu! play enrichment samples osu! window-title changes every 500 ms during game detection and
  stores them in the pending `.osu-enrichment.json` sidecar. When osu! omits `started_at`, the
  mapper prefers the latest matching title event before `ended_at`; failed plays without a match
  stay end-only, and passed plays still include 1 s of results-screen padding.
- osu! full-session saves now write title-only `ClipPlay` blocks immediately from window-title
  changes even without osu! API credentials; later API enrichment replaces those fallback plays
  with full score metadata. In Set plays, no `pp` plus rank other than `F` renders as
  `Incomplete`, and right-clicking an interval play exports that play via the same keyframe-aligned
  `export_clip` path as trims. Play exports request an `Artist - Title` filename and pass
  `includeMarkers: false`, so the resulting clip opens without the Set plays sidebar/timeline
  metadata.
- WebView2 layout: a CSS grid row only bounds its children if the track is sized — the
  `.app`/`.review-viewer` grids pin rows with `minmax(0, 1fr)` and shrink children carry
  `min-height: 0`. A content-sized row lets the video's intrinsic height push the control
  deck below the window (this exact bug shipped once and was fixed in review-player v2).
- `ddoc.md` Caveats section lists every externally-verified Windows API claim with nuance —
  check it before trusting API behavior.

## What's next (rough value order; each gets its own plan)

1. **Auto-clip on importance** (ddoc §5): `importance ≥ threshold` → auto-save; marker kinds
   already carry importance.
2. **Next supported game investigation:** CS2 is the cleanest candidate because Valve Game State
   Integration is official and maps naturally to Clipline's event rail. Apex LiveAPI is promising
   after a local normal-match smoke test. TFT likely needs OCR/synthetic round markers plus Riot
   postgame data. Valorant/Fortnite should wait until there is a safe official data source worth
   integrating.
3. **Frame-accurate trim polish** (ddoc §11): re-encode only boundary GOPs, keep the current
   stream-copy path as the instant/lossless mode.
4. **In-app HEVC/AV1 playback** (ddoc §11): the encoder matrix (milestone 23) can record HEVC/AV1,
   but WebView2 can't decode them without OS extensions — Automatic avoids them and explicit picks
   warn. A native FFmpeg decode path feeding frames to the review player would close that gap.
   Smaller follow-ups from milestone 23: wire the Microsoft software H.264 MFT (the only
   software H.264 under LGPL), bundle the lgpl-shared ffmpeg into the installer, and revisit
   NVENC/QSV arg tuning (only AMF + SVT-AV1 were verified live on this RDNA2 box).
5. **Dynamic audio-session tracking** (ddoc §10): process audio is split at recorder start; new app sessions that appear mid-recording and multi-process grouping remain next.
6. **Polish toward release:** display-capture privacy warning (ddoc §9), borderless-fullscreen
   guidance (§8), WebView2-destroyed-when-minimized RAM trick (§4), installer/signing (§4).

Also worth knowing: the default `Videos\Clipline` folder on this machine holds test clips from the milestone
verifications (including `clip_1781160331.mp4` + sidecar — the marked test clip the library
demos nicely). The app may still be running in the tray from the last session.
