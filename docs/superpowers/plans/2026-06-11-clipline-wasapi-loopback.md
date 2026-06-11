# Clipline WASAPI Loopback Audio (Windows Platform Layer, Part 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A real `AudioSource` — system audio via WASAPI loopback, encoded to **real Opus**
(handoff milestone 3: "real Opus before shipping" — we do it now rather than PCM-in-Opus-clothing).
**Exit criterion:** `record_smoke --audio` records the live game with sound → finalized MP4 with
h264 + opus streams, ffprobe-sane, fully decodable, audibly correct.

**Architecture:** Platform-neutral first (the discipline that has held since milestone 1):
`opus.rs` wraps `audiopus` (libopus) into 20 ms / 960-sample stereo frame encoding with
`pre_skip` from the encoder lookahead; `pcm.rs` is `LoopbackAssembler` — QPC-stamped PCM chunks
in, continuity-checked interleaved stereo out, **silence inserted on gaps** (WASAPI loopback
goes quiet when nothing renders; without gap fill the MP4 audio timeline — which is
duration-cumulative — would desync), sliced into exact 960-sample frames with derived pts.
Windows side: `windows/wasapi.rs` activates the default render endpoint in loopback shared
mode, drains `IAudioCaptureClient` buffers (QPC positions are 100 ns units — the same timebase
as WGC's `SystemRelativeTime`, anchored through the existing `RelativeClock`), converts the
mix-format float channels to stereo, and composes assembler + Opus behind `AudioSource`.

**Tech Stack:** `audiopus 0.2` (prebuilt libopus on Windows MSVC — verified, no cmake needed
locally; ubuntu CI builds via pkg-config/source and has the toolchain). `windows` features:
`Win32_Media_Audio`, `Win32_Media_KernelStreaming` (float subformat GUID), `Win32_System_Com`
(present).

**Environment notes:** Clock origins: `WgcCapture` still takes its own QPC origin internally;
the smoke anchors the audio clock with `qpc_now_ticks_100ns()` (promoted to `windows/mod.rs`)
right next to capture creation — offset is sub-ms. Unifying one origin across both engines is
milestone 4 (A/V sync hardening), not this plan. Loopback delivers nothing when no audio
renders — the assembler's gap fill covers both that and `AUDCLNT_BUFFERFLAGS_SILENT`. Mix
format is required to be 48 kHz float for v1 (typical Windows default); a resampler is a
follow-up. Device tests CI-skip (no audio endpoint on runners). Commits end with
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: `OpusFrameEncoder` (platform-neutral)

**Files:** Create `crates/clipline-capture/src/opus.rs`; modify `lib.rs`
(dep `audiopus = "0.2"` already added).

- [ ] **Step 1: failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sine_frame() -> Vec<f32> {
        // 960 samples of 440 Hz stereo at 48 kHz, interleaved.
        (0..FRAME_SAMPLES)
            .flat_map(|i| {
                let s = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn encodes_a_20ms_stereo_frame() {
        let mut enc = OpusFrameEncoder::new().expect("opus encoder");
        let packet = enc.encode_frame(&sine_frame()).expect("encode");
        assert!(!packet.is_empty());
        assert!(packet.len() < 1500, "20ms of opus is small, got {}", packet.len());
    }

    #[test]
    fn rejects_wrong_frame_size() {
        let mut enc = OpusFrameEncoder::new().unwrap();
        assert!(enc.encode_frame(&[0.0; 100]).is_err());
    }

    #[test]
    fn exposes_a_sane_pre_skip() {
        let enc = OpusFrameEncoder::new().unwrap();
        let ps = enc.pre_skip();
        assert!(ps > 0 && ps < 1000, "lookahead in 48k samples, got {ps}");
    }

    #[test]
    fn track_config_matches_the_muxer_contract() {
        let enc = OpusFrameEncoder::new().unwrap();
        let cfg = enc.track_config();
        assert_eq!((cfg.channels, cfg.sample_rate), (2, 48_000));
        assert_eq!(cfg.pre_skip, enc.pre_skip());
    }
}
```

- [ ] **Step 2: verify failure → Step 3: implement**

```rust
//! Real Opus encoding (ddoc §4: AV1+Opus default; handoff: real Opus
//! before shipping). Fixed 20 ms stereo frames at 48 kHz — the shape both
//! the muxer (dOps) and WASAPI assembler agree on.

use audiopus::coder::Encoder;
use audiopus::{Application, Channels, SampleRate};
use clipline_mp4::AudioTrackConfig;

/// Samples per channel in one 20 ms frame at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;
/// Interleaved stereo length of one frame.
pub const FRAME_LEN: usize = FRAME_SAMPLES * 2;
pub const FRAME_DURATION_S: f64 = 0.02;

pub struct OpusFrameEncoder {
    encoder: Encoder,
    pre_skip: u16,
}

impl OpusFrameEncoder {
    pub fn new() -> Result<Self, audiopus::Error> {
        let encoder = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        let pre_skip = encoder.lookahead()? as u16; // adjust to audiopus API at compile time
        Ok(Self { encoder, pre_skip })
    }

    /// Encode one interleaved stereo frame (`FRAME_LEN` floats).
    pub fn encode_frame(&mut self, interleaved: &[f32]) -> Result<Vec<u8>, audiopus::Error> {
        // audiopus validates length; surface its error for wrong sizes.
        let mut out = vec![0u8; 4000];
        let n = self.encoder.encode_float(interleaved, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn pre_skip(&self) -> u16 {
        self.pre_skip
    }

    pub fn track_config(&self) -> AudioTrackConfig {
        AudioTrackConfig { channels: 2, sample_rate: 48_000, pre_skip: self.pre_skip }
    }
}
```

`lib.rs`: `pub mod opus;` + re-export `OpusFrameEncoder`.

- [ ] **Step 4: tests pass on this machine** (and later on both CI OSes — the ubuntu build is
the real test of the audiopus dependency). **Step 5: commit**
`feat(capture): real Opus 20ms frame encoder`.

---

### Task 2: `LoopbackAssembler` (platform-neutral)

Continuity engine between QPC-stamped device chunks and exact Opus frames.

**Files:** Create `crates/clipline-capture/src/pcm.rs`; modify `lib.rs`.

Behavior contract (the tests pin it):
- First chunk's pts anchors the stream; frame N's pts = `base + N * 0.02`.
- Chunks are assumed contiguous unless the incoming pts is **more than half a frame (10 ms)**
  ahead of the expected position — then the difference is filled with silence (interleaved
  zeros) before appending. Early/overlapping chunks just append (device jitter is not
  resequenced).
- `pop_frame()` yields `(pts_s, [f32; FRAME_LEN])` once 960 sample-pairs are buffered.
- `extract_stereo(samples, channels)` maps N-channel interleaved to stereo: first two channels,
  or duplicated mono.

- [ ] **Step 1: failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::opus::{FRAME_LEN, FRAME_DURATION_S};

    fn pairs(n: usize, v: f32) -> Vec<f32> {
        vec![v; n * 2]
    }

    #[test]
    fn slices_contiguous_chunks_into_frames() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(1.0, &pairs(960, 0.5));         // exactly one frame
        asm.push_chunk(1.02, &pairs(480, 0.25));       // half a frame
        let (pts, frame) = asm.pop_frame().expect("one full frame");
        assert_eq!(pts, 1.0);
        assert_eq!(frame.len(), FRAME_LEN);
        assert!(frame.iter().all(|&s| s == 0.5));
        assert!(asm.pop_frame().is_none(), "half frame still pending");
        asm.push_chunk(1.03, &pairs(480, 0.25));
        let (pts2, _) = asm.pop_frame().expect("second frame");
        assert!((pts2 - (1.0 + FRAME_DURATION_S)).abs() < 1e-9);
    }

    #[test]
    fn fills_gaps_with_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        // 40 ms gap (nothing rendered): next chunk stamps at 0.06.
        asm.push_chunk(0.06, &pairs(960, 1.0));
        let mut frames = Vec::new();
        while let Some(f) = asm.pop_frame() {
            frames.push(f);
        }
        assert_eq!(frames.len(), 3, "frame + 2 silence-filled + frame boundary");
        assert!(frames[0].1.iter().all(|&s| s == 1.0));
        assert!(frames[1].1.iter().all(|&s| s == 0.0), "gap became silence");
        // pts stays continuous despite the gap.
        assert!((frames[2].0 - 0.04).abs() < 1e-9);
    }

    #[test]
    fn small_jitter_does_not_insert_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        // 5 ms late — within tolerance, treated as contiguous.
        asm.push_chunk(0.025, &pairs(960, 1.0));
        let mut n = 0;
        while asm.pop_frame().is_some() {
            n += 1;
        }
        assert_eq!(n, 2, "no silence frames inserted");
    }

    #[test]
    fn extract_stereo_handles_channel_counts() {
        // Stereo passes through.
        assert_eq!(extract_stereo(&[1.0, 2.0, 3.0, 4.0], 2), vec![1.0, 2.0, 3.0, 4.0]);
        // 5.1 keeps front L/R.
        let six: Vec<f32> = (0..12).map(|i| i as f32).collect();
        assert_eq!(extract_stereo(&six, 6), vec![0.0, 1.0, 6.0, 7.0]);
        // Mono duplicates.
        assert_eq!(extract_stereo(&[0.5, 0.7], 1), vec![0.5, 0.5, 0.7, 0.7]);
    }
}
```

- [ ] **Step 2: verify failure → Step 3: implement**

```rust
//! PCM continuity between QPC-stamped WASAPI chunks and exact Opus frames.
//! Loopback goes quiet when nothing renders; the MP4 audio timeline is
//! duration-cumulative, so gaps MUST become silence or A/V desyncs.

use crate::opus::{FRAME_DURATION_S, FRAME_LEN};

const SAMPLE_RATE: f64 = 48_000.0;
/// Gaps shorter than half a frame are treated as device jitter.
const GAP_TOLERANCE_S: f64 = FRAME_DURATION_S / 2.0;

#[derive(Default)]
pub struct LoopbackAssembler {
    /// pts of buffered sample-pair 0 == base + popped frames * 0.02.
    base_pts_s: Option<f64>,
    buffered: Vec<f32>, // interleaved stereo
    frames_popped: u64,
}

impl LoopbackAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// `pts_s` stamps the first sample of `interleaved` (stereo pairs).
    pub fn push_chunk(&mut self, pts_s: f64, interleaved: &[f32]) {
        let base = *self.base_pts_s.get_or_insert(pts_s);
        let expected =
            base + (self.frames_popped as f64 * FRAME_DURATION_S)
                + (self.buffered.len() / 2) as f64 / SAMPLE_RATE;
        let gap = pts_s - expected;
        if gap > GAP_TOLERANCE_S {
            let missing_pairs = (gap * SAMPLE_RATE).round() as usize;
            self.buffered.extend(std::iter::repeat(0.0).take(missing_pairs * 2));
        }
        self.buffered.extend_from_slice(interleaved);
    }

    /// One 20 ms frame once enough samples are buffered.
    pub fn pop_frame(&mut self) -> Option<(f64, Vec<f32>)> {
        if self.buffered.len() < FRAME_LEN {
            return None;
        }
        let pts = self.base_pts_s? + self.frames_popped as f64 * FRAME_DURATION_S;
        let frame: Vec<f32> = self.buffered.drain(..FRAME_LEN).collect();
        self.frames_popped += 1;
        Some((pts, frame))
    }
}

/// First two channels of N-channel interleaved float PCM (mono duplicates).
pub fn extract_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mut out = Vec::with_capacity(samples.len() / ch * 2);
    for frame in samples.chunks_exact(ch) {
        out.push(frame[0]);
        out.push(if ch >= 2 { frame[1] } else { frame[0] });
    }
    out
}
```

`lib.rs`: `pub mod pcm;` + re-exports.

- [ ] **Step 4: tests pass → Step 5: commit**
`feat(capture): loopback PCM assembler with silence gap fill`.

---

### Task 3: `WasapiLoopback` behind `AudioSource`

**Files:** Create `crates/clipline-capture/src/windows/wasapi.rs`; modify `windows/mod.rs`
(also promote `qpc_now_ticks_100ns` from `wgc.rs` to `windows/mod.rs` as `pub fn`),
`Cargo.toml` (features `Win32_Media_Audio`, `Win32_Media_KernelStreaming`).

Construction (`WasapiLoopback::start(clock: RelativeClock)`):
1. Best-effort `CoInitializeEx(MTA)` (RPC_E_CHANGED_MODE ok — same pattern as `init_winrt`).
2. `CoCreateInstance(MMDeviceEnumerator)` → `GetDefaultAudioEndpoint(eRender, eConsole)` →
   `Activate::<IAudioClient>()`.
3. `GetMixFormat` (CoTaskMem; read then free). Require float32 (IEEE_FLOAT tag or EXTENSIBLE +
   `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`) at 48 kHz — else `CaptureError::Init` with the actual
   format in the message. Keep `channels` for `extract_stereo`.
4. `Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 10_000_000 /*1 s*/, 0,
   mix_format, None)` → `GetService::<IAudioCaptureClient>()` → `Start()`.

`poll_packets(until_pts_s)`:
```text
drain: while GetNextPacketSize()? > 0:
    GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_100ns))
    pts = clock.pts_s(qpc_100ns as i64)
    floats = if flags & AUDCLNT_BUFFERFLAGS_SILENT { zeros } else { raw f32 slice copy }
    assembler.push_chunk(pts, &extract_stereo(&floats, channels))
    ReleaseBuffer(frames)
while let Some((pts, frame)) = assembler.pop_frame():
    queue.push(AudioPacket { data: opus.encode_frame(&frame)?, pts_s: pts, duration_s: 0.02 })
return queue packets with pts + duration <= until + 1e-9 (drain split, mock semantics)
```
`track_config()` delegates to the Opus encoder. `Drop` stops the client.

- [ ] **Step 1: failing test** (CI-skipped; lenient about silence — an idle desktop may
deliver nothing, which is exactly what the gap fill is for):

```rust
    #[test]
    fn captures_system_loopback_audio() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: audio endpoint test");
            return;
        }
        let clock = RelativeClock::new(crate::windows::qpc_now_ticks_100ns().unwrap());
        let mut src = match WasapiLoopback::start(clock) {
            Ok(s) => s,
            Err(e) => { eprintln!("SKIP: loopback unavailable: {e}"); return; }
        };
        let cfg = src.track_config();
        assert_eq!((cfg.channels, cfg.sample_rate), (2, 48_000));
        assert!(cfg.pre_skip > 0);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let packets = src.poll_packets(f64::MAX).expect("poll");
        for w in packets.windows(2) {
            assert!((w[1].pts_s - w[0].pts_s - 0.02).abs() < 1e-6, "20 ms cadence");
        }
        for p in &packets { assert!(!p.data.is_empty()); }
        eprintln!("captured {} opus packets in 300 ms", packets.len());
    }
```

- [ ] **Step 2: verify failure → Step 3: implement → Step 4: device test runs for real
locally** (League is usually making noise on this machine) **→ Step 5: commit**
`feat(capture): WASAPI system loopback behind AudioSource with real Opus`.

---

### Task 4: A/V end-to-end (`record_smoke --audio`)

**Files:** Modify `crates/clipline-capture/examples/record_smoke.rs`.

- [ ] Add `--audio` flag: when set, anchor a `RelativeClock` at `qpc_now_ticks_100ns()` just
before building the recorder, `WasapiLoopback::start(clock)`, and attach via
`Recorder::with_audio`. Print the opus packet count after saving.
- [ ] Run: `cargo run -p clipline-capture --example record_smoke -- --seconds 5 --window league --audio`
  while game audio is playing.
- [ ] Verify: ffprobe shows `h264` + `opus` streams with sane durations (within ~0.1 s of each
  other); full ffmpeg decode clean; extract a 1 s wav (`ffmpeg -i out.mp4 -map 0:a -t 1 t.wav`)
  and check it is not all zeros (RMS > 0) while audio was playing. Leave the MP4 for the human
  to hear.
- [ ] Commit with observed output: `feat(capture): record_smoke --audio - A/V e2e`.

---

### Task 5: quality gates

- [ ] `cargo test --workspace` green locally (ffprobe now installed — the mp4/audio e2e tests
  run for real).
- [ ] `cargo clippy --workspace --all-targets` zero warnings.
- [ ] Push; CI green on **both** OSes — ubuntu compiling `audiopus_sys` is the new risk; if the
  runner lacks libopus headers it builds from source (cmake is present). Fix forward if not.
- [ ] Update `handoff.md`: milestone 3 done, milestone 4 (A/V sync hardening) next.

---

## Out of scope (follow-ups)

- Per-process loopback (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`) — second per the
  handoff; needs the fixed-format assumption (ddoc Caveats: `GetMixFormat` is `E_NOTIMPL`).
- Resampling non-48 kHz mix formats; >2-channel downmix beyond front L/R.
- Microphone capture track; multi-track UI selection.
- One shared QPC origin across video + audio engines and drift handling — milestone 4.
