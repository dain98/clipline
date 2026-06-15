# Clipline — Development Handoff

> For a fresh Claude Code session (or human) continuing this project.
> **`ddoc.md` is the single source of truth** for product/architecture decisions. This file is
> the bridge: where the project stands, how it's built, what bit us, and what's next.

## What this project is

Clipline is an open-source, lightweight, ad-free game recorder for Windows (see `ddoc.md`):
ShadowPlay-style replay buffer, **no DLL injection ever** (anti-cheat safety is the core
architectural bet), automatic timeline event markers via the League of Legends Live Client
Data API, Hybrid MP4 output, Rust core + Tauri UI.

## Current state (2026-06-14): a working tray recorder with a first-party review player

Twenty-eight milestones executed (plans in `docs/superpowers/plans/*.md` — thirty-two plan docs, all
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

> Claude handoff: the library clip-icon/labeling thread was paused at the user's request. If you
> resume it, the user wants no monitor/desktop icon and no tiny checkbox/corner badge. The desired
> shape is a full-size clapper icon on the left, only for videos that are actually user-created
> clips, likely after finishing a clearer labeling model.

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
| `clipline-capture` | Traits + mocks + `Recorder` (steppable, save-while-recording) + **all real Windows engines** under `src/windows/` (`wgc`, `mft`, `nv12`, `wasapi`, `mft_probe`, `d3d11`, `window`) + the **FFmpeg subprocess encoder** (`ffmpeg`, `ffmpeg_encoder`, `framing`) + explicit SDR Rec.709 limited-range conversion/encoder metadata + neutral `annexb`/`hevc`/`av1`/`opus`/`pcm`/`clock`/`avsync`/`probe`; WASAPI covers selectable output loopback, mic capture, mic level testing, PCM decode, and resampling to 48 kHz; window helpers enumerate visible HWND/process metadata for custom game detection | mocks on CI; CI-skipped device + ffmpeg tests run real on the dev machine |
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
  there; the search order (`CLIPLINE_FFMPEG` override → exe dir → that folder → PATH) means it
  wins over any GPL PATH ffmpeg. Attribution: `THIRD-PARTY-NOTICES.md`.
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
- WebView2 layout: a CSS grid row only bounds its children if the track is sized — the
  `.app`/`.review-viewer` grids pin rows with `minmax(0, 1fr)` and shrink children carry
  `min-height: 0`. A content-sized row lets the video's intrinsic height push the control
  deck below the window (this exact bug shipped once and was fixed in review-player v2).
- `ddoc.md` Caveats section lists every externally-verified Windows API claim with nuance —
  check it before trusting API behavior.

## What's next (rough value order; each gets its own plan)

1. **Auto-clip on importance** (ddoc §5): `importance ≥ threshold` → auto-save; marker kinds
   already carry importance.
2. **Frame-accurate trim polish** (ddoc §11): re-encode only boundary GOPs, keep the current
   stream-copy path as the instant/lossless mode.
3. **In-app HEVC/AV1 playback** (ddoc §11): the encoder matrix (milestone 23) can record HEVC/AV1,
   but WebView2 can't decode them without OS extensions — Automatic avoids them and explicit picks
   warn. A native FFmpeg decode path feeding frames to the review player would close that gap.
   Smaller follow-ups from milestone 23: wire the Microsoft software H.264 MFT (the only
   software H.264 under LGPL), bundle the lgpl-shared ffmpeg into the installer, and revisit
   NVENC/QSV arg tuning (only AMF + SVT-AV1 were verified live on this RDNA2 box).
4. **Per-process audio loopback** (ddoc §10): system output + mic capture are in; per-game/process audio remains next.
5. **Polish toward release:** display-capture privacy warning (ddoc §9), borderless-fullscreen
   guidance (§8), WebView2-destroyed-when-minimized RAM trick (§4), installer/signing (§4).

Also worth knowing: the default `Videos\Clipline` folder on this machine holds test clips from the milestone
verifications (including `clip_1781160331.mp4` + sidecar — the marked test clip the library
demos nicely). The app may still be running in the tray from the last session.
