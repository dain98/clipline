use crate::boxes::{full_box, mp4_box, Payload};

/// Movie-header timescale. 720 kHz is the least common multiple of Clipline's
/// 90 kHz video and 48 kHz Opus clocks, so edit-list gaps stay exact integers.
pub const MOVIE_TIMESCALE: u32 = 720_000;
/// Identity transformation matrix for mvhd/tkhd.
const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EditListEntry {
    pub duration_movie_ts: u64,
    /// Track-media time, or -1 for an empty edit (silence/blank presentation).
    pub media_time: i64,
}

/// Codec-specific decoder configuration for the video sample entry
/// (ddoc §4 encoder matrix: AV1 / HEVC / H.264). Parameter sets are raw
/// NAL units (no start codes / length prefixes); the AV1 sequence header
/// is a full OBU (header + size field + payload).
#[derive(Debug, Clone)]
pub enum VideoCodecParams {
    H264 {
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    },
    Hevc {
        vps: Vec<Vec<u8>>,
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    },
    Av1 {
        sequence_header_obu: Vec<u8>,
    },
}

/// Video track parameters.
#[derive(Debug, Clone)]
pub struct VideoTrackConfig {
    pub width: u16,
    pub height: u16,
    /// Media timescale (e.g. 90_000); sample durations use these ticks.
    pub timescale: u32,
    pub codec: VideoCodecParams,
}

impl VideoTrackConfig {
    pub fn h264(width: u16, height: u16, timescale: u32, sps: Vec<u8>, pps: Vec<u8>) -> Self {
        Self::h264_with_parameter_sets(width, height, timescale, vec![sps], vec![pps])
    }

    pub fn h264_with_parameter_sets(
        width: u16,
        height: u16,
        timescale: u32,
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            width,
            height,
            timescale,
            codec: VideoCodecParams::H264 { sps, pps },
        }
    }

    pub fn hevc(
        width: u16,
        height: u16,
        timescale: u32,
        vps: Vec<u8>,
        sps: Vec<u8>,
        pps: Vec<u8>,
    ) -> Self {
        Self::hevc_with_parameter_sets(width, height, timescale, vec![vps], vec![sps], vec![pps])
    }

    pub fn hevc_with_parameter_sets(
        width: u16,
        height: u16,
        timescale: u32,
        vps: Vec<Vec<u8>>,
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            width,
            height,
            timescale,
            codec: VideoCodecParams::Hevc { vps, sps, pps },
        }
    }

    pub fn av1(width: u16, height: u16, timescale: u32, sequence_header_obu: Vec<u8>) -> Self {
        Self {
            width,
            height,
            timescale,
            codec: VideoCodecParams::Av1 {
                sequence_header_obu,
            },
        }
    }
}

/// Opus audio track parameters (ddoc §4/§10). Track timescale = sample rate.
#[derive(Debug, Clone)]
pub struct AudioTrackConfig {
    pub channels: u16,
    pub sample_rate: u32,
    /// Opus pre-skip in 48 kHz samples (dOps PreSkip).
    pub pre_skip: u16,
}

pub fn ftyp() -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(b"isom")
        .u32(512)
        .bytes(b"isom")
        .bytes(b"iso6")
        .bytes(b"mp41");
    mp4_box(*b"ftyp", p.into_vec())
}

/// 16-byte placeholder; finalize() overwrites it in place with a
/// largesize `mdat` header (ddoc §10, the OBS Hybrid MP4 trick).
pub fn free_placeholder() -> Vec<u8> {
    mp4_box(*b"free", vec![0; 8])
}

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

/// Fragmented-init `moov`: zero-duration sample tables plus `mvex` so
/// readers know sample data lives in movie fragments.
pub fn moov_init(cfg: &VideoTrackConfig) -> Vec<u8> {
    moov_init_multi(&[TrackConfig::Video(cfg.clone())])
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
        p.u32(id) // track_ID
            .u32(1) // default_sample_description_index
            .u32(0) // default_sample_duration
            .u32(0) // default_sample_size
            .u32(0); // default_sample_flags
        payload.extend(full_box(*b"trex", 0, 0, p.into_vec()));
    }
    mp4_box(*b"mvex", payload)
}

pub fn mvhd(duration_movie_ts: u64, next_track_id: u32) -> Vec<u8> {
    let version = u8::from(duration_movie_ts > u32::MAX as u64);
    let mut p = Payload::new();
    if version == 1 {
        p.u64(0) // creation_time
            .u64(0) // modification_time
            .u32(MOVIE_TIMESCALE)
            .u64(duration_movie_ts);
    } else {
        p.u32(0) // creation_time
            .u32(0) // modification_time
            .u32(MOVIE_TIMESCALE)
            .u32(duration_movie_ts as u32);
    }
    p.u32(0x0001_0000) // rate 1.0
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
    p.u32(next_track_id);
    full_box(*b"mvhd", version, 0, p.into_vec())
}

pub fn video_trak_with_tables(
    cfg: &VideoTrackConfig,
    track_id: u32,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    video_trak_with_tables_and_edits(
        cfg,
        track_id,
        duration_movie_ts,
        duration_media_ts,
        stbl_tail,
        &[],
    )
}

pub(crate) fn video_trak_with_tables_and_edits(
    cfg: &VideoTrackConfig,
    track_id: u32,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
    edits: &[EditListEntry],
) -> Vec<u8> {
    let mut v = Payload::new();
    v.u16(0).u16(0).u16(0).u16(0); // graphicsmode + opcolor
    let vmhd = full_box(*b"vmhd", 0, 1, v.into_vec());

    let mut t = tkhd(track_id, duration_movie_ts, 0, cfg.width, cfg.height);
    if !edits.is_empty() {
        t.extend(edts(edits));
    }
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
    audio_trak_with_tables_and_edits(
        cfg,
        track_id,
        duration_movie_ts,
        duration_media_ts,
        stbl_tail,
        &[],
    )
}

pub(crate) fn audio_trak_with_tables_and_edits(
    cfg: &AudioTrackConfig,
    track_id: u32,
    duration_movie_ts: u64,
    duration_media_ts: u64,
    stbl_tail: Vec<u8>,
    edits: &[EditListEntry],
) -> Vec<u8> {
    let mut s = Payload::new();
    s.u16(0).u16(0); // balance + reserved
    let smhd = full_box(*b"smhd", 0, 0, s.into_vec());

    let mut t = tkhd(track_id, duration_movie_ts, 0x0100, 0, 0);
    if !edits.is_empty() {
        t.extend(edts(edits));
    }
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

fn edts(entries: &[EditListEntry]) -> Vec<u8> {
    let version = u8::from(entries.iter().any(|entry| {
        entry.duration_movie_ts > u32::MAX as u64
            || entry.media_time < i32::MIN as i64
            || entry.media_time > i32::MAX as i64
    }));
    let mut p = Payload::new();
    p.u32(entries.len() as u32);
    for entry in entries {
        if version == 1 {
            p.u64(entry.duration_movie_ts).u64(entry.media_time as u64);
        } else {
            p.u32(entry.duration_movie_ts as u32)
                .u32(entry.media_time as i32 as u32);
        }
        p.u16(1).u16(0); // media_rate = 1.0
    }
    mp4_box(*b"edts", full_box(*b"elst", version, 0, p.into_vec()))
}

fn tkhd(track_id: u32, duration_movie_ts: u64, volume: u16, width: u16, height: u16) -> Vec<u8> {
    let version = u8::from(duration_movie_ts > u32::MAX as u64);
    let mut p = Payload::new();
    if version == 1 {
        p.u64(0) // creation_time
            .u64(0) // modification_time
            .u32(track_id)
            .u32(0) // reserved
            .u64(duration_movie_ts);
    } else {
        p.u32(0) // creation_time
            .u32(0) // modification_time
            .u32(track_id)
            .u32(0) // reserved
            .u32(duration_movie_ts as u32);
    }
    p.u32(0)
        .u32(0) // reserved
        .u16(0) // layer
        .u16(0) // alternate_group
        .u16(volume)
        .u16(0); // reserved
    for m in MATRIX {
        p.u32(m);
    }
    p.u32((width as u32) << 16).u32((height as u32) << 16);
    full_box(*b"tkhd", version, 0x000003, p.into_vec()) // enabled | in_movie
}

fn mdhd(timescale: u32, duration_media_ts: u64) -> Vec<u8> {
    let version = u8::from(duration_media_ts > u32::MAX as u64);
    let mut p = Payload::new();
    if version == 1 {
        p.u64(0) // creation_time
            .u64(0) // modification_time
            .u32(timescale)
            .u64(duration_media_ts);
    } else {
        p.u32(0) // creation_time
            .u32(0) // modification_time
            .u32(timescale)
            .u32(duration_media_ts as u32);
    }
    p.u16(0x55C4) // language: und
        .u16(0);
    full_box(*b"mdhd", version, 0, p.into_vec())
}

fn mdia_generic(
    timescale: u32,
    duration_media_ts: u64,
    handler: [u8; 4],
    handler_name: &[u8],
    media_header_box: Vec<u8>,
    stsd_box: Vec<u8>,
    stbl_tail: Vec<u8>,
) -> Vec<u8> {
    let mdhd = mdhd(timescale, duration_media_ts);

    let mut h = Payload::new();
    h.u32(0)
        .bytes(&handler)
        .u32(0)
        .u32(0)
        .u32(0)
        .bytes(handler_name);
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

fn stsd(cfg: &VideoTrackConfig) -> Vec<u8> {
    let mut p = Payload::new();
    p.u32(1); // entry_count
    let mut payload = p.into_vec();
    let (fourcc, codec_box) = match &cfg.codec {
        VideoCodecParams::H264 { sps, pps } => (*b"avc1", avcc(sps, pps)),
        VideoCodecParams::Hevc { vps, sps, pps } => (*b"hvc1", crate::hvcc::hvcc(vps, sps, pps)),
        VideoCodecParams::Av1 {
            sequence_header_obu,
        } => (*b"av01", crate::av1c::av1c(sequence_header_obu)),
    };
    payload.extend(visual_sample_entry(
        fourcc, cfg.width, cfg.height, codec_box,
    ));
    full_box(*b"stsd", 0, 0, payload)
}

/// The VisualSampleEntry shell shared by avc1/hvc1/av01 — only the fourcc
/// and the trailing codec configuration box differ.
fn visual_sample_entry(fourcc: [u8; 4], width: u16, height: u16, codec_box: Vec<u8>) -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(&[0; 6]) // reserved
        .u16(1) // data_reference_index
        .u16(0)
        .u16(0) // pre_defined/reserved
        .u32(0)
        .u32(0)
        .u32(0) // pre_defined
        .u16(width)
        .u16(height)
        .u32(0x0048_0000) // horizresolution 72dpi
        .u32(0x0048_0000) // vertresolution
        .u32(0) // reserved
        .u16(1) // frame_count
        .bytes(&[0; 32]) // compressorname
        .u16(0x0018) // depth
        .u16(0xFFFF); // pre_defined = -1
    let mut payload = p.into_vec();
    payload.extend(codec_box);
    payload.extend(nclx_rec709_limited_colr());
    mp4_box(fourcc, payload)
}

fn nclx_rec709_limited_colr() -> Vec<u8> {
    let mut p = Payload::new();
    p.bytes(b"nclx")
        .u16(1) // colour_primaries: BT.709
        .u16(1) // transfer_characteristics: BT.709
        .u16(1) // matrix_coefficients: BT.709
        .u8(0); // full_range_flag = 0 (limited/video range), 7 reserved bits
    mp4_box(*b"colr", p.into_vec())
}

fn avcc(sps: &[Vec<u8>], pps: &[Vec<u8>]) -> Vec<u8> {
    // avcC NAL-length fields are u16 by spec; real parameter sets are well
    // under 64 KiB, so a longer one signals an upstream bug, not valid input.
    debug_assert!(
        !sps.is_empty() && sps.len() <= 31,
        "avcC requires 1..=31 SPS entries"
    );
    debug_assert!(
        !pps.is_empty() && pps.len() <= u8::MAX as usize,
        "avcC requires 1..=255 PPS entries"
    );
    debug_assert!(
        sps.iter()
            .chain(pps)
            .all(|nal| nal.len() <= u16::MAX as usize),
        "AVC parameter set exceeds avcC u16 length"
    );
    let primary_sps = &sps[0];
    let mut p = Payload::new();
    p.u8(1) // configurationVersion
        .u8(primary_sps.get(1).copied().unwrap_or(0)) // AVCProfileIndication
        .u8(primary_sps.get(2).copied().unwrap_or(0)) // profile_compatibility
        .u8(primary_sps.get(3).copied().unwrap_or(0)) // AVCLevelIndication
        .u8(0xFF) // lengthSizeMinusOne = 3
        .u8(0xE0 | sps.len() as u8);
    for nal in sps {
        p.u16(nal.len() as u16).bytes(nal);
    }
    p.u8(pps.len() as u8);
    for nal in pps {
        p.u16(nal.len() as u16).bytes(nal);
    }
    mp4_box(*b"avcC", p.into_vec())
}

/// Empty stts/stsc/stsz/stco for the fragmented init moov.
pub(crate) fn empty_stbl_tail() -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::{children, find, walk};

    fn cfg() -> VideoTrackConfig {
        VideoTrackConfig::h264(
            1920,
            1080,
            90_000,
            vec![0x67, 0x64, 0x00, 0x28, 0xAA],
            vec![0x68, 0xEE, 0x3C, 0x80],
        )
    }

    fn box_version(bytes: &[u8], info: &crate::walker::BoxInfo) -> u8 {
        bytes[info.payload_offset as usize]
    }

    #[test]
    fn duration_headers_keep_version_zero_at_u32_max() {
        let duration = u32::MAX as u64;
        let movie = mvhd(duration, 2);
        let movie_box = walk(&movie).remove(0);
        let movie_payload = movie_box.payload_offset as usize;
        assert_eq!(box_version(&movie, &movie_box), 0);
        assert_eq!(movie_box.size, 108);
        assert_eq!(
            u32::from_be_bytes(
                movie[movie_payload + 16..movie_payload + 20]
                    .try_into()
                    .unwrap()
            ),
            u32::MAX
        );

        let track = video_trak_with_tables(&cfg(), 1, duration, duration, empty_stbl_tail());
        let trak = walk(&track).remove(0);
        let trak_children = children(&track, &trak);
        let tkhd = find(&trak_children, b"tkhd").unwrap();
        let tkhd_payload = tkhd.payload_offset as usize;
        assert_eq!(box_version(&track, tkhd), 0);
        assert_eq!(tkhd.size, 92);
        assert_eq!(
            u32::from_be_bytes(
                track[tkhd_payload + 20..tkhd_payload + 24]
                    .try_into()
                    .unwrap()
            ),
            u32::MAX
        );
        let mdia = find(&trak_children, b"mdia").unwrap();
        let mdhd = find(&children(&track, mdia), b"mdhd").unwrap().clone();
        let mdhd_payload = mdhd.payload_offset as usize;
        assert_eq!(box_version(&track, &mdhd), 0);
        assert_eq!(mdhd.size, 32);
        assert_eq!(
            u32::from_be_bytes(
                track[mdhd_payload + 16..mdhd_payload + 20]
                    .try_into()
                    .unwrap()
            ),
            u32::MAX
        );
    }

    #[test]
    fn duration_headers_use_version_one_and_preserve_first_u64_value() {
        let duration = u32::MAX as u64 + 1;
        let movie = mvhd(duration, 2);
        let movie_box = walk(&movie).remove(0);
        let movie_payload = movie_box.payload_offset as usize;
        assert_eq!(box_version(&movie, &movie_box), 1);
        assert_eq!(movie_box.size, 120);
        assert_eq!(
            u64::from_be_bytes(
                movie[movie_payload + 24..movie_payload + 32]
                    .try_into()
                    .unwrap()
            ),
            duration
        );

        let track = video_trak_with_tables(&cfg(), 1, duration, duration, empty_stbl_tail());
        let trak = walk(&track).remove(0);
        let trak_children = children(&track, &trak);
        let tkhd = find(&trak_children, b"tkhd").unwrap();
        let tkhd_payload = tkhd.payload_offset as usize;
        assert_eq!(box_version(&track, tkhd), 1);
        assert_eq!(tkhd.size, 104);
        assert_eq!(
            u64::from_be_bytes(
                track[tkhd_payload + 28..tkhd_payload + 36]
                    .try_into()
                    .unwrap()
            ),
            duration
        );
        let mdia = find(&trak_children, b"mdia").unwrap();
        let mdhd = find(&children(&track, mdia), b"mdhd").unwrap().clone();
        let mdhd_payload = mdhd.payload_offset as usize;
        assert_eq!(box_version(&track, &mdhd), 1);
        assert_eq!(mdhd.size, 44);
        assert_eq!(
            u64::from_be_bytes(
                track[mdhd_payload + 24..mdhd_payload + 32]
                    .try_into()
                    .unwrap()
            ),
            duration
        );
    }

    #[test]
    fn track_duration_headers_select_versions_independently() {
        let short = u32::MAX as u64;
        let long = short + 1;

        for (movie_duration, media_duration, expected_tkhd, expected_mdhd) in
            [(long, short, 1, 0), (short, long, 0, 1)]
        {
            let track = video_trak_with_tables(
                &cfg(),
                1,
                movie_duration,
                media_duration,
                empty_stbl_tail(),
            );
            let trak = walk(&track).remove(0);
            let trak_children = children(&track, &trak);
            let tkhd = find(&trak_children, b"tkhd").unwrap();
            assert_eq!(box_version(&track, tkhd), expected_tkhd);
            let mdia = find(&trak_children, b"mdia").unwrap();
            let mdhd = find(&children(&track, mdia), b"mdhd").unwrap().clone();
            assert_eq!(box_version(&track, &mdhd), expected_mdhd);
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

    fn audio_cfg() -> AudioTrackConfig {
        AudioTrackConfig {
            channels: 2,
            sample_rate: 48_000,
            pre_skip: 312,
        }
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

    #[test]
    fn multi_track_moov_has_one_trak_and_trex_per_track() {
        let tracks = vec![TrackConfig::Video(cfg()), TrackConfig::Audio(audio_cfg())];
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

    #[test]
    fn video_sample_entry_embeds_rec709_limited_color_info() {
        let buf = moov_init(&cfg());
        let colr = buf.windows(4).position(|w| w == b"colr").expect("colr box");
        let payload = &buf[colr + 4..colr + 15];

        assert_eq!(
            payload,
            &[
                b'n', b'c', b'l', b'x', 0, 1, // BT.709 primaries
                0, 1, // BT.709 transfer
                0, 1, // BT.709 matrix
                0, // limited range
            ]
        );
    }

    #[test]
    fn hevc_config_yields_hvc1_sample_entry_with_hvcc() {
        let cfg = VideoTrackConfig::hevc(
            128,
            72,
            90_000,
            vec![0x40, 0x01, 0x0C],
            vec![0x42, 0x01, 0x01],
            vec![0x44, 0x01, 0xC1],
        );
        let buf = moov_init(&cfg);
        assert!(buf.windows(4).any(|w| w == b"hvc1"));
        assert!(buf.windows(4).any(|w| w == b"hvcC"));
        assert!(!buf.windows(4).any(|w| w == b"avc1"));
        assert!(!buf.windows(4).any(|w| w == b"avcC"));
    }

    #[test]
    fn av1_config_yields_av01_sample_entry_with_av1c() {
        let seq = vec![0x0A, 0x02, 0x4B, 0x00];
        let cfg = VideoTrackConfig::av1(128, 72, 90_000, seq.clone());
        let buf = moov_init(&cfg);
        assert!(buf.windows(4).any(|w| w == b"av01"));
        assert!(buf.windows(4).any(|w| w == b"av1C"));
        assert!(!buf.windows(4).any(|w| w == b"avc1"));
        // The sequence header OBU is embedded verbatim as configOBUs.
        assert!(buf.windows(seq.len()).any(|w| w == seq.as_slice()));
    }
}
