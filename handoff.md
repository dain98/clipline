# Clipline — Windows Development Handoff

> For a fresh Claude Code session (or human) continuing this project on a Windows machine.
> **`ddoc.md` is the single source of truth** for product/architecture decisions. This file is the
> bridge: where the project stands, how it's built, and exactly what the Windows milestone needs.

## What this project is

Clipline is an open-source, lightweight, ad-free game recorder for Windows (see `ddoc.md`):
ShadowPlay-style replay buffer, **no DLL injection ever** (anti-cheat safety is the core
architectural bet), automatic timeline event markers via the League of Legends Live Client Data
API, Hybrid MP4 output, Rust core + Tauri UI planned.

## Current state (2026-06-10)

All platform-neutral logic is built, tested (71 tests), clippy-clean (`-D warnings` in CI), and
validated against a real demuxer. CI runs `cargo test --workspace` + clippy on
**ubuntu-latest and windows-latest** on every push — the whole workspace already compiles and
passes on Windows runners.

| Crate | What it does | Verified by |
|---|---|---|
| `clipline-events` | Normalized event schema (ddoc §5) + game-clock→recording-timeline anchor math (pause-self-correcting) | unit tests |
| `clipline-lol` | League adapter: `127.0.0.1:2999` client, EventID dedupe, normalization w/ local-player tagging, `poll_once` pipeline | httpmock integration tests |
| `clipline-buffer` | Replay ring of GOP-aligned segments (video + N audio tracks), byte-budget eviction, keyframe-aligned `save_window` with smart no-overlap mode, RAM estimator | unit tests |
| `clipline-mp4` | Hybrid MP4 muxer (ddoc §10): fragmented while recording, finalized to standard MP4 in place; multi-track (h264 `avc1`/`avcC` + Opus `Opus`/`dOps`); box walker for validation/recovery | **ffprobe** parses output: correct streams, frame counts, duration |
| `clipline-capture` | `CaptureEngine`/`Encoder`/`AudioSource` traits, encoder probe (NVENC→AMF→QSV→x264, AV1→HEVC→H.264), `Recorder` pipeline (capture→encode→GOP segments→ring), `save_replay` → finalized A/V MP4 | mock-driven e2e + ffprobe |

Executed implementation plans (read these to see the conventions in action):
`docs/superpowers/plans/*.md` — eleven so far, all completed task-by-task with TDD.

**Milestone 6 (event markers) done 2026-06-11 — the differentiating feature is live.**
While the app records, a poller thread hits the League Live Client API at 1 Hz (quiet 5 s
retry while no game runs; `--lol-url` overrides for mocks), anchored events accumulate in
`clipline_events::MarkerLog`, and Save Replay writes `<clip>.markers.json` (markers re-based
to clip time) plus a marker count in the UI. The chain is CI-proven by
`crates/clipline-lol/tests/markers_e2e.rs` (httpmock), and was verified against the real app
with a local mock server: a DragonKill landed at t_s 2.15 s in the sidecar while an
out-of-window kill was correctly excluded. Clock bridge: `recording_t0 = Instant::now()`
sampled adjacent to `WgcCapture::new_clock()` (both QPC). Not yet: timeline UI rendering,
auto-clip on importance, VALORANT OCR.

**Milestone 5 (Tauri shell) done 2026-06-11 — Clipline is now a usable tray recorder.**
`apps/clipline-app` (`cargo run -p clipline-app [-- --window <title>]`): a windows-gated
Tauri 2 app — recorder service thread (WGC + AMF + WASAPI on one clock, 120 s ring,
`Recorder::step()` loop tolerating idle-screen timeouts) with **Alt+F10** global hotkey,
tray menu, and a status webview. Saves land in `Videos\Clipline` with smart no-overlap.
Verified live: two hotkey saves → 20.1 s clip (A/V durations equal to the millisecond) and a
4.000 s no-overlap follow-up, both decode-clean. Non-Windows targets build a stub `main`
(ubuntu CI needs no webkit2gtk). The neutral enabler: `Recorder::step()`/`finish_stream()`
(save-while-recording). Not yet: installer/bundling, settings UI, event markers in clips,
WebView2-destroyed-when-minimized.

**Windows progress: all four milestones done — the M0 platform layer is complete.**
Milestones 1 (WGC capture), 2 (MFT H.264 encoder), 3 (WASAPI loopback audio), and 4 (A/V
sync hardening) — see
`docs/superpowers/plans/2026-06-10-clipline-wgc-capture.md`,
`…-clipline-mft-encoder.md`, `2026-06-11-clipline-wasapi-loopback.md`, and
`2026-06-11-clipline-av-sync.md`.
The `#[cfg(windows)]` `windows/` module now holds: `WgcCapture`
(`CaptureEngine`, monitor + window, GPU-side frames, QPC-anchored pts), `MftH264Encoder`
(`Encoder`, async hardware MFT — AMF on the dev box — D3D-aware NV12 input, AVCC output,
CleanPoint keyframes, SPS/PPS from sequence header or first IDR, drain via the new
`Encoder::finish()`), `VideoConverter` (GPU BGRA→NV12 + scaling via D3D11 video processor),
`mft_probe::enumerate()` (real ddoc §3 probe; reports Amf{Hevc,H264}+MfSoftware{H264} here),
`WasapiLoopback` (`AudioSource`: default render endpoint in shared loopback, QPC-stamped
drains, real Opus via `audiopus`), and `d3d11` device plumbing (one shared device for
capture+encode, MT-protected). Platform-neutral additions: `annexb` (Annex B→AVCC, SPS/PPS
extraction), `opus` (20 ms/960-sample frame encoder), `pcm` (`LoopbackAssembler` — continuity
+ **silence gap fill**, required because loopback goes quiet when nothing renders and the MP4
audio timeline is duration-cumulative), `LimitedCapture`, `Encoder::finish()`.
**A/V sync (milestone 4):** the MP4 video timeline is derived from capture stamps at seal
time (`duration[i] = pts[i+1] − pts[i]`, the sealing keyframe closes each GOP exactly —
ddoc §6 "stamps, not cadence"; VRR-jitter mock test pins it); `avsync::validate_timeline`
checks keyframe-led segments, video continuity, per-segment audio coverage, and cumulative
drift against tolerances; the clock is an explicit constructor parameter
(`WgcCapture::new_clock()` → shared by `*_on(device, …, clock)` and `WasapiLoopback::start`);
the `Recorder` drops audio captured before the first video packet (the validator caught this
for real: engine-init lead-in would have shifted video ~63 ms early). Real-engine device test
(`real_engines_on_one_clock_produce_a_synced_timeline`) records WGC+AMF+WASAPI+Opus on one
clock and validates: total drift −8.3 ms.
**A/V end-to-end verified** via
`cargo run -p clipline-capture --example record_smoke -- --seconds 5 --window <w> --audio`:
5 s window capture → h264 (300 frames, 5.008 s) + opus (5.020 s), max inter-GOP gap 0.0 ms,
total drift 11.7 ms; decode clean; audio volumedetect shows real content. ffmpeg is installed
via winget (`Gyan.FFmpeg`) so the ffprobe e2e tests run for real locally. Sharp edges:
WGC/MFT/WASAPI device tests are hard-skipped under `CI` (windows-2025 runners access-violate
in WGC; no hardware encoder/audio endpoint); B-frames must stay disabled until the muxer
grows ctts support; the loopback path requires a 48 kHz float mix format (resampler is a
follow-up). **Next frontier (ddoc §15):** FFmpeg encoder matrix (NVENC/AMF/QSV, AV1/HEVC),
per-process audio loopback, mic track, Tauri shell wiring hotkey → `save_replay`, and the
continuous-recording second sink.

## Machine setup (do this first on the Windows clone)

1. **Git identity (repo-local config does not travel with a clone):**
   ```
   git config user.email "dain98@gmail.com"
   git config user.name "Dain"
   ```
   All commits in this repo are authored as `Dain <dain98@gmail.com>` (personal account).
2. **GitHub account:** the repo lives at `https://github.com/dain98/clipline` (personal account
   `dain98`, NOT the company account). Make sure `gh auth status` shows dain98 active — or pin the
   remote: `git remote set-url origin https://dain98@github.com/dain98/clipline.git`.
3. **Rust:** `rustup` stable + clippy. Verify with `cargo test --workspace` — all tests must pass
   before starting (the ffprobe e2e tests self-skip if ffprobe isn't installed; installing ffmpeg
   and having the tests run for real is recommended).

## Development conventions

- **Plan-driven TDD.** Each milestone gets a plan in `docs/superpowers/plans/YYYY-MM-DD-<name>.md`
  (complete code in the plan, bite-sized steps), executed strictly test-first: write failing test →
  verify failure → implement → verify pass → commit. Look at any existing plan for the format.
- **Commits:** conventional-commit style (`feat(capture): …`), one logical change each, ending with
  the trailer `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` (when Claude authors them).
- **Quality gates per milestone:** `cargo test --workspace` green, `cargo clippy --workspace
  --all-targets` zero warnings, push and confirm CI green on both OSes.
- **Never break the platform-neutral tests.** Windows code goes behind `#[cfg(windows)]`; the
  existing traits are the contract. If a trait needs to change, change it with tests on the
  neutral side first.

## Next milestone: the Windows platform layer

Goal: real implementations behind the existing traits in `clipline-capture`, so `Recorder` +
`save_replay` produce a real screen recording on a real GPU. Work top-down through these, one
plan each (they're independently verifiable):

1. ~~**WGC capture → `CaptureEngine`**~~ ✅ done 2026-06-10 (`crates/clipline-capture/src/windows/wgc.rs`)
   - `windows` crate (windows-rs): `Direct3D11CaptureFramePool` + `GraphicsCaptureItem`
     (monitor first, window capture second). Frames stay GPU-side
     (`FrameData` gains a `Gpu(ID3D11Texture2D)` variant behind `#[cfg(windows)]`).
   - Requirements from ddoc §3/§8: **no injection**, borderless-fullscreen guidance, display
     capture fallback w/ warning. `IsBorderRequired` suppression needs Win11 (ddoc Caveats).
   - Timestamps: WGC `SystemRelativeTime` → `pts_s` against capture start (QPC timebase —
     ddoc §6 "Clocking & A/V sync").
   - Verify: a windowed smoke binary that captures N frames and reports resolution/fps. Run
     manually — CI runners have no interactive desktop session for WGC.
2. ~~**Hardware encoder → `Encoder`**~~ ✅ done 2026-06-11 (`crates/clipline-capture/src/windows/mft.rs`)
   - Recommended first path: **Media Foundation H.264 encoder** (`IMFTransform`, hardware MFT) —
     no FFmpeg dependency yet, simplest route to validated end-to-end MP4s. The encoder probe
     (`probe.rs`) gets a real `enumerate()` that lists available MFTs/backends.
   - The ddoc §4 FFmpeg (LGPL, dynamic-link) decision still stands for the full matrix
     (NVENC/AMF/QSV, AV1/HEVC) — that can be milestone +1; don't block on it.
   - Must produce: SPS/PPS for `VideoTrackConfig` (strip from the MFT output / 
     `MF_MT_MPEG_SEQUENCE_HEADER`), keyframe flags, length-prefixed NALs (MP4 stream format —
     convert from Annex B if the MFT emits start codes).
   - Verify: `Recorder` with WGC + MFT on the dev machine → `save_replay` → file plays in a real
     player, ffprobe shows sane stream. That moment is the milestone exit criterion.
3. ~~**WASAPI loopback → `AudioSource`**~~ ✅ done 2026-06-11 system loopback + real Opus (`crates/clipline-capture/src/windows/wasapi.rs`); per-process loopback still pending:
   - System loopback first; per-process (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) second —
     note ddoc Caveats: documented for build 20348+/Win11 but works on updated Win10 2004+;
     `GetMixFormat`/`IsFormatSupported` return `E_NOTIMPL` on the process-loopback path, assume a
     fixed format. Opus encoding: `audiopus`/`opus` crate (libopus) or defer encoding and store
     PCM-in-Opus-clothing only for testing — real Opus before shipping.
4. ~~**A/V sync hardening**~~ ✅ done 2026-06-11 — stamp-derived MP4 timeline, shared-clock
   API, lead-in trimming, `avsync` validator + real-engine sync test (drift −8.3 ms).

Useful references: robmikh's windows-rs capture samples (ddoc §4 cites them as the de-risk),
Microsoft's ApplicationLoopback sample, `clipline-capture/src/mock.rs` for the contract each
trait implementation must honor.

## Things to know / sharp edges

- The CI Windows runner compiles `#[cfg(windows)]` code and runs non-interactive tests, but it
  has **no GPU encoder and no desktop session** — WGC/MFT runtime verification is manual on the
  dev machine. Structure Windows code so logic (NAL conversion, format negotiation, timestamp
  math) is unit-testable without devices, and only the thin device layer needs a human.
- `clipline-mp4` expects **length-prefixed NALs** (avcC `lengthSizeMinusOne=3` → 4-byte
  lengths). MFTs commonly emit Annex B. Convert and unit-test the converter.
- The repo has zero `unsafe` so far; the Windows layer will need it (COM). Keep it confined to
  the `windows/` modules with safe wrappers at the trait boundary.
- League Live Client API testing needs a live League match (or the httpmock fixtures — see
  `crates/clipline-lol/tests/`). The real endpoint is HTTPS with Riot's self-signed cert;
  `LiveClient::default_local()` already handles that.
- `ddoc.md` Caveats section lists every externally-verified fact with its source nuance — check
  it before relying on a Windows API behavior claim.
