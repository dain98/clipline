# Clipline Audio-Track Muxing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend `clipline-mp4` from single-video to multi-track (ddoc §10 multi-track output: game / mic / system audio), with Opus audio tracks (ddoc §4's royalty-free AV1+Opus default), validated by ffprobe seeing both an h264 and an opus stream in one finalized Hybrid MP4.

**Architecture:** Generalize in three layers, preserving the existing public single-track API as wrappers so `clipline-capture` and all current tests keep passing unchanged. (1) `init.rs` gains `AudioTrackConfig`, the `Opus` sample entry with its `dOps` box (Opus-in-ISOBMFF: not a full box), `smhd`/`soun` audio trak builders, parameterized track IDs, and `moov_init_multi`. (2) `fragment.rs` gains `fragment_multi(seq, &[TrackRun])` — one `moof` containing one `traf` per track, one shared `mdat`, per-`traf` data offsets computed by the same two-pass build. (3) `writer.rs` moves per-sample bookkeeping into a per-track `TrackState`, adds `new_multi`/`write_fragment_multi`, and finalize emits one fully-tabled `trak` per track with the movie duration = the longest track.

**Tech Stack:** unchanged (std-only crate; ffprobe e2e with skip-if-absent).

**Spec notes:**
- Audio sample entry layout: 6 reserved bytes, `data_reference_index=1`, 8 reserved bytes, `channelcount`, `samplesize=16`, `pre_defined=0`, `reserved=0`, `samplerate` as 16.16 fixed (`rate << 16`).
- `dOps` payload: `u8 version=0`, `u8 OutputChannelCount`, `u16 PreSkip`, `u32 InputSampleRate` (true rate, not shifted), `i16 OutputGain=0`, `u8 ChannelMappingFamily=0`.
- Audio `mdhd` timescale = sample rate; Opus frames are all sync samples → no `stss` (absent ⇒ all sync, already handled).
- `smhd` full box: `i16 balance=0` + `u16 reserved`.

**Environment notes:** `cargo` at `~/.cargo/bin/cargo`. Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: AudioTrackConfig + Opus sample entry + audio trak

**Files:**
- Modify: `crates/clipline-mp4/src/init.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: extend `#[cfg(test)]` in `init.rs`

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `init.rs`)

```rust
    fn audio_cfg() -> AudioTrackConfig {
        AudioTrackConfig { channels: 2, sample_rate: 48_000, pre_skip: 312 }
    }

    #[test]
    fn audio_trak_uses_soun_handler_and_smhd() {
        let buf = audio_trak_with_tables(&audio_cfg(), 2, 0, 0, empty_stbl_tail());
        assert!(buf.windows(4).any(|w| w == b"soun"));
        assert!(buf.windows(4).any(|w| w == b"smhd"));
        assert!(!buf.windows(4).any(|w| w == b"vmhd"));
    }

    #[test]
    fn audio_stsd_embeds_opus_and_dops() {
        let buf = audio_trak_with_tables(&audio_cfg(), 2, 0, 0, empty_stbl_tail());
        assert!(buf.windows(4).any(|w| w == b"Opus"));
        // dOps payload: ver=0, ch=2, pre_skip=312 (0x0138), rate=48000
        // (0x0000BB80), gain=0, mapping=0.
        let dops: &[u8] = &[
            b'd', b'O', b'p', b's', 0, 2, 0x01, 0x38, 0x00, 0x00, 0xBB, 0x80, 0, 0, 0,
        ];
        assert!(buf.windows(dops.len()).any(|w| w == dops));
    }

    #[test]
    fn track_ids_are_parameterized() {
        let buf = audio_trak_with_tables(&audio_cfg(), 7, 0, 0, empty_stbl_tail());
        // tkhd payload: version/flags(4) creation(4) modification(4) track_ID(4)
        let top = walk(&buf);
        let kids = children(&buf, &top[0]);
        let tkhd = find(&kids, b"tkhd").unwrap();
        let p = tkhd.payload_offset as usize;
        let id = u32::from_be_bytes(buf[p + 12..p + 16].try_into().unwrap());
        assert_eq!(id, 7);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: COMPILE ERROR (`AudioTrackConfig` / `audio_trak_with_tables` not defined; `empty_stbl_tail` not visible to tests).

- [ ] **Step 3: Implement**

In `init.rs`:

Add after `VideoTrackConfig`:
```rust
/// Opus audio track parameters (ddoc §4/§10). Track timescale = sample rate.
#[derive(Debug, Clone)]
pub struct AudioTrackConfig {
    pub channels: u16,
    pub sample_rate: u32,
    /// Opus pre-skip in 48 kHz samples (dOps PreSkip).
    pub pre_skip: u16,
}
```

Make `empty_stbl_tail` `pub(crate)` (tests and the multi-track writer need it):
```rust
pub(crate) fn empty_stbl_tail() -> Vec<u8> {
```

Generalize `tkhd` (replace the existing private fn — the video path passes its width/height and volume 0; audio passes volume 0x0100 and zero dimensions):
```rust
fn tkhd(track_id: u32, duration_movie_ts: u64, volume: u16, width: u16, height: u16) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(0).u32(0) // creation/modification
        .u32(track_id)
        .u32(0) // reserved
        .u32(duration_movie_ts as u32)
        .u32(0)
        .u32(0) // reserved
        .u16(0) // layer
        .u16(0) // alternate_group
        .u16(volume)
        .u16(0); // reserved
    for m in MATRIX {
        p.u32(m);
    }
    p.u32((width as u32) << 16).u32((height as u32) << 16);
    full_box(*b"tkhd", 0, 0x000003, p.into_vec())
}
```

Generalize `mdia`/`minf` into shared helpers and rebuild the video/audio traks on top (replace the existing `trak_with_tables`, `mdia`, `minf` bodies):
```rust
/// Video trak with populated sample tables. Existing callers pass track 1.
pub fn trak_with_tables(
    cfg: &VideoTrackConfig,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    video_trak_with_tables(cfg, 1, duration_movie_ts, duration_media_ts, stbl_tail)
}

pub fn video_trak_with_tables(
    cfg: &VideoTrackConfig,
    track_id: u32,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    let mut v = Payload::new();
    v.u16(0).u16(0).u16(0).u16(0); // graphicsmode + opcolor
    let vmhd = full_box(*b"vmhd", 0, 1, v.into_vec());

    let mut t = tkhd(track_id, duration_movie_ts, 0, cfg.width, cfg.height);
    t.extend(mdia_generic(
        cfg.timescale,
        duration_media_ts,
        *b"vide",
        b"Clipline Video\0",
        vmhd,
        stsd(cfg),
        stbl_tail,
    ));
    mp4_box(*b"trak", t)
}

pub fn audio_trak_with_tables(
    cfg: &AudioTrackConfig,
    track_id: u32,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    let mut s = Payload::new();
    s.u16(0).u16(0); // balance + reserved
    let smhd = full_box(*b"smhd", 0, 0, s.into_vec());

    let mut t = tkhd(track_id, duration_movie_ts, 0x0100, 0, 0);
    t.extend(mdia_generic(
        cfg.sample_rate,
        duration_media_ts,
        *b"soun",
        b"Clipline Audio\0",
        smhd,
        audio_stsd(cfg),
        stbl_tail,
    ));
    mp4_box(*b"trak", t)
}

#[allow(clippy::too_many_arguments)]
fn mdia_generic(
    timescale: u32,
    duration_media_ts: u64,
    handler: [u8; 4],
    handler_name: &[u8],
    media_header_box: Vec<u8>,
    stsd_box: Vec<u8>,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(0).u32(0).u32(timescale).u32(duration_media_ts as u32)
        .u16(0x55C4) // language: und
        .u16(0);
    let mdhd = full_box(*b"mdhd", 0, 0, p.into_vec());

    let mut h = Payload::new();
    h.u32(0).bytes(&handler).u32(0).u32(0).u32(0).bytes(handler_name);
    let hdlr = full_box(*b"hdlr", 0, 0, h.into_vec());

    let url = full_box(*b"url ", 0, 1, vec![]); // self-contained
    let mut d = Payload::new();
    d.u32(1); // entry_count
    let mut dref_payload = d.into_vec();
    dref_payload.extend(url);
    let dref = full_box(*b"dref", 0, 0, dref_payload);
    let dinf = mp4_box(*b"dinf", dref);

    let mut stbl = stsd_box;
    stbl.extend(stbl_tail);

    let mut minf = media_header_box;
    minf.extend(dinf);
    minf.extend(mp4_box(*b"stbl", stbl));

    let mut m = mdhd;
    m.extend(hdlr);
    m.extend(mp4_box(*b"minf", minf));
    mp4_box(*b"mdia", m)
}

fn audio_stsd(cfg: &AudioTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(1); // entry_count
    let mut payload = p.into_vec();
    payload.extend(opus_sample_entry(cfg));
    full_box(*b"stsd", 0, 0, payload)
}

fn opus_sample_entry(cfg: &AudioTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(&[0; 6]) // reserved
        .u16(1) // data_reference_index
        .u32(0)
        .u32(0) // reserved
        .u16(cfg.channels)
        .u16(16) // samplesize
        .u16(0) // pre_defined
        .u16(0) // reserved
        .u32(cfg.sample_rate << 16); // 16.16 fixed
    let mut payload = p.into_vec();
    payload.extend(dops(cfg));
    mp4_box(*b"Opus", payload)
}

/// Opus-in-ISOBMFF `dOps` box (plain box, NOT a full box).
fn dops(cfg: &AudioTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.u8(0) // version
        .u8(cfg.channels as u8) // OutputChannelCount
        .u16(cfg.pre_skip)
        .u32(cfg.sample_rate) // InputSampleRate (true rate)
        .u16(0) // OutputGain (i16 0)
        .u8(0); // ChannelMappingFamily
    mp4_box(*b"dOps", p.into_vec())
}
```

Delete the now-replaced private `mdia`/`minf` video-only fns and the old `trak` body; `trak(cfg, dm, dmed)` (used by `moov_init`) becomes:
```rust
pub fn trak(cfg: &VideoTrackConfig, duration_movie_ts: u64, duration_media_ts: u64) -> Vec<u8> {
    video_trak_with_tables(cfg, 1, duration_movie_ts, duration_media_ts, empty_stbl_tail())
}
```

Export from `lib.rs`:
```rust
pub use init::{AudioTrackConfig, VideoTrackConfig};
```

- [ ] **Step 4: Run tests to verify everything passes** (old 16 + 3 new)

Run: `~/.cargo/bin/cargo test -p clipline-mp4`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): Opus audio trak with dOps sample entry

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Multi-track init moov

**Files:**
- Modify: `crates/clipline-mp4/src/init.rs`, `crates/clipline-mp4/src/writer.rs` (mvhd call site), `crates/clipline-mp4/src/lib.rs`
- Test: extend `#[cfg(test)]` in `init.rs`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn multi_track_moov_has_one_trak_and_trex_per_track() {
        let tracks = vec![
            TrackConfig::Video(cfg()),
            TrackConfig::Audio(audio_cfg()),
        ];
        let buf = moov_init_multi(&tracks);
        let top = walk(&buf);
        let kids = children(&buf, &top[0]);
        let traks: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"trak").collect();
        assert_eq!(traks.len(), 2);
        let mvex = find(&kids, b"mvex").unwrap();
        let trexes = children(&buf, mvex);
        assert_eq!(trexes.len(), 2);
        // trex payload: version/flags(4) then track_ID.
        let p = trexes[1].payload_offset as usize;
        assert_eq!(u32::from_be_bytes(buf[p + 4..p + 8].try_into().unwrap()), 2);
    }
```

- [ ] **Step 2: Run to verify failure** (`TrackConfig`/`moov_init_multi` undefined)

- [ ] **Step 3: Implement** (in `init.rs`)

```rust
/// One track in a multi-track recording (ddoc §10: video + game/mic/system).
#[derive(Debug, Clone)]
pub enum TrackConfig {
    Video(VideoTrackConfig),
    Audio(AudioTrackConfig),
}

impl TrackConfig {
    /// Media timescale: sample durations for this track use these ticks.
    pub fn timescale(&self) -> u32 {
        match self {
            TrackConfig::Video(v) => v.timescale,
            TrackConfig::Audio(a) => a.sample_rate,
        }
    }
}

/// Fragmented-init moov for N tracks; track IDs are 1-based positions.
pub fn moov_init_multi(tracks: &[TrackConfig]) -> Vec<u8> {
    let mut moov = mvhd(0, tracks.len() as u32 + 1);
    for (i, t) in tracks.iter().enumerate() {
        let id = i as u32 + 1;
        moov.extend(match t {
            TrackConfig::Video(v) => video_trak_with_tables(v, id, 0, 0, empty_stbl_tail()),
            TrackConfig::Audio(a) => audio_trak_with_tables(a, id, 0, 0, empty_stbl_tail()),
        });
    }
    moov.extend(mvex_multi(tracks.len() as u32));
    mp4_box(*b"moov", moov)
}

fn mvex_multi(track_count: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    for id in 1..=track_count {
        let mut p = Payload::new();
        p.u32(id).u32(1).u32(0).u32(0).u32(0);
        payload.extend(full_box(*b"trex", 0, 0, p.into_vec()));
    }
    mp4_box(*b"mvex", payload)
}
```

`mvhd` gains the `next_track_id` parameter (replace `p.u32(2); // next_track_ID` with the param):
```rust
pub fn mvhd(duration_movie_ts: u64, next_track_id: u32) -> Vec<u8> {
```
…and at the end: `p.u32(next_track_id);`

Update the two call sites: `moov_init` → `mvhd(0, 2)` and delete its inline `mvex()` (keep `moov_init` as `moov_init_multi(&[TrackConfig::Video(cfg.clone())])` for exact equivalence); `writer.rs::final_moov` → `mvhd(duration_movie, 2)` (rewritten fully in Task 4 anyway). Delete the old single `mvex()`.

Export `TrackConfig` and `moov_init_multi` from `lib.rs`.

- [ ] **Step 4: Run tests** — all pass (old single-track tests included).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): multi-track init moov with per-track trex

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Multi-traf fragments

**Files:**
- Modify: `crates/clipline-mp4/src/fragment.rs`, `crates/clipline-mp4/src/lib.rs`
- Test: extend `#[cfg(test)]` in `fragment.rs`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn multi_track_fragment_has_one_traf_per_track() {
        let video = samples();
        let audio = vec![FragSample { data: b"OPUSPKT1".to_vec(), duration: 960, is_sync: true }];
        let runs = [
            TrackRun { track_id: 1, base_decode_time: 0, samples: &video },
            TrackRun { track_id: 2, base_decode_time: 0, samples: &audio },
        ];
        let buf = fragment_multi(9, &runs);
        let boxes = walk(&buf);
        let kids = children(&buf, &boxes[0]);
        let trafs: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"traf").collect();
        assert_eq!(trafs.len(), 2);

        // Each traf's trun data_offset points at that track's first byte
        // within the shared mdat.
        for (traf, expected) in trafs.iter().zip([b"KEYFRAME".as_slice(), b"OPUSPKT1".as_slice()]) {
            let tk = children(&buf, traf);
            let trun = find(&tk, b"trun").unwrap();
            let p = trun.payload_offset as usize;
            let off = i32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()) as usize;
            assert_eq!(&buf[off..off + expected.len()], expected);
        }
    }
```

- [ ] **Step 2: Run to verify failure** (`TrackRun`/`fragment_multi` undefined)

- [ ] **Step 3: Implement** (in `fragment.rs`)

```rust
/// One track's slice of a fragment.
#[derive(Debug)]
pub struct TrackRun<'a> {
    pub track_id: u32,
    /// In this track's timescale ticks.
    pub base_decode_time: u64,
    pub samples: &'a [FragSample],
}

/// One `moof` with a `traf` per run, plus one shared `mdat` holding all
/// runs' samples in run order.
pub fn fragment_multi(sequence: u32, runs: &[TrackRun<'_>]) -> Vec<u8> {
    // Two-pass: data offsets depend on the moof's size, which is stable.
    let zeros = vec![0i32; runs.len()];
    let moof = build_moof_multi(sequence, runs, &zeros);
    let mut offsets = Vec::with_capacity(runs.len());
    let mut acc = (moof.len() + 8) as i32; // + mdat header
    for r in runs {
        offsets.push(acc);
        acc += r.samples.iter().map(|s| s.data.len()).sum::<usize>() as i32;
    }
    let moof = build_moof_multi(sequence, runs, &offsets);

    let mut mdat_payload = Vec::new();
    for r in runs {
        for s in r.samples {
            mdat_payload.extend_from_slice(&s.data);
        }
    }
    let mut out = moof;
    out.extend(mp4_box(*b"mdat", mdat_payload));
    out
}

/// Single-track fragment (track 1) — the original API.
pub fn fragment(sequence: u32, base_decode_time: u64, samples: &[FragSample]) -> Vec<u8> {
    fragment_multi(sequence, &[TrackRun { track_id: 1, base_decode_time, samples }])
}

fn build_moof_multi(sequence: u32, runs: &[TrackRun<'_>], data_offsets: &[i32]) -> Vec<u8> {
    let mut mfhd_p = Payload::new();
    mfhd_p.u32(sequence);
    let mut moof = full_box(*b"mfhd", 0, 0, mfhd_p.into_vec());
    for (run, &off) in runs.iter().zip(data_offsets) {
        moof.extend(traf(run, off));
    }
    mp4_box(*b"moof", moof)
}

fn traf(run: &TrackRun<'_>, data_offset: i32) -> Vec<u8> {
    let mut tfhd_p = Payload::new();
    tfhd_p.u32(run.track_id);
    let tfhd = full_box(*b"tfhd", 0, 0x020000, tfhd_p.into_vec()); // default-base-is-moof

    let mut tfdt_p = Payload::new();
    tfdt_p.u64(run.base_decode_time);
    let tfdt = full_box(*b"tfdt", 1, 0, tfdt_p.into_vec());

    // flags: data-offset | sample-duration | sample-size | sample-flags
    let mut trun_p = Payload::new();
    trun_p.u32(run.samples.len() as u32).i32(data_offset);
    for s in run.samples {
        trun_p.u32(s.duration).u32(s.data.len() as u32).u32(if s.is_sync {
            FLAG_SYNC
        } else {
            FLAG_NON_SYNC
        });
    }
    let trun = full_box(*b"trun", 0, 0x000701, trun_p.into_vec());

    let mut t = tfhd;
    t.extend(tfdt);
    t.extend(trun);
    mp4_box(*b"traf", t)
}
```

Delete the old `build_moof` (replaced). Export `TrackRun` and `fragment_multi` from `lib.rs` (`pub use fragment::{FragSample, TrackRun};` etc.).

- [ ] **Step 4: Run tests** — all pass including the original single-track fragment tests (wrapper preserves layout: one traf, identical boxes).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): multi-traf fragments with shared mdat

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Multi-track writer + ffprobe e2e

**Files:**
- Modify: `crates/clipline-mp4/src/writer.rs`, `crates/clipline-mp4/src/lib.rs`
- Test: `crates/clipline-mp4/tests/multitrack.rs`

- [ ] **Step 1: Write the failing test** (`tests/multitrack.rs`)

```rust
use std::io::Cursor;
use std::process::Command;

use clipline_mp4::walker::{children, find, walk};
use clipline_mp4::{
    AudioTrackConfig, FragSample, HybridMp4Writer, TrackConfig, VideoTrackConfig,
};

fn tracks() -> Vec<TrackConfig> {
    vec![
        TrackConfig::Video(VideoTrackConfig {
            width: 128,
            height: 128,
            timescale: 90_000,
            sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            pps: vec![0x68, 0xEE, 0x38, 0x80],
        }),
        TrackConfig::Audio(AudioTrackConfig { channels: 2, sample_rate: 48_000, pre_skip: 312 }),
    ]
}

fn video_gop(start: u32, frames: u32) -> Vec<FragSample> {
    (0..frames)
        .map(|i| FragSample {
            data: format!("V{:05}", start + i).into_bytes(),
            duration: 3000, // 30 fps @ 90 kHz
            is_sync: i == 0,
        })
        .collect()
}

fn audio_packets(start: u32, count: u32) -> Vec<FragSample> {
    (0..count)
        .map(|i| FragSample {
            data: format!("A{:05}", start + i).into_bytes(),
            duration: 960, // 20 ms @ 48 kHz
            is_sync: true,
        })
        .collect()
}

fn mux_2s() -> Vec<u8> {
    let mut w = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), tracks()).unwrap();
    // 2 fragments of 1 s each: 30 video frames + 50 audio packets per fragment.
    for f in 0..2u32 {
        let v = video_gop(f * 30, 30);
        let a = audio_packets(f * 50, 50);
        w.write_fragment_multi(&[&v, &a]).unwrap();
    }
    w.finalize().unwrap().into_inner()
}

#[test]
fn finalized_multitrack_file_has_two_fully_tabled_traks() {
    let buf = mux_2s();
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);

    let moov = find(&boxes, b"moov").unwrap();
    let kids = children(&buf, moov);
    let traks: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"trak").collect();
    assert_eq!(traks.len(), 2);
    assert!(find(&kids, b"mvex").is_none());

    // Both tracks' first samples are reachable: video "V00000", audio "A00000".
    assert!(buf.windows(6).any(|w| w == b"V00000"));
    assert!(buf.windows(6).any(|w| w == b"A00000"));
}

#[test]
fn single_track_write_fragment_still_works() {
    let cfg = match &tracks()[0] {
        TrackConfig::Video(v) => v.clone(),
        _ => unreachable!(),
    };
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg).unwrap();
    w.write_fragment(&video_gop(0, 30)).unwrap();
    let buf = w.finalize().unwrap().into_inner();
    let boxes = walk(&buf);
    assert_eq!(boxes.len(), 3); // ftyp mdat moov
}

#[test]
fn ffprobe_sees_h264_and_opus_streams() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found");
        return;
    };
    let buf = mux_2s();
    let path = std::env::temp_dir().join("clipline_multitrack.mp4");
    std::fs::write(&path, &buf).unwrap();
    let out = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,nb_frames",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "ffprobe failed: {stderr}");
    assert!(stdout.contains("codec_name=h264"), "got: {stdout}");
    assert!(stdout.contains("codec_name=opus"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=100"), "got: {stdout}");
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");
    std::fs::remove_file(&path).ok();
}

fn ffprobe_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let local = std::path::Path::new(&home).join("bin/ffprobe");
    if local.exists() {
        return Some(local);
    }
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| std::path::Path::new(d).join("ffprobe"))
        .find(|p| p.exists())
}
```

- [ ] **Step 2: Run to verify failure** (`new_multi`/`write_fragment_multi`/`TrackConfig` not exported)

- [ ] **Step 3: Rewrite `writer.rs`**

```rust
use std::io::{self, Seek, SeekFrom, Write};

use crate::boxes::{full_box, mp4_box, Payload};
use crate::fragment::{fragment_multi, FragSample, TrackRun};
use crate::init::{
    audio_trak_with_tables, free_placeholder, ftyp, moov_init_multi, mvhd,
    video_trak_with_tables, TrackConfig, VideoTrackConfig, MOVIE_TIMESCALE,
};

/// Per-track bookkeeping for the final moov.
struct TrackState {
    cfg: TrackConfig,
    next_decode_time: u64,
    sizes: Vec<u32>,
    durations: Vec<u32>,
    sync: Vec<bool>,
    /// (absolute offset of first sample byte, sample count) per fragment
    /// in which this track had samples.
    chunks: Vec<(u64, u32)>,
}

/// Streaming Hybrid MP4 writer (ddoc §10). While recording the file is a
/// fragmented MP4 (crash-safe); `finalize()` turns it into a standard
/// seekable MP4 in place. Supports N tracks (video + audio).
pub struct HybridMp4Writer<W: Write + Seek> {
    w: W,
    tracks: Vec<TrackState>,
    free_offset: u64,
    next_sequence: u32,
}

impl<W: Write + Seek> HybridMp4Writer<W> {
    /// Single video track (original API).
    pub fn new(w: W, cfg: VideoTrackConfig) -> io::Result<Self> {
        Self::new_multi(w, vec![TrackConfig::Video(cfg)])
    }

    pub fn new_multi(mut w: W, tracks: Vec<TrackConfig>) -> io::Result<Self> {
        let ftyp = ftyp();
        w.write_all(&ftyp)?;
        let free_offset = ftyp.len() as u64;
        w.write_all(&free_placeholder())?;
        w.write_all(&moov_init_multi(&tracks))?;
        Ok(Self {
            w,
            tracks: tracks
                .into_iter()
                .map(|cfg| TrackState {
                    cfg,
                    next_decode_time: 0,
                    sizes: Vec::new(),
                    durations: Vec::new(),
                    sync: Vec::new(),
                    chunks: Vec::new(),
                })
                .collect(),
            free_offset,
            next_sequence: 1,
        })
    }

    /// Single-track fragment write (original API; requires exactly 1 track).
    pub fn write_fragment(&mut self, samples: &[FragSample]) -> io::Result<()> {
        self.write_fragment_multi(&[samples])
    }

    /// One fragment carrying samples for each track, positionally aligned
    /// with the track list. Empty slices are allowed (track sat this
    /// fragment out).
    pub fn write_fragment_multi(&mut self, per_track: &[&[FragSample]]) -> io::Result<()> {
        if per_track.len() != self.tracks.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("expected {} track slices, got {}", self.tracks.len(), per_track.len()),
            ));
        }
        if per_track.iter().all(|s| s.is_empty()) {
            return Ok(());
        }

        let runs: Vec<TrackRun<'_>> = per_track
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_empty())
            .map(|(i, s)| TrackRun {
                track_id: i as u32 + 1,
                base_decode_time: self.tracks[i].next_decode_time,
                samples: s,
            })
            .collect();

        let frag = fragment_multi(self.next_sequence, &runs);
        let frag_start = self.w.stream_position()?;
        let total_payload: usize = runs
            .iter()
            .flat_map(|r| r.samples.iter())
            .map(|s| s.data.len())
            .sum();
        let moof_len = frag.len() - (8 + total_payload);
        self.w.write_all(&frag)?;

        // Record chunk offsets in run order (same order the mdat was laid out).
        let mut sample_offset = frag_start + moof_len as u64 + 8;
        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            let state = &mut self.tracks[idx];
            state.chunks.push((sample_offset, run.samples.len() as u32));
            for s in run.samples {
                state.sizes.push(s.data.len() as u32);
                state.durations.push(s.duration);
                state.sync.push(s.is_sync);
                state.next_decode_time += s.duration as u64;
                sample_offset += s.data.len() as u64;
            }
        }
        self.next_sequence += 1;
        Ok(())
    }

    /// Append the full moov, then overwrite the leading free box with a
    /// largesize mdat header hiding init-moov + fragments (ddoc §10).
    pub fn finalize(mut self) -> io::Result<W> {
        let moov_offset = self.w.stream_position()?;
        let moov = self.final_moov();
        self.w.write_all(&moov)?;

        let hidden_span = moov_offset - self.free_offset;
        self.w.seek(SeekFrom::Start(self.free_offset))?;
        let mut hdr = Payload::new();
        hdr.u32(1).bytes(b"mdat").u64(hidden_span);
        self.w.write_all(&hdr.into_vec())?;
        self.w.flush()?;
        Ok(self.w)
    }

    /// Abort without finalizing (crash-simulation / tests).
    pub fn into_inner(self) -> W {
        self.w
    }

    fn final_moov(&self) -> Vec<u8> {
        let duration_movie = self
            .tracks
            .iter()
            .map(|t| t.duration_movie_ts())
            .max()
            .unwrap_or(0);

        let mut moov = mvhd(duration_movie, self.tracks.len() as u32 + 1);
        for (i, t) in self.tracks.iter().enumerate() {
            moov.extend(t.trak(i as u32 + 1, duration_movie));
        }
        mp4_box(*b"moov", moov)
    }
}

impl TrackState {
    fn duration_media_ts(&self) -> u64 {
        self.durations.iter().map(|&d| d as u64).sum()
    }

    fn duration_movie_ts(&self) -> u64 {
        self.duration_media_ts() * MOVIE_TIMESCALE as u64 / self.cfg.timescale() as u64
    }

    fn trak(&self, track_id: u32, duration_movie: u64) -> Vec<u8> {
        let mut tail = self.stts();
        if let Some(stss) = self.stss() {
            tail.extend(stss);
        }
        tail.extend(self.stsc());
        tail.extend(self.stsz());
        tail.extend(self.co64());
        let media = self.duration_media_ts();
        match &self.cfg {
            TrackConfig::Video(v) => {
                video_trak_with_tables(v, track_id, duration_movie, media, tail)
            }
            TrackConfig::Audio(a) => {
                audio_trak_with_tables(a, track_id, duration_movie, media, tail)
            }
        }
    }

    fn stts(&self) -> Vec<u8> {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for &d in &self.durations {
            match runs.last_mut() {
                Some((count, delta)) if *delta == d => *count += 1,
                _ => runs.push((1, d)),
            }
        }
        let mut p = Payload::new();
        p.u32(runs.len() as u32);
        for (count, delta) in runs {
            p.u32(count).u32(delta);
        }
        full_box(*b"stts", 0, 0, p.into_vec())
    }

    /// None when every sample is sync (spec: absent stss ⇒ all sync).
    fn stss(&self) -> Option<Vec<u8>> {
        if self.sync.iter().all(|&s| s) {
            return None;
        }
        let syncs: Vec<u32> = self
            .sync
            .iter()
            .enumerate()
            .filter(|(_, &s)| s)
            .map(|(i, _)| i as u32 + 1)
            .collect();
        let mut p = Payload::new();
        p.u32(syncs.len() as u32);
        for s in syncs {
            p.u32(s);
        }
        Some(full_box(*b"stss", 0, 0, p.into_vec()))
    }

    fn stsc(&self) -> Vec<u8> {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for (i, &(_, count)) in self.chunks.iter().enumerate() {
            match runs.last() {
                Some(&(_, c)) if c == count => {}
                _ => runs.push((i as u32 + 1, count)),
            }
        }
        let mut p = Payload::new();
        p.u32(runs.len() as u32);
        for (first_chunk, samples_per_chunk) in runs {
            p.u32(first_chunk).u32(samples_per_chunk).u32(1);
        }
        full_box(*b"stsc", 0, 0, p.into_vec())
    }

    fn stsz(&self) -> Vec<u8> {
        let mut p = Payload::new();
        p.u32(0).u32(self.sizes.len() as u32);
        for &s in &self.sizes {
            p.u32(s);
        }
        full_box(*b"stsz", 0, 0, p.into_vec())
    }

    fn co64(&self) -> Vec<u8> {
        let mut p = Payload::new();
        p.u32(self.chunks.len() as u32);
        for &(offset, _) in &self.chunks {
            p.u64(offset);
        }
        full_box(*b"co64", 0, 0, p.into_vec())
    }
}
```

Export `TrackConfig` etc. from `lib.rs` (already added in Task 2).

- [ ] **Step 4: Run the full workspace** — every prior test (single-track roundtrip, capture e2e, ffprobe) plus the three new multitrack tests must pass.

Run: `~/.cargo/bin/cargo test --workspace && ~/.cargo/bin/cargo clippy --workspace --all-targets`

- [ ] **Step 5: Commit and push**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): multi-track HybridMp4Writer with Opus audio, ffprobe-validated

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
git push
```

---

## Out of scope (follow-ups)

- `AudioCapture` trait + mock audio source in `clipline-capture`, audio packets flowing through the replay ring (multi-track segments), and `save_replay` emitting video+audio — next plan.
- WASAPI loopback / per-process loopback Windows implementations (ddoc §10).
- Mic as a third track and track naming/`udta` labels for the editor.
