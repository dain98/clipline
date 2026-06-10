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

/// The whole `trak` with empty sample tables (fragmented init moov).
pub fn trak(cfg: &VideoTrackConfig, duration_movie_ts: u64, duration_media_ts: u64) -> Vec<u8> {
    trak_with_tables(cfg, duration_movie_ts, duration_media_ts, empty_stbl_tail())
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
        .u16(0)
        .u16(0) // pre_defined/reserved
        .u32(0)
        .u32(0)
        .u32(0) // pre_defined
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
