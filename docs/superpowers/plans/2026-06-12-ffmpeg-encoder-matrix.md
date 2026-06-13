# Clipline FFmpeg Encoder Matrix Implementation Plan

**Goal:** Implement the ddoc §4 encoder matrix: NVENC / AMF / QuickSync hardware backends and an
LGPL-clean software tier, across AV1 / HEVC / H.264, via dynamically-loaded LGPL FFmpeg —
alongside (not replacing) the proven MFT H.264 path. Auto-selection follows ddoc merit order
(NVENC → AMF → QuickSync → software; AV1 → HEVC → H.264) but never auto-picks a codec the
in-app player cannot decode; explicit picks get a playback warning instead (in-app FFmpeg
decode for the editor is a separate follow-up milestone, ddoc §11).

**Shape (decisions locked with Dain 2026-06-12):**

- **FFmpeg alongside MFT.** `MftH264Encoder` stays as the proven zero-copy H.264 path and the
  ultimate fallback. FFmpeg adds `h264/hevc/av1_nvenc`, `*_amf`, `*_qsv`, and software
  `libsvtav1`. If FFmpeg DLLs are missing, the app behaves exactly as today.
- **LGPL only.** No `--enable-gpl`: x264/x265 are out. Software tier = SVT-AV1 (BSD, in LGPL
  FFmpeg builds) for AV1 plus the existing Microsoft software H.264 MFT as last resort. No
  software HEVC. The `EncoderBackend::X264` placeholder becomes `SvtAv1`.
- **Runtime loading, not link-time.** `libloading` opens `avcodec`/`avutil` at probe time;
  `avcodec_version()` must match one pinned major (set when the dev-box LGPL shared build is
  installed) or the FFmpeg tier reports unavailable. All `AVCodecContext` configuration goes
  through the `av_opt_set` string API so we never depend on that struct's layout; only the
  pinned-major `AVFrame`/`AVPacket` layouts are vendored, with a loader self-test
  (`av_frame_get_buffer` round-trip) as a layout canary. No build-time FFmpeg, no bindgen, no
  CI toolchain changes; Windows/Ubuntu CI compile everything and self-skip real-encode tests
  when no matching DLLs exist.
- **CPU NV12 input for FFmpeg v1.** GPU texture → staging readback → NV12 `AVFrame`. The MFT
  path keeps the zero-copy GPU route; D3D11 hwframes for FFmpeg is a later optimization. For
  same-vendor H.264 the MFT encoder therefore outranks the FFmpeg one.
- **Codec-aware muxing.** `VideoTrackConfig` grows a codec enum (H264{sps,pps},
  Hevc{vps,sps,pps}, Av1{seq_header_obu}); the muxer writes `avc1`/avcC, `hvc1`/hvcC (profile
  tier level parsed from the HEVC SPS), or `av01`/av1C. HEVC Annex B handling parallels the
  H.264 module (NAL type = (b>>1)&0x3F; strip VPS/SPS/PPS/AUD; keyframe = IDR/CRA). AV1
  samples are OBU passthrough minus temporal-delimiter OBUs; sequence header OBU goes to av1C.
- **Playback probe + warn.** The webview reports `canPlayType` decode support for
  hvc1/av01 once at startup; Auto-selection only considers decodable codecs, explicit
  HEVC/AV1 picks show a "may not play in the in-app player" warning in Settings and a clear
  in-player error on decode failure.

## Tasks

- [ ] **clipline-mp4: codec-aware video track config.** Replace raw `sps`/`pps` fields with a
  `VideoCodecParams` enum (H264/Hevc/Av1 as above); write `avc1`+avcC, `hvc1`+hvcC, `av01`+av1C
  sample entries; parse HEVC profile_tier_level and AV1 sequence-header fields needed for the
  config boxes; unit tests on hand-built parameter sets + ffprobe validation tests (self-skip
  without ffprobe) for all three codecs.
- [ ] **clipline-mp4: codec-agnostic trim.** `trim_keyframe_aligned` passes the source `stsd`
  sample entry through untouched instead of assuming avc1; keyframe alignment already rides
  `stss`. Tests trim an HEVC and an AV1 fixture.
- [ ] **clipline-capture: HEVC + AV1 bitstream modules.** Neutral `hevc.rs` (split/strip/
  keyframe/extract VPS+SPS+PPS, Annex B → length-prefixed) and `av1.rs` (OBU walker, strip
  temporal delimiters, extract sequence header, keyframe detection), unit-tested like
  `annexb.rs`.
- [ ] **Probe model rework (neutral).** `EncoderBackend::X264` → `SvtAv1`; capabilities carry
  which API provides them (Mft vs Ffmpeg); selection becomes an ordered candidate list
  `rank_encoders(caps, decodable_codecs, user_pref) -> Vec<Candidate>` for runtime fallback,
  with the rules: backend merit order, codec preference within backend, Auto restricted to
  decodable codecs, MFT preferred over FFmpeg for same backend+H.264, `MfSoftware` last.
  Unit tests for every rule.
- [ ] **FFmpeg loader (neutral, self-skipping).** `ffi` module: `libloading` of avcodec/avutil
  (Windows DLL names + Unix sonames), pinned-major version gate, vendored AVFrame/AVPacket
  layouts, function table, layout-canary self-test, `probe_ffmpeg()` returning capabilities by
  `avcodec_find_encoder_by_name` + test-open (hardware encoders confirm against the real GPU).
  Search order: exe dir → `%APPDATA%\Clipline\ffmpeg` → PATH/system default.
- [ ] **`FfmpegVideoEncoder` implementing `Encoder`.** Open by encoder name; configure via
  `av_opt_set` (`video_size`, `pixel_format`, `time_base`, `b`, `g` = 2 s GOP, `bf=0`,
  `flags=+global_header`, per-backend rate-control/preset/low-latency opts); NV12 frames in;
  packets out with Annex B → length-prefix conversion for H.264/HEVC and OBU handling for AV1;
  `track_config()` built from `extradata`; `finish()` drains. CPU-frame tests run real encodes
  when a matching FFmpeg is present (self-skip otherwise; live on the dev box, skipped on CI).
- [ ] **GPU → CPU NV12 readback (windows).** Staging-texture copy + map in the existing
  nv12/d3d11 modules so `FrameData::Gpu` feeds FFmpeg; device test (CI-skipped).
- [ ] **Service wiring.** `VideoEncoder` setting grows backend×codec choices (Auto default,
  serialized names stay snake_case; legacy values keep deserializing); recorder start walks the
  ranked candidate list until one opens, reports the active encoder/codec in status events, and
  falls back with a user-visible warning when the explicit choice fails.
- [ ] **Settings UI.** Recording tab gains Encoder and Codec selects populated from a
  `probe_encoders` command (only available combos listed); webview decode-capability probe
  (`canPlayType`) feeds both the warning badges ("may not play in the in-app player") and the
  Rust-side Auto policy; pure formatting/selection logic in `player-core.js` with Boa tests;
  `ui_contract` covers the new controls.
- [ ] **Dev-box FFmpeg install + docs.** Install the pinned-major BtbN win64 **lgpl-shared**
  build under `%APPDATA%\Clipline\ffmpeg`; record the pinned major in code + handoff; ddoc
  caveat notes (LGPL build contents, SVT-AV1 presence, no software HEVC); LGPL §6 attribution
  text in the app/docs.
- [ ] **Verify.** Workspace tests + clippy (after `cargo clean -p` on touched crates); live
  matrix on the dev box: AMF H.264, AMF HEVC, SVT-AV1, MS software MFT (RX 6700 XT is RDNA2 —
  no AV1 hardware encode; NVENC/QSV paths verified by probe unit tests only), each saving a
  clip validated by ffprobe + `avsync`; CI green on ubuntu + windows; handoff updated.

## Manual Test Checklist

- Settings > Recording shows Encoder/Codec selects listing only what this machine offers;
  AV1 and HEVC carry a playback warning badge if the webview can't decode them.
- Auto on the dev box picks AMF + the best decodable codec; recording works and the sidebar
  status shows the active encoder.
- Force AMF HEVC: clip saves, ffprobe shows hevc/hvc1, plays in-app if the HEVC extension is
  installed (clear error otherwise), trim/export of the HEVC clip works.
- Force SVT-AV1: clip saves at plausible CPU cost, ffprobe shows av1/av01, markers + trim work.
- Rename the FFmpeg folder away: app still records via MFT exactly as before; FFmpeg-backed
  options disappear from Settings; no startup error.
- Settings restart honors encoder changes without losing the rolling buffer behavior rules
  (stop clears, settings restart does not emit stale stopped status).
