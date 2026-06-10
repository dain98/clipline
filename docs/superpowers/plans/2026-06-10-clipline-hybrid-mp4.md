# Clipline Hybrid MP4 Muxer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Hybrid MP4 writer from ddoc §10: fragmented MP4 while recording (crash-safe — every `moof`/`mdat` fragment is independently decodable), finalized on stop into a standard seekable MP4 by appending a full `moov` and overwriting the leading `free` placeholder with an `mdat` header that hides the fragments.

**Architecture:** New `clipline-mp4` crate, std-only. `boxes.rs` holds pure functions that build ISO-BMFF boxes as `Vec<u8>` (no seeking needed during construction). `walker.rs` is a minimal box parser used by tests for structural validation (and later by the editor/recovery path). `writer.rs` is `HybridMp4Writer<W: Write + Seek>`: writes `ftyp` + 16-byte `free` placeholder + fragmented-init `moov` (with `mvex`), appends `moof`+`mdat` fragments while bookkeeping per-sample metadata, and on `finalize()` appends a full `moov` (stts/stss/stsc/stsz/co64 built from the bookkeeping) then rewrites the `free` box as a 16-byte large-size `mdat` header spanning everything up to the final `moov`. Video-only (H.264/`avc1`) for now; audio tracks are an M1 follow-up (YAGNI).

**Tech Stack:** Rust std only (`std::io::{Write, Seek}`); tests use `std::io::Cursor`. Optional end-to-end validation against `ffprobe` (static build in `~/bin`), skipped gracefully when absent.

**Spec notes (ISO 14496-12 essentials used below):**
- A box = `u32 size` + 4-byte fourcc + payload; `size == 1` means a `u64` largesize follows the fourcc (16-byte header).
- A "full box" adds `u8 version` + 24-bit flags before its payload.
- Pure container boxes (`moov`, `trak`, `mdia`, `minf`, `stbl`, `dinf`, `mvex`, `moof`, `traf`): children start immediately at payload.
- Absent `stss` ⇒ all samples are sync samples; write it only when some sample is non-sync.
- `trun` sample_flags: sync sample `0x02000000`, non-sync `0x01010000`.
- `tfhd` flag `0x020000` = default-base-is-moof → `trun.data_offset` is relative to the `moof`'s first byte.

**Environment notes:** `cargo` lives at `~/.cargo/bin/cargo`. Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: Crate scaffold + box-builder primitives

**Files:**
- Modify: `Cargo.toml` (workspace members)
- Create: `crates/clipline-mp4/Cargo.toml`, `crates/clipline-mp4/src/lib.rs`, `crates/clipline-mp4/src/boxes.rs`
- Test: inline `#[cfg(test)]` in `boxes.rs`

- [ ] **Step 1: Add the crate to the workspace**

In root `Cargo.toml`, extend members:
```toml
members = [
    "crates/clipline-events",
    "crates/clipline-lol",
    "crates/clipline-buffer",
    "crates/clipline-mp4",
]
```

`crates/clipline-mp4/Cargo.toml`:
```toml
[package]
name = "clipline-mp4"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
```

`crates/clipline-mp4/src/lib.rs`:
```rust
pub mod boxes;
```

- [ ] **Step 2: Write the failing tests** (`boxes.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_box_is_size_fourcc_payload() {
        let b = mp4_box(*b"ftyp", vec![1, 2, 3, 4]);
        assert_eq!(b.len(), 12);
        assert_eq!(&b[0..4], &12u32.to_be_bytes());
        assert_eq!(&b[4..8], b"ftyp");
        assert_eq!(&b[8..], &[1, 2, 3, 4]);
    }

    #[test]
    fn full_box_prepends_version_and_flags() {
        let b = full_box(*b"tfdt", 1, 0x000002, vec![9]);
        assert_eq!(b.len(), 13);
        assert_eq!(b[8], 1); // version
        assert_eq!(&b[9..12], &[0x00, 0x00, 0x02]); // 24-bit flags
        assert_eq!(b[12], 9);
    }

    #[test]
    fn payload_builder_emits_big_endian() {
        let mut p = Payload::new();
        p.u8(1).u16(2).u32(3).u64(4).bytes(b"ab").i32(-1);
        assert_eq!(
            p.into_vec(),
            vec![
                1, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 4, b'a', b'b', 0xFF, 0xFF,
                0xFF, 0xFF
            ]
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: COMPILE ERROR (`mp4_box` not defined).

- [ ] **Step 4: Write the implementation** (top of `boxes.rs`)

```rust
/// Build a plain ISO-BMFF box: u32 size + fourcc + payload.
pub fn mp4_box(fourcc: [u8; 4], payload: Vec<u8>) -> Vec<u8> {
    let size = 8 + payload.len() as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(&payload);
    out
}

/// Build a "full box": version byte + 24-bit flags precede the payload.
pub fn full_box(fourcc: [u8; 4], version: u8, flags: u32, payload: Vec<u8>) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + payload.len());
    p.push(version);
    p.extend_from_slice(&flags.to_be_bytes()[1..4]);
    p.extend_from_slice(&payload);
    mp4_box(fourcc, p)
}

/// Chainable big-endian byte builder for box payloads.
#[derive(Default)]
pub struct Payload(Vec<u8>);

impl Payload {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}
```

Note: `Payload::into_vec` takes `self` by value while the chain methods return `&mut Self`; the test therefore builds with a `let mut p` then consumes it — matching the test code above.

- [ ] **Step 5: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): box-builder primitives for ISO-BMFF

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Box walker (structural parser)

Used by tests to validate output, and later by the editor/recovery path.

**Files:**
- Create: `crates/clipline-mp4/src/walker.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: inline `#[cfg(test)]` in `walker.rs`

- [ ] **Step 1: Write the failing tests**

Add to `lib.rs`:
```rust
pub mod walker;
```

`walker.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::boxes::mp4_box;

    #[test]
    fn walks_top_level_boxes() {
        let mut buf = mp4_box(*b"ftyp", vec![0; 4]);
        buf.extend(mp4_box(*b"free", vec![0; 8]));
        let boxes = walk(&buf);
        assert_eq!(boxes.len(), 2);
        assert_eq!(&boxes[0].fourcc, b"ftyp");
        assert_eq!(boxes[0].offset, 0);
        assert_eq!(boxes[0].size, 12);
        assert_eq!(&boxes[1].fourcc, b"free");
        assert_eq!(boxes[1].offset, 12);
        assert_eq!(boxes[1].payload_offset, 12 + 8);
    }

    #[test]
    fn handles_largesize_boxes() {
        // size=1 → u64 largesize follows fourcc (16-byte header).
        let payload = vec![7u8; 4];
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(b"mdat");
        buf.extend_from_slice(&(16u64 + 4).to_be_bytes());
        buf.extend_from_slice(&payload);
        let boxes = walk(&buf);
        assert_eq!(boxes.len(), 1);
        assert_eq!(&boxes[0].fourcc, b"mdat");
        assert_eq!(boxes[0].size, 20);
        assert_eq!(boxes[0].payload_offset, 16);
    }

    #[test]
    fn children_walks_container_payload() {
        let inner = mp4_box(*b"mvhd", vec![0; 4]);
        let outer = mp4_box(*b"moov", inner);
        let top = walk(&outer);
        let kids = children(&outer, &top[0]);
        assert_eq!(kids.len(), 1);
        assert_eq!(&kids[0].fourcc, b"mvhd");
        assert_eq!(kids[0].offset, 8); // absolute within buf
    }

    #[test]
    fn find_locates_by_fourcc() {
        let buf = mp4_box(*b"ftyp", vec![]);
        let boxes = walk(&buf);
        assert!(find(&boxes, b"ftyp").is_some());
        assert!(find(&boxes, b"moov").is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: COMPILE ERROR (`walk` not defined).

- [ ] **Step 3: Write the implementation** (top of `walker.rs`)

```rust
/// One parsed box header. Offsets are absolute within the parsed buffer.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub fourcc: [u8; 4],
    pub offset: u64,
    /// Total box size including header.
    pub size: u64,
    /// Absolute offset where the payload begins (8 or 16 past `offset`).
    pub payload_offset: u64,
}

/// Parse consecutive boxes starting at `buf[0]`. Stops at truncation.
pub fn walk(buf: &[u8]) -> Vec<BoxInfo> {
    walk_range(buf, 0, buf.len() as u64)
}

/// Parse the children of a pure container box (moov/trak/moof/…).
pub fn children(buf: &[u8], parent: &BoxInfo) -> Vec<BoxInfo> {
    walk_range(buf, parent.payload_offset, parent.offset + parent.size)
}

/// First box with the given fourcc, if any.
pub fn find<'a>(boxes: &'a [BoxInfo], fourcc: &[u8; 4]) -> Option<&'a BoxInfo> {
    boxes.iter().find(|b| &b.fourcc == fourcc)
}

fn walk_range(buf: &[u8], mut pos: u64, end: u64) -> Vec<BoxInfo> {
    let mut out = Vec::new();
    while pos + 8 <= end && (pos + 8) as usize <= buf.len() {
        let p = pos as usize;
        let size32 = u32::from_be_bytes(buf[p..p + 4].try_into().unwrap());
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&buf[p + 4..p + 8]);
        let (size, header) = if size32 == 1 {
            if (pos + 16) as usize > buf.len() {
                break;
            }
            let large = u64::from_be_bytes(buf[p + 8..p + 16].try_into().unwrap());
            (large, 16u64)
        } else if size32 == 0 {
            (end - pos, 8u64) // box extends to end
        } else {
            (size32 as u64, 8u64)
        };
        if size < header || pos + size > end {
            break; // truncated/corrupt — stop, return what we have
        }
        out.push(BoxInfo { fourcc, offset: pos, size, payload_offset: pos + header });
        pos += size;
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: 7 tests pass (3 boxes + 4 walker).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): ISO-BMFF box walker for validation and recovery

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: ftyp, free placeholder, and fragmented-init moov

**Files:**
- Create: `crates/clipline-mp4/src/init.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: inline `#[cfg(test)]` in `init.rs`

- [ ] **Step 1: Write the failing tests**

Add to `lib.rs`:
```rust
pub mod init;

pub use init::VideoTrackConfig;
```

`init.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::{children, find, walk};

    fn cfg() -> VideoTrackConfig {
        VideoTrackConfig {
            width: 1920,
            height: 1080,
            timescale: 90_000,
            sps: vec![0x67, 0x64, 0x00, 0x28, 0xAA],
            pps: vec![0x68, 0xEE, 0x3C, 0x80],
        }
    }

    #[test]
    fn init_section_is_ftyp_free_moov() {
        let mut buf = ftyp();
        buf.extend(free_placeholder());
        buf.extend(moov_init(&cfg()));
        let boxes = walk(&buf);
        let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
        assert_eq!(fourccs, vec![b"ftyp", b"free", b"moov"]);
        // The placeholder must be exactly 16 bytes so finalize() can
        // overwrite it with a largesize mdat header in place.
        assert_eq!(boxes[1].size, 16);
    }

    #[test]
    fn moov_contains_mvhd_trak_mvex() {
        let buf = moov_init(&cfg());
        let top = walk(&buf);
        let kids = children(&buf, &top[0]);
        assert!(find(&kids, b"mvhd").is_some());
        assert!(find(&kids, b"trak").is_some());
        assert!(find(&kids, b"mvex").is_some());
    }

    #[test]
    fn stsd_embeds_avcc_with_sps_pps() {
        let buf = moov_init(&cfg());
        // The avcC payload must contain the SPS and PPS byte strings.
        let needle_sps: &[u8] = &[0x67, 0x64, 0x00, 0x28, 0xAA];
        let needle_pps: &[u8] = &[0x68, 0xEE, 0x3C, 0x80];
        assert!(buf.windows(needle_sps.len()).any(|w| w == needle_sps));
        assert!(buf.windows(needle_pps.len()).any(|w| w == needle_pps));
        // And width/height as 16.16 fixed point inside tkhd.
        assert!(buf
            .windows(8)
            .any(|w| w == [0x07, 0x80, 0, 0, 0x04, 0x38, 0, 0]));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: COMPILE ERROR (`ftyp` not defined).

- [ ] **Step 3: Write the implementation** (top of `init.rs`)

```rust
use crate::boxes::{full_box, mp4_box, Payload};

/// Movie-header timescale (ticks per second) for mvhd/tkhd durations.
pub const MOVIE_TIMESCALE: u32 = 1000;
/// Identity transformation matrix for mvhd/tkhd.
const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// H.264 video track parameters. `sps`/`pps` are single raw NAL units
/// (no start codes / length prefixes).
#[derive(Debug, Clone)]
pub struct VideoTrackConfig {
    pub width: u16,
    pub height: u16,
    /// Media timescale (e.g. 90_000); sample durations use these ticks.
    pub timescale: u32,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

pub fn ftyp() -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(b"isom").u32(512).bytes(b"isom").bytes(b"iso6").bytes(b"mp41");
    mp4_box(*b"ftyp", p.into_vec())
}

/// 16-byte placeholder; finalize() overwrites it in place with a
/// largesize `mdat` header (ddoc §10, the OBS Hybrid MP4 trick).
pub fn free_placeholder() -> Vec<u8> {
    mp4_box(*b"free", vec![0; 8])
}

/// Fragmented-init `moov`: zero-duration sample tables plus `mvex` so
/// readers know sample data lives in movie fragments.
pub fn moov_init(cfg: &VideoTrackConfig) -> Vec<u8> {
    let mut moov = mvhd(0);
    moov.extend(trak(cfg, 0, 0));
    moov.extend(mvex());
    mp4_box(*b"moov", moov)
}

pub fn mvhd(duration_movie_ts: u64) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(0) // creation_time
        .u32(0) // modification_time
        .u32(MOVIE_TIMESCALE)
        .u32(duration_movie_ts as u32)
        .u32(0x0001_0000) // rate 1.0
        .u16(0x0100) // volume 1.0
        .u16(0) // reserved
        .u32(0)
        .u32(0); // reserved
    for m in MATRIX {
        p.u32(m);
    }
    for _ in 0..6 {
        p.u32(0); // pre_defined
    }
    p.u32(2); // next_track_ID
    full_box(*b"mvhd", 0, 0, p.into_vec())
}

/// The whole `trak`. Durations are 0 for the fragmented init moov and real
/// values for the final moov; `stbl_tail` swaps empty vs. populated tables.
pub fn trak(cfg: &VideoTrackConfig, duration_movie_ts: u64, duration_media_ts: u64) -> Vec<u8> {
    let mut t = tkhd(cfg, duration_movie_ts);
    t.extend(mdia(cfg, duration_media_ts, empty_stbl_tail()));
    mp4_box(*b"trak", t)
}

/// Same as `trak` but with caller-provided populated sample tables
/// (stts/stss/stsc/stsz/co64) — used by finalize.
pub fn trak_with_tables(
    cfg: &VideoTrackConfig,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    let mut t = tkhd(cfg, duration_movie_ts);
    t.extend(mdia(cfg, duration_media_ts, stbl_tail));
    mp4_box(*b"trak", t)
}

fn tkhd(cfg: &VideoTrackConfig, duration_movie_ts: u64) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(0).u32(0) // creation/modification
        .u32(1) // track_ID
        .u32(0) // reserved
        .u32(duration_movie_ts as u32)
        .u32(0)
        .u32(0) // reserved
        .u16(0) // layer
        .u16(0) // alternate_group
        .u16(0) // volume (video)
        .u16(0); // reserved
    for m in MATRIX {
        p.u32(m);
    }
    p.u32((cfg.width as u32) << 16).u32((cfg.height as u32) << 16);
    full_box(*b"tkhd", 0, 0x000003, p.into_vec()) // enabled | in_movie
}

fn mdia(cfg: &VideoTrackConfig, duration_media_ts: u64, stbl_tail: Vec<u8>) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(0).u32(0).u32(cfg.timescale).u32(duration_media_ts as u32)
        .u16(0x55C4) // language: und
        .u16(0);
    let mdhd = full_box(*b"mdhd", 0, 0, p.into_vec());

    let mut h = Payload::new();
    h.u32(0).bytes(b"vide").u32(0).u32(0).u32(0).bytes(b"Clipline Video\0");
    let hdlr = full_box(*b"hdlr", 0, 0, h.into_vec());

    let mut m = mdhd;
    m.extend(hdlr);
    m.extend(minf(cfg, stbl_tail));
    mp4_box(*b"mdia", m)
}

fn minf(cfg: &VideoTrackConfig, stbl_tail: Vec<u8>) -> Vec<u8> {
    let mut v = Payload::new();
    v.u16(0).u16(0).u16(0).u16(0); // graphicsmode + opcolor
    let vmhd = full_box(*b"vmhd", 0, 1, v.into_vec());

    let url = full_box(*b"url ", 0, 1, vec![]); // self-contained
    let mut d = Payload::new();
    d.u32(1); // entry_count
    let mut dref_payload = d.into_vec();
    dref_payload.extend(url);
    let dref = full_box(*b"dref", 0, 0, dref_payload);
    let dinf = mp4_box(*b"dinf", dref);

    let mut stbl = stsd(cfg);
    stbl.extend(stbl_tail);

    let mut m = vmhd;
    m.extend(dinf);
    m.extend(mp4_box(*b"stbl", stbl));
    mp4_box(*b"minf", m)
}

fn stsd(cfg: &VideoTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(1); // entry_count
    let mut payload = p.into_vec();
    payload.extend(avc1(cfg));
    full_box(*b"stsd", 0, 0, payload)
}

fn avc1(cfg: &VideoTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(&[0; 6]) // reserved
        .u16(1) // data_reference_index
        .u16(0).u16(0) // pre_defined/reserved
        .u32(0).u32(0).u32(0) // pre_defined
        .u16(cfg.width)
        .u16(cfg.height)
        .u32(0x0048_0000) // horizresolution 72dpi
        .u32(0x0048_0000) // vertresolution
        .u32(0) // reserved
        .u16(1) // frame_count
        .bytes(&[0; 32]) // compressorname
        .u16(0x0018) // depth
        .u16(0xFFFF); // pre_defined = -1
    let mut payload = p.into_vec();
    payload.extend(avcc(cfg));
    mp4_box(*b"avc1", payload)
}

fn avcc(cfg: &VideoTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.u8(1) // configurationVersion
        .u8(cfg.sps.get(1).copied().unwrap_or(0)) // AVCProfileIndication
        .u8(cfg.sps.get(2).copied().unwrap_or(0)) // profile_compatibility
        .u8(cfg.sps.get(3).copied().unwrap_or(0)) // AVCLevelIndication
        .u8(0xFF) // lengthSizeMinusOne = 3
        .u8(0xE1) // 1 SPS
        .u16(cfg.sps.len() as u16)
        .bytes(&cfg.sps)
        .u8(1) // 1 PPS
        .u16(cfg.pps.len() as u16)
        .bytes(&cfg.pps);
    mp4_box(*b"avcC", p.into_vec())
}

/// Empty stts/stsc/stsz/stco for the fragmented init moov.
fn empty_stbl_tail() -> Vec<u8> {
    let mut out = full_box(*b"stts", 0, 0, 0u32.to_be_bytes().to_vec());
    let mut stsc = Payload::new();
    stsc.u32(0);
    out.extend(full_box(*b"stsc", 0, 0, stsc.into_vec()));
    let mut stsz = Payload::new();
    stsz.u32(0).u32(0); // sample_size=0, sample_count=0
    out.extend(full_box(*b"stsz", 0, 0, stsz.into_vec()));
    let mut stco = Payload::new();
    stco.u32(0);
    out.extend(full_box(*b"stco", 0, 0, stco.into_vec()));
    out
}

fn mvex() -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(1) // track_ID
        .u32(1) // default_sample_description_index
        .u32(0) // default_sample_duration
        .u32(0) // default_sample_size
        .u32(0); // default_sample_flags
    let trex = full_box(*b"trex", 0, 0, p.into_vec());
    mp4_box(*b"mvex", trex)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: 10 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): ftyp, free placeholder, fragmented-init moov with avcC

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Fragment writing (moof + mdat)

**Files:**
- Create: `crates/clipline-mp4/src/fragment.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: inline `#[cfg(test)]` in `fragment.rs`

- [ ] **Step 1: Write the failing tests**

Add to `lib.rs`:
```rust
pub mod fragment;

pub use fragment::FragSample;
```

`fragment.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::{children, find, walk};

    fn samples() -> Vec<FragSample> {
        vec![
            FragSample { data: b"KEYFRAME".to_vec(), duration: 1500, is_sync: true },
            FragSample { data: b"delta1".to_vec(), duration: 1500, is_sync: false },
        ]
    }

    #[test]
    fn fragment_is_moof_then_mdat_with_sample_bytes() {
        let buf = fragment(1, 0, &samples());
        let boxes = walk(&buf);
        assert_eq!(&boxes[0].fourcc, b"moof");
        assert_eq!(&boxes[1].fourcc, b"mdat");
        let mdat_payload =
            &buf[boxes[1].payload_offset as usize..(boxes[1].offset + boxes[1].size) as usize];
        assert_eq!(mdat_payload, b"KEYFRAMEdelta1");
    }

    #[test]
    fn trun_data_offset_points_at_first_sample_byte() {
        let buf = fragment(1, 0, &samples());
        let boxes = walk(&buf);
        let moof = &boxes[0];
        let kids = children(&buf, moof);
        let traf = find(&kids, b"traf").unwrap();
        let traf_kids = children(&buf, traf);
        let trun = find(&traf_kids, b"trun").unwrap();
        // trun payload: version/flags(4) sample_count(4) data_offset(4)…
        let p = trun.payload_offset as usize;
        let data_offset =
            i32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()) as u64;
        // default-base-is-moof: offset is relative to moof start (= 0 here).
        assert_eq!(&buf[data_offset as usize..data_offset as usize + 8], b"KEYFRAME");
    }

    #[test]
    fn tfdt_carries_base_decode_time() {
        let buf = fragment(7, 123_456, &samples());
        let boxes = walk(&buf);
        let kids = children(&buf, &boxes[0]);
        let traf = find(&kids, b"traf").unwrap();
        let traf_kids = children(&buf, traf);
        let tfdt = find(&traf_kids, b"tfdt").unwrap();
        let p = tfdt.payload_offset as usize;
        assert_eq!(buf[p], 1, "tfdt version 1 (64-bit)");
        let bdt = u64::from_be_bytes(buf[p + 4..p + 12].try_into().unwrap());
        assert_eq!(bdt, 123_456);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: COMPILE ERROR (`FragSample` not defined).

- [ ] **Step 3: Write the implementation** (top of `fragment.rs`)

```rust
use crate::boxes::{full_box, mp4_box, Payload};

/// trun sample_flags for a sync sample (I-frame).
const FLAG_SYNC: u32 = 0x0200_0000;
/// trun sample_flags for a non-sync sample (depends on others).
const FLAG_NON_SYNC: u32 = 0x0101_0000;

/// One encoded sample handed to the muxer.
#[derive(Debug, Clone)]
pub struct FragSample {
    /// Encoded bytes in MP4 stream format (length-prefixed NALs for AVC).
    pub data: Vec<u8>,
    /// Duration in media-timescale ticks.
    pub duration: u32,
    pub is_sync: bool,
}

/// Build one complete `moof` + `mdat` fragment.
pub fn fragment(sequence: u32, base_decode_time: u64, samples: &[FragSample]) -> Vec<u8> {
    // The trun data_offset (moof start → first mdat payload byte) depends on
    // the moof's own size, so build the moof once with a placeholder, then
    // patch — its size doesn't change.
    let mut moof = build_moof(sequence, base_decode_time, samples, 0);
    let data_offset = (moof.len() + 8) as i32; // + mdat header
    moof = build_moof(sequence, base_decode_time, samples, data_offset);

    let mut mdat_payload = Vec::new();
    for s in samples {
        mdat_payload.extend_from_slice(&s.data);
    }
    let mut out = moof;
    out.extend(mp4_box(*b"mdat", mdat_payload));
    out
}

fn build_moof(
    sequence: u32,
    base_decode_time: u64,
    samples: &[FragSample],
    data_offset: i32,
) -> Vec<u8> {
    let mut mfhd_p = Payload::new();
    mfhd_p.u32(sequence);
    let mfhd = full_box(*b"mfhd", 0, 0, mfhd_p.into_vec());

    let mut tfhd_p = Payload::new();
    tfhd_p.u32(1); // track_ID
    let tfhd = full_box(*b"tfhd", 0, 0x020000, tfhd_p.into_vec()); // default-base-is-moof

    let mut tfdt_p = Payload::new();
    tfdt_p.u64(base_decode_time);
    let tfdt = full_box(*b"tfdt", 1, 0, tfdt_p.into_vec());

    // flags: data-offset(0x1) | sample-duration(0x100) | sample-size(0x200)
    //        | sample-flags(0x400)
    let mut trun_p = Payload::new();
    trun_p.u32(samples.len() as u32).i32(data_offset);
    for s in samples {
        trun_p.u32(s.duration).u32(s.data.len() as u32).u32(if s.is_sync {
            FLAG_SYNC
        } else {
            FLAG_NON_SYNC
        });
    }
    let trun = full_box(*b"trun", 0, 0x000701, trun_p.into_vec());

    let mut traf = tfhd;
    traf.extend(tfdt);
    traf.extend(trun);
    let traf = mp4_box(*b"traf", traf);

    let mut moof = mfhd;
    moof.extend(traf);
    mp4_box(*b"moof", moof)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: 13 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): moof/mdat fragment builder with patched trun offsets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: HybridMp4Writer with finalize

The writer ties it together: init section, streaming fragments with bookkeeping, then finalize — append full `moov`, overwrite `free` with a largesize `mdat` header (ddoc §10).

**Files:**
- Create: `crates/clipline-mp4/src/writer.rs`
- Modify: `crates/clipline-mp4/src/lib.rs`
- Test: `crates/clipline-mp4/tests/hybrid_roundtrip.rs`

- [ ] **Step 1: Write the failing test**

Add to `lib.rs`:
```rust
pub mod writer;

pub use writer::HybridMp4Writer;
```

`tests/hybrid_roundtrip.rs`:
```rust
use std::io::Cursor;

use clipline_mp4::walker::{children, find, walk};
use clipline_mp4::{FragSample, HybridMp4Writer, VideoTrackConfig};

fn cfg() -> VideoTrackConfig {
    VideoTrackConfig {
        width: 64,
        height: 64,
        timescale: 90_000,
        sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
        pps: vec![0x68, 0xEE, 0x38, 0x80],
    }
}

fn gop(start: u32) -> Vec<FragSample> {
    (0..3)
        .map(|i| FragSample {
            data: format!("sample-{:04}", start + i).into_bytes(),
            duration: 3000, // 30 fps @ 90kHz
            is_sync: i == 0,
        })
        .collect()
}

#[test]
fn while_recording_file_is_fragmented_and_walkable() {
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg()).unwrap();
    w.write_fragment(&gop(0)).unwrap();
    w.write_fragment(&gop(3)).unwrap();
    // Simulate a crash: inspect the bytes WITHOUT finalize.
    let buf = w.into_inner().into_inner();
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(
        fourccs,
        vec![b"ftyp", b"free", b"moov", b"moof", b"mdat", b"moof", b"mdat"],
        "fragmented layout must survive a crash mid-recording"
    );
}

#[test]
fn finalized_file_reads_as_standard_mp4() {
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg()).unwrap();
    w.write_fragment(&gop(0)).unwrap();
    w.write_fragment(&gop(3)).unwrap();
    let buf = w.finalize().unwrap().into_inner();

    // Standard layout: the free box became a giant mdat hiding the
    // fragments; a full moov sits at the end.
    let boxes = walk(&buf);
    let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
    assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);

    // The final moov has populated tables.
    let moov = find(&boxes, b"moov").unwrap();
    let buf_ref = &buf;
    let moov_kids = children(buf_ref, moov);
    assert!(find(&moov_kids, b"mvex").is_none(), "final moov is not fragmented");

    // stsz lists 6 samples; co64 chunk offsets point at real sample bytes.
    let stsz = find_deep(buf_ref, moov, b"stsz").expect("stsz");
    let p = stsz.payload_offset as usize;
    let sample_count = u32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap());
    assert_eq!(sample_count, 6);

    let co64 = find_deep(buf_ref, moov, b"co64").expect("co64");
    let p = co64.payload_offset as usize;
    let entry_count = u32::from_be_bytes(buf[p + 4..p + 8].try_into().unwrap());
    assert_eq!(entry_count, 2, "one chunk per fragment");
    let first_chunk =
        u64::from_be_bytes(buf[p + 8..p + 16].try_into().unwrap()) as usize;
    assert_eq!(&buf[first_chunk..first_chunk + 11], b"sample-0000");

    // stss marks samples 1 and 4 as sync.
    let stss = find_deep(buf_ref, moov, b"stss").expect("stss");
    let p = stss.payload_offset as usize;
    let n = u32::from_be_bytes(buf[p + 4..p + 8].try_into().unwrap());
    assert_eq!(n, 2);
    assert_eq!(u32::from_be_bytes(buf[p + 8..p + 12].try_into().unwrap()), 1);
    assert_eq!(u32::from_be_bytes(buf[p + 12..p + 16].try_into().unwrap()), 4);
}

/// Depth-first search for a fourcc under a container box.
fn find_deep<'a>(
    buf: &'a [u8],
    parent: &clipline_mp4::walker::BoxInfo,
    fourcc: &[u8; 4],
) -> Option<clipline_mp4::walker::BoxInfo> {
    const CONTAINERS: [&[u8; 4]; 6] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts"];
    for child in children(buf, parent) {
        if &child.fourcc == fourcc {
            return Some(child);
        }
        if CONTAINERS.contains(&&child.fourcc) {
            if let Some(hit) = find_deep(buf, &child, fourcc) {
                return Some(hit);
            }
        }
    }
    None
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p clipline-mp4 --test hybrid_roundtrip`
Expected: COMPILE ERROR (`HybridMp4Writer` not defined).

- [ ] **Step 3: Write the implementation** (`writer.rs`)

```rust
use std::io::{self, Seek, SeekFrom, Write};

use crate::boxes::{full_box, mp4_box, Payload};
use crate::fragment::{fragment, FragSample};
use crate::init::{ftyp, free_placeholder, moov_init, mvhd, trak_with_tables, VideoTrackConfig, MOVIE_TIMESCALE};

/// Streaming Hybrid MP4 writer (ddoc §10). While recording the file is a
/// fragmented MP4 (crash-safe); `finalize()` turns it into a standard
/// seekable MP4 in place.
pub struct HybridMp4Writer<W: Write + Seek> {
    w: W,
    cfg: VideoTrackConfig,
    free_offset: u64,
    next_sequence: u32,
    next_decode_time: u64,
    /// Per-sample bookkeeping for the final moov.
    sizes: Vec<u32>,
    durations: Vec<u32>,
    sync: Vec<bool>,
    /// (absolute offset of first sample byte, sample count) per fragment.
    chunks: Vec<(u64, u32)>,
}

impl<W: Write + Seek> HybridMp4Writer<W> {
    pub fn new(mut w: W, cfg: VideoTrackConfig) -> io::Result<Self> {
        let ftyp = ftyp();
        w.write_all(&ftyp)?;
        let free_offset = ftyp.len() as u64;
        w.write_all(&free_placeholder())?;
        w.write_all(&moov_init(&cfg))?;
        Ok(Self {
            w,
            cfg,
            free_offset,
            next_sequence: 1,
            next_decode_time: 0,
            sizes: Vec::new(),
            durations: Vec::new(),
            sync: Vec::new(),
            chunks: Vec::new(),
        })
    }

    pub fn write_fragment(&mut self, samples: &[FragSample]) -> io::Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let frag = fragment(self.next_sequence, self.next_decode_time, samples);
        let frag_start = self.w.stream_position()?;

        // First sample byte = fragment start + moof size + mdat header (8).
        // The moof is everything before the trailing mdat box.
        let mdat_payload_len: usize = samples.iter().map(|s| s.data.len()).sum();
        let moof_len = frag.len() - (8 + mdat_payload_len);
        let first_sample = frag_start + moof_len as u64 + 8;

        self.w.write_all(&frag)?;

        self.chunks.push((first_sample, samples.len() as u32));
        for s in samples {
            self.sizes.push(s.data.len() as u32);
            self.durations.push(s.duration);
            self.sync.push(s.is_sync);
            self.next_decode_time += s.duration as u64;
        }
        self.next_sequence += 1;
        Ok(())
    }

    /// Append the full moov, then overwrite the leading free box with a
    /// largesize mdat header spanning init-moov + all fragments — hiding
    /// them so the file parses as ftyp / mdat / moov (ddoc §10).
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

    /// Abort without finalizing (crash-simulation / tests): hand back the
    /// underlying writer with the fragmented layout intact.
    pub fn into_inner(self) -> W {
        self.w
    }

    fn final_moov(&self) -> Vec<u8> {
        let duration_media: u64 = self.durations.iter().map(|&d| d as u64).sum();
        let duration_movie =
            duration_media * MOVIE_TIMESCALE as u64 / self.cfg.timescale as u64;

        let mut tail = self.stts();
        if let Some(stss) = self.stss() {
            tail.extend(stss);
        }
        tail.extend(self.stsc());
        tail.extend(self.stsz());
        tail.extend(self.co64());

        let mut moov = mvhd(duration_movie);
        moov.extend(trak_with_tables(&self.cfg, duration_movie, duration_media, tail));
        mp4_box(*b"moov", moov)
    }

    fn stts(&self) -> Vec<u8> {
        // Run-length encode consecutive equal durations.
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
            .map(|(i, _)| i as u32 + 1) // 1-based sample numbers
            .collect();
        let mut p = Payload::new();
        p.u32(syncs.len() as u32);
        for s in syncs {
            p.u32(s);
        }
        Some(full_box(*b"stss", 0, 0, p.into_vec()))
    }

    fn stsc(&self) -> Vec<u8> {
        // One chunk per fragment; run-length over samples_per_chunk.
        let mut runs: Vec<(u32, u32)> = Vec::new(); // (first_chunk, samples_per_chunk)
        for (i, &(_, count)) in self.chunks.iter().enumerate() {
            match runs.last() {
                Some(&(_, c)) if c == count => {}
                _ => runs.push((i as u32 + 1, count)),
            }
        }
        let mut p = Payload::new();
        p.u32(runs.len() as u32);
        for (first_chunk, samples_per_chunk) in runs {
            p.u32(first_chunk).u32(samples_per_chunk).u32(1); // sample_description_index
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

- [ ] **Step 4: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test -p clipline-mp4`
Expected: all tests pass (13 unit + 2 integration).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(mp4): HybridMp4Writer — fragmented while recording, standard on finalize

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: ffprobe end-to-end validation

Validate against a real demuxer. The test skips (with a printed notice) when `ffprobe` is unavailable, so the suite stays green on minimal machines/CI.

**Files:**
- Test: `crates/clipline-mp4/tests/ffprobe_validation.rs`

- [ ] **Step 1: Write the test**

```rust
use std::io::Cursor;
use std::process::Command;

use clipline_mp4::{FragSample, HybridMp4Writer, VideoTrackConfig};

fn ffprobe_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let local = std::path::Path::new(&home).join("bin/ffprobe");
    if local.exists() {
        return Some(local);
    }
    which("ffprobe")
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| std::path::Path::new(d).join(bin))
        .find(|p| p.exists())
}

#[test]
fn ffprobe_parses_finalized_file_as_standard_mp4() {
    let Some(ffprobe) = ffprobe_path() else {
        eprintln!("SKIP: ffprobe not found; container validated by walker tests only");
        return;
    };

    let cfg = VideoTrackConfig {
        width: 128,
        height: 128,
        timescale: 90_000,
        sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC, 0x72, 0x84, 0x44, 0x26, 0x84],
        pps: vec![0x68, 0xEE, 0x38, 0x80],
    };
    let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), cfg).unwrap();
    // 60 frames at 30 fps in GOPs of 10 → 2.0 s duration.
    for g in 0..6 {
        let samples: Vec<FragSample> = (0..10)
            .map(|i| FragSample {
                data: vec![0xAB; 100 + i],
                duration: 3000,
                is_sync: i == 0,
            })
            .collect();
        w.write_fragment(&samples).unwrap();
        let _ = g;
    }
    let buf = w.finalize().unwrap().into_inner();

    let path = std::env::temp_dir().join("clipline_hybrid_test.mp4");
    std::fs::write(&path, &buf).unwrap();

    let out = Command::new(&ffprobe)
        .args(["-v", "error", "-show_entries", "stream=codec_name,nb_frames", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1"])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "ffprobe failed: {stderr}");
    assert!(stdout.contains("codec_name=h264"), "got: {stdout}");
    assert!(stdout.contains("nb_frames=60"), "got: {stdout}");
    // duration=2.000000 (±ffprobe formatting)
    assert!(stdout.contains("duration=2.0"), "got: {stdout}");

    std::fs::remove_file(&path).ok();
}
```

- [ ] **Step 2: Run the test**

Run: `~/.cargo/bin/cargo test -p clipline-mp4 --test ffprobe_validation -- --nocapture`
Expected: PASS with real ffprobe output assertions (or SKIP notice if ffprobe is missing).

- [ ] **Step 3: Full workspace verification**

Run: `~/.cargo/bin/cargo test --workspace && ~/.cargo/bin/cargo clippy --workspace --all-targets`
Expected: all green, no clippy warnings.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
test(mp4): validate finalized hybrid MP4 against ffprobe

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

## Out of scope (follow-ups)

- Audio track muxing (multi-track: game/mic/system — ddoc §10) — extends `moov`/`moof` with a second `trak`/`traf`.
- Crash-recovery reader (parse a non-finalized fragmented file and finalize it offline) — `walker.rs` is the foundation.
- Keyframe-aligned stream-copy trim for the editor (ddoc §11) — consumes the same box builders.
