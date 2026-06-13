//! Keyframe-aligned stream-copy trim for finalized Clipline MP4s.

use std::fs::File;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

use crate::walker::{children, find, walk, BoxInfo};
use crate::{
    AudioTrackConfig, FragSample, HybridMp4Writer, SourceSample, TrackConfig, VideoCodecParams,
    VideoTrackConfig,
};

#[derive(Debug, Clone, PartialEq)]
pub struct TrimInfo {
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub aligned_start_s: f64,
    pub aligned_end_s: f64,
    pub duration_s: f64,
}

#[derive(Debug)]
pub enum TrimError {
    InvalidRange(String),
    Unsupported(String),
    Corrupt(String),
    Io(std::io::Error),
}

impl std::fmt::Display for TrimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange(message) => write!(f, "invalid trim range: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported mp4: {message}"),
            Self::Corrupt(message) => write!(f, "corrupt mp4: {message}"),
            Self::Io(e) => write!(f, "mp4 trim io: {e}"),
        }
    }
}

impl std::error::Error for TrimError {}

impl From<std::io::Error> for TrimError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn trim_keyframe_aligned(
    input: &[u8],
    start_s: f64,
    end_s: f64,
) -> Result<(Vec<u8>, TrimInfo), TrimError> {
    let mut out = Cursor::new(Vec::new());
    let info = trim_keyframe_aligned_to_writer(input, start_s, end_s, &mut out)?;
    Ok((out.into_inner(), info))
}

pub fn trim_keyframe_aligned_to_writer<W: Write + Seek>(
    input: &[u8],
    start_s: f64,
    end_s: f64,
    output: W,
) -> Result<TrimInfo, TrimError> {
    validate_range(start_s, end_s)?;
    let movie = parse_movie(input)?;
    let selection = select_trim_range(&movie, start_s, end_s)?;

    let mut selected: Vec<Vec<FragSample>> = Vec::with_capacity(movie.tracks.len());
    for (idx, track) in movie.tracks.iter().enumerate() {
        let samples: Vec<FragSample> = if idx == selection.video_idx {
            track.samples[selection.start_idx..selection.end_idx]
                .iter()
                .map(|s| s.to_frag_sample(input))
                .collect::<Result<_, _>>()?
        } else {
            track
                .samples
                .iter()
                .filter(|s| {
                    let start = s.start_s(track.timescale);
                    start >= selection.aligned_start_s && start < selection.aligned_end_s
                })
                .map(|s| s.to_frag_sample(input))
                .collect::<Result<_, _>>()?
        };
        selected.push(samples);
    }

    let tracks: Vec<TrackConfig> = movie.tracks.iter().map(|t| t.cfg.clone()).collect();
    let mut writer = HybridMp4Writer::new_multi(output, tracks)?;
    let refs: Vec<&[FragSample]> = selected.iter().map(Vec::as_slice).collect();
    writer.write_fragment_multi(&refs)?;
    let _ = writer.finalize()?;

    Ok(selection.info(start_s, end_s))
}

pub fn trim_keyframe_aligned_file(
    source: &Path,
    target: &Path,
    start_s: f64,
    end_s: f64,
) -> Result<TrimInfo, TrimError> {
    validate_range(start_s, end_s)?;
    reject_same_file(source, target)?;
    let input = std::fs::read(source)?;
    let movie = parse_movie(&input)?;
    let selection = select_trim_range(&movie, start_s, end_s)?;
    let mut per_track: Vec<Vec<SourceSample>> = Vec::with_capacity(movie.tracks.len());
    for (idx, track) in movie.tracks.iter().enumerate() {
        let samples: Vec<SourceSample> = if idx == selection.video_idx {
            track.samples[selection.start_idx..selection.end_idx]
                .iter()
                .map(SampleRecord::to_source_sample)
                .collect()
        } else {
            track
                .samples
                .iter()
                .filter(|s| {
                    let start = s.start_s(track.timescale);
                    start >= selection.aligned_start_s && start < selection.aligned_end_s
                })
                .map(SampleRecord::to_source_sample)
                .collect()
        };
        per_track.push(samples);
    }

    let tracks: Vec<TrackConfig> = movie.tracks.iter().map(|t| t.cfg.clone()).collect();
    let mut source_file = File::open(source)?;
    let target_file = File::create(target)?;
    let mut writer = HybridMp4Writer::new_multi(target_file, tracks)?;
    let refs: Vec<&[SourceSample]> = per_track.iter().map(Vec::as_slice).collect();
    writer.write_fragment_multi_from_source(&mut source_file, &refs)?;
    let _ = writer.finalize()?;
    Ok(selection.info(start_s, end_s))
}

fn reject_same_file(source: &Path, target: &Path) -> Result<(), TrimError> {
    let source = std::fs::canonicalize(source)?;
    if let Ok(target) = std::fs::canonicalize(target) {
        if source == target {
            return Err(TrimError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "trim source and target must be different files",
            )));
        }
    }
    Ok(())
}

struct ParsedMovie {
    tracks: Vec<ParsedTrack>,
}

struct TrimSelection {
    video_idx: usize,
    start_idx: usize,
    end_idx: usize,
    aligned_start_s: f64,
    aligned_end_s: f64,
}

struct ParsedTrack {
    cfg: TrackConfig,
    timescale: u32,
    samples: Vec<SampleRecord>,
}

struct SampleRecord {
    offset: usize,
    size: u32,
    duration: u32,
    is_sync: bool,
    start_ticks: u64,
}

impl ParsedTrack {
    fn track_end_s(&self) -> f64 {
        self.samples
            .last()
            .map(|s| s.end_s(self.timescale))
            .unwrap_or(0.0)
    }
}

impl SampleRecord {
    fn start_s(&self, timescale: u32) -> f64 {
        self.start_ticks as f64 / timescale as f64
    }

    fn end_s(&self, timescale: u32) -> f64 {
        (self.start_ticks + self.duration as u64) as f64 / timescale as f64
    }

    fn to_frag_sample(&self, input: &[u8]) -> Result<FragSample, TrimError> {
        let start = self.offset;
        let end = start
            .checked_add(self.size as usize)
            .ok_or_else(|| TrimError::Corrupt("sample byte range overflow".into()))?;
        let data = input
            .get(start..end)
            .ok_or_else(|| TrimError::Corrupt("sample byte range is outside file".into()))?
            .to_vec();
        Ok(FragSample {
            data,
            duration: self.duration,
            is_sync: self.is_sync,
        })
    }

    fn to_source_sample(&self) -> SourceSample {
        SourceSample {
            offset: self.offset as u64,
            size: self.size,
            duration: self.duration,
            is_sync: self.is_sync,
        }
    }
}

impl TrimSelection {
    fn info(&self, requested_start_s: f64, requested_end_s: f64) -> TrimInfo {
        TrimInfo {
            requested_start_s,
            requested_end_s,
            aligned_start_s: self.aligned_start_s,
            aligned_end_s: self.aligned_end_s,
            duration_s: self.aligned_end_s - self.aligned_start_s,
        }
    }
}

fn select_trim_range(
    movie: &ParsedMovie,
    start_s: f64,
    end_s: f64,
) -> Result<TrimSelection, TrimError> {
    let video_idx = movie
        .tracks
        .iter()
        .position(|t| matches!(t.cfg, TrackConfig::Video(_)))
        .ok_or_else(|| TrimError::Unsupported("missing video track".into()))?;
    let video = &movie.tracks[video_idx];
    let video_end_s = video.track_end_s();
    if start_s >= video_end_s {
        return Err(TrimError::InvalidRange("start is past the clip end".into()));
    }

    let start_idx = video
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_sync && s.start_s(video.timescale) <= start_s)
        .map(|(i, _)| i)
        .next_back()
        .or_else(|| video.samples.iter().position(|s| s.is_sync))
        .ok_or_else(|| TrimError::Unsupported("video track has no sync samples".into()))?;

    let end_idx = video
        .samples
        .iter()
        .enumerate()
        .skip(start_idx + 1)
        .find(|(_, s)| s.is_sync && s.start_s(video.timescale) >= end_s)
        .map(|(i, _)| i)
        .unwrap_or(video.samples.len());

    let aligned_start_s = video.samples[start_idx].start_s(video.timescale);
    let aligned_end_s = if end_idx < video.samples.len() {
        video.samples[end_idx].start_s(video.timescale)
    } else {
        video.track_end_s()
    };
    if aligned_end_s <= aligned_start_s {
        return Err(TrimError::InvalidRange(
            "aligned range does not contain a video sample".into(),
        ));
    }
    Ok(TrimSelection {
        video_idx,
        start_idx,
        end_idx,
        aligned_start_s,
        aligned_end_s,
    })
}

fn parse_movie(input: &[u8]) -> Result<ParsedMovie, TrimError> {
    let top = walk(input);
    let moov = find(&top, b"moov")
        .ok_or_else(|| TrimError::Unsupported("missing finalized moov".into()))?
        .clone();
    let moov_children = children(input, &moov);
    if find(&moov_children, b"mvex").is_some() {
        return Err(TrimError::Unsupported(
            "fragmented/unfinalized files are not trim-ready".into(),
        ));
    }

    let tracks: Vec<ParsedTrack> = moov_children
        .iter()
        .filter(|b| &b.fourcc == b"trak")
        .map(|trak| parse_track(input, trak))
        .collect::<Result<_, _>>()?;
    if tracks.is_empty() {
        return Err(TrimError::Unsupported("no tracks found".into()));
    }
    Ok(ParsedMovie { tracks })
}

fn parse_track(input: &[u8], trak: &BoxInfo) -> Result<ParsedTrack, TrimError> {
    let mdia = require_child(input, trak, b"mdia")?;
    let mdhd = require_child(input, &mdia, b"mdhd")?;
    let timescale = parse_mdhd_timescale(input, &mdhd)?;
    let hdlr = require_child(input, &mdia, b"hdlr")?;
    let handler = parse_hdlr(input, &hdlr)?;
    let minf = require_child(input, &mdia, b"minf")?;
    let stbl = require_child(input, &minf, b"stbl")?;
    let stsd = require_child(input, &stbl, b"stsd")?;
    let cfg = parse_stsd(input, &stsd, handler, timescale)?;
    let samples = parse_sample_table(input, &stbl)?;
    if samples.is_empty() {
        return Err(TrimError::Unsupported("track has no samples".into()));
    }
    Ok(ParsedTrack {
        cfg,
        timescale,
        samples,
    })
}

fn validate_range(start_s: f64, end_s: f64) -> Result<(), TrimError> {
    if !start_s.is_finite() || !end_s.is_finite() {
        return Err(TrimError::InvalidRange(
            "start and end must be finite".into(),
        ));
    }
    if start_s < 0.0 {
        return Err(TrimError::InvalidRange("start must be non-negative".into()));
    }
    if end_s <= start_s {
        return Err(TrimError::InvalidRange(
            "end must be greater than start".into(),
        ));
    }
    Ok(())
}

fn parse_mdhd_timescale(input: &[u8], mdhd: &BoxInfo) -> Result<u32, TrimError> {
    let p = mdhd.payload_offset as usize;
    let version = *input
        .get(p)
        .ok_or_else(|| TrimError::Corrupt("truncated mdhd".into()))?;
    let ts_off = match version {
        0 => p + 12,
        1 => p + 20,
        _ => return Err(TrimError::Unsupported("unknown mdhd version".into())),
    };
    let timescale = read_u32(input, ts_off)?;
    if timescale == 0 {
        return Err(TrimError::Corrupt("zero track timescale".into()));
    }
    Ok(timescale)
}

fn parse_hdlr(input: &[u8], hdlr: &BoxInfo) -> Result<[u8; 4], TrimError> {
    let p = hdlr.payload_offset as usize;
    read_fourcc(input, p + 8)
}

fn parse_stsd(
    input: &[u8],
    stsd: &BoxInfo,
    handler: [u8; 4],
    timescale: u32,
) -> Result<TrackConfig, TrimError> {
    let p = stsd.payload_offset as usize;
    let entry_count = read_u32(input, p + 4)?;
    if entry_count != 1 {
        return Err(TrimError::Unsupported(
            "expected exactly one sample description".into(),
        ));
    }
    let entry = read_box_at(input, p + 8, box_end(stsd)?)?;
    match &handler {
        b"vide" => parse_video_stsd(input, &entry, timescale),
        b"soun" => parse_audio_stsd(input, &entry),
        _ => Err(TrimError::Unsupported(format!(
            "unsupported handler {}",
            fourcc_str(&handler)
        ))),
    }
}

fn parse_video_stsd(
    input: &[u8],
    entry: &BoxInfo,
    timescale: u32,
) -> Result<TrackConfig, TrimError> {
    let p = entry.payload_offset as usize;
    let entry_end = box_end(entry)?;
    if p + 78 > entry_end {
        return Err(TrimError::Corrupt("truncated visual sample entry".into()));
    }
    let width = read_u16(input, p + 24)?;
    let height = read_u16(input, p + 26)?;
    // The codec configuration box follows the 78-byte VisualSampleEntry
    // shell, which is identical for avc1/hvc1/av01.
    let codec = match &entry.fourcc {
        b"avc1" => {
            let avcc = find_box_between(input, p + 78, entry_end, b"avcC")?
                .ok_or_else(|| TrimError::Unsupported("missing avcC".into()))?;
            let (sps, pps) = parse_avcc(input, &avcc)?;
            VideoCodecParams::H264 { sps, pps }
        }
        b"hvc1" | b"hev1" => {
            let hvcc = find_box_between(input, p + 78, entry_end, b"hvcC")?
                .ok_or_else(|| TrimError::Unsupported("missing hvcC".into()))?;
            let (vps, sps, pps) = parse_hvcc(input, &hvcc)?;
            VideoCodecParams::Hevc { vps, sps, pps }
        }
        b"av01" => {
            let av1c = find_box_between(input, p + 78, entry_end, b"av1C")?
                .ok_or_else(|| TrimError::Unsupported("missing av1C".into()))?;
            let sequence_header_obu = parse_av1c(input, &av1c)?;
            VideoCodecParams::Av1 {
                sequence_header_obu,
            }
        }
        other => {
            return Err(TrimError::Unsupported(format!(
                "unsupported video sample entry {}",
                fourcc_str(other)
            )))
        }
    };
    Ok(TrackConfig::Video(VideoTrackConfig {
        width,
        height,
        timescale,
        codec,
    }))
}

fn parse_audio_stsd(input: &[u8], entry: &BoxInfo) -> Result<TrackConfig, TrimError> {
    if &entry.fourcc != b"Opus" {
        return Err(TrimError::Unsupported(format!(
            "unsupported audio sample entry {}",
            fourcc_str(&entry.fourcc)
        )));
    }
    let p = entry.payload_offset as usize;
    let entry_end = box_end(entry)?;
    if p + 28 > entry_end {
        return Err(TrimError::Corrupt("truncated Opus sample entry".into()));
    }
    let channels = read_u16(input, p + 16)?;
    let dops = find_box_between(input, p + 28, entry_end, b"dOps")?
        .ok_or_else(|| TrimError::Unsupported("missing dOps".into()))?;
    let dp = dops.payload_offset as usize;
    let pre_skip = read_u16(input, dp + 2)?;
    let sample_rate = read_u32(input, dp + 4)?;
    Ok(TrackConfig::Audio(AudioTrackConfig {
        channels,
        sample_rate,
        pre_skip,
    }))
}

fn parse_avcc(input: &[u8], avcc: &BoxInfo) -> Result<(Vec<u8>, Vec<u8>), TrimError> {
    let p = avcc.payload_offset as usize;
    let end = box_end(avcc)?;
    if p + 7 > end {
        return Err(TrimError::Corrupt("truncated avcC".into()));
    }
    let sps_count = input[p + 5] & 0x1f;
    if sps_count == 0 {
        return Err(TrimError::Unsupported("avcC has no SPS".into()));
    }
    let mut pos = p + 6;
    let mut sps = None;
    for i in 0..sps_count {
        let len = read_u16(input, pos)? as usize;
        pos += 2;
        let data = read_slice(input, pos, len, end)?.to_vec();
        pos += len;
        if i == 0 {
            sps = Some(data);
        }
    }
    let pps_count = *input
        .get(pos)
        .ok_or_else(|| TrimError::Corrupt("truncated avcC PPS count".into()))?;
    pos += 1;
    if pps_count == 0 {
        return Err(TrimError::Unsupported("avcC has no PPS".into()));
    }
    let pps_len = read_u16(input, pos)? as usize;
    pos += 2;
    let pps = read_slice(input, pos, pps_len, end)?.to_vec();
    Ok((sps.unwrap(), pps))
}

/// (VPS, SPS, PPS) raw NAL units recovered from an `hvcC` record.
type HevcParamSets = (Vec<u8>, Vec<u8>, Vec<u8>);

/// Recover the first VPS/SPS/PPS NAL from an `hvcC` NAL-array section.
fn parse_hvcc(input: &[u8], hvcc: &BoxInfo) -> Result<HevcParamSets, TrimError> {
    let p = hvcc.payload_offset as usize;
    let end = box_end(hvcc)?;
    // The fixed configuration prefix is 22 bytes; numOfArrays is the 23rd.
    if p + 23 > end {
        return Err(TrimError::Corrupt("truncated hvcC".into()));
    }
    let num_arrays = input[p + 22];
    let mut pos = p + 23;
    let mut vps = None;
    let mut sps = None;
    let mut pps = None;
    for _ in 0..num_arrays {
        let nal_type = *input
            .get(pos)
            .ok_or_else(|| TrimError::Corrupt("truncated hvcC array header".into()))?
            & 0x3F;
        pos += 1;
        let num_nalus = read_u16(input, pos)?;
        pos += 2;
        for i in 0..num_nalus {
            let len = read_u16(input, pos)? as usize;
            pos += 2;
            let data = read_slice(input, pos, len, end)?.to_vec();
            pos += len;
            if i == 0 {
                match nal_type {
                    32 => vps = Some(data),
                    33 => sps = Some(data),
                    34 => pps = Some(data),
                    _ => {}
                }
            }
        }
    }
    match (vps, sps, pps) {
        (Some(vps), Some(sps), Some(pps)) => Ok((vps, sps, pps)),
        _ => Err(TrimError::Unsupported("hvcC missing VPS/SPS/PPS".into())),
    }
}

/// The `av1C` configOBUs payload is the sequence-header OBU verbatim.
fn parse_av1c(input: &[u8], av1c: &BoxInfo) -> Result<Vec<u8>, TrimError> {
    let p = av1c.payload_offset as usize;
    let end = box_end(av1c)?;
    // 4-byte fixed configuration record, then configOBUs.
    if p + 4 > end {
        return Err(TrimError::Corrupt("truncated av1C".into()));
    }
    let obu = read_slice(input, p + 4, end - (p + 4), end)?.to_vec();
    if obu.is_empty() {
        return Err(TrimError::Unsupported("av1C has no configOBUs".into()));
    }
    Ok(obu)
}

fn parse_sample_table(input: &[u8], stbl: &BoxInfo) -> Result<Vec<SampleRecord>, TrimError> {
    let stts = require_child(input, stbl, b"stts")?;
    let durations = parse_stts(input, &stts)?;
    let stsz = require_child(input, stbl, b"stsz")?;
    let sizes = parse_stsz(input, &stsz)?;
    if durations.len() != sizes.len() {
        return Err(TrimError::Corrupt(format!(
            "stts/stsz sample count mismatch: {} vs {}",
            durations.len(),
            sizes.len()
        )));
    }
    let sync = match child(input, stbl, b"stss") {
        Some(stss) => parse_stss(input, &stss, sizes.len())?,
        None => vec![true; sizes.len()],
    };
    let stsc = require_child(input, stbl, b"stsc")?;
    let chunk_offsets = if let Some(co64) = child(input, stbl, b"co64") {
        parse_co64(input, &co64)?
    } else {
        let stco = require_child(input, stbl, b"stco")?;
        parse_stco(input, &stco)?
    };
    let samples_per_chunk = parse_stsc(input, &stsc, chunk_offsets.len())?;
    records_from_tables(
        input,
        &sizes,
        &durations,
        &sync,
        &chunk_offsets,
        &samples_per_chunk,
    )
}

fn parse_stts(input: &[u8], stts: &BoxInfo) -> Result<Vec<u32>, TrimError> {
    let p = stts.payload_offset as usize;
    let count = read_u32(input, p + 4)? as usize;
    let end = box_end(stts)?;
    let mut pos = p + 8;
    let mut out = Vec::new();
    for _ in 0..count {
        if pos + 8 > end {
            return Err(TrimError::Corrupt("truncated stts".into()));
        }
        let sample_count = read_u32(input, pos)?;
        let delta = read_u32(input, pos + 4)?;
        out.extend(std::iter::repeat_n(delta, sample_count as usize));
        pos += 8;
    }
    Ok(out)
}

fn parse_stsz(input: &[u8], stsz: &BoxInfo) -> Result<Vec<u32>, TrimError> {
    let p = stsz.payload_offset as usize;
    let sample_size = read_u32(input, p + 4)?;
    let sample_count = read_u32(input, p + 8)? as usize;
    if sample_size != 0 {
        return Ok(vec![sample_size; sample_count]);
    }
    let end = box_end(stsz)?;
    let mut pos = p + 12;
    let mut out = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        if pos + 4 > end {
            return Err(TrimError::Corrupt("truncated stsz".into()));
        }
        out.push(read_u32(input, pos)?);
        pos += 4;
    }
    Ok(out)
}

fn parse_stss(input: &[u8], stss: &BoxInfo, sample_count: usize) -> Result<Vec<bool>, TrimError> {
    let p = stss.payload_offset as usize;
    let entry_count = read_u32(input, p + 4)? as usize;
    let end = box_end(stss)?;
    let mut pos = p + 8;
    let mut sync = vec![false; sample_count];
    for _ in 0..entry_count {
        if pos + 4 > end {
            return Err(TrimError::Corrupt("truncated stss".into()));
        }
        let n = read_u32(input, pos)? as usize;
        if n == 0 || n > sample_count {
            return Err(TrimError::Corrupt("stss sample number out of range".into()));
        }
        sync[n - 1] = true;
        pos += 4;
    }
    Ok(sync)
}

fn parse_co64(input: &[u8], co64: &BoxInfo) -> Result<Vec<u64>, TrimError> {
    let p = co64.payload_offset as usize;
    let count = read_u32(input, p + 4)? as usize;
    let end = box_end(co64)?;
    let mut pos = p + 8;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 8 > end {
            return Err(TrimError::Corrupt("truncated co64".into()));
        }
        out.push(read_u64(input, pos)?);
        pos += 8;
    }
    Ok(out)
}

fn parse_stco(input: &[u8], stco: &BoxInfo) -> Result<Vec<u64>, TrimError> {
    let p = stco.payload_offset as usize;
    let count = read_u32(input, p + 4)? as usize;
    let end = box_end(stco)?;
    let mut pos = p + 8;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 4 > end {
            return Err(TrimError::Corrupt("truncated stco".into()));
        }
        out.push(read_u32(input, pos)? as u64);
        pos += 4;
    }
    Ok(out)
}

fn parse_stsc(input: &[u8], stsc: &BoxInfo, chunk_count: usize) -> Result<Vec<u32>, TrimError> {
    let p = stsc.payload_offset as usize;
    let entry_count = read_u32(input, p + 4)? as usize;
    if entry_count == 0 && chunk_count > 0 {
        return Err(TrimError::Corrupt("stsc has no entries".into()));
    }
    let end = box_end(stsc)?;
    let mut pos = p + 8;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        if pos + 12 > end {
            return Err(TrimError::Corrupt("truncated stsc".into()));
        }
        let first_chunk = read_u32(input, pos)?;
        let samples_per_chunk = read_u32(input, pos + 4)?;
        if first_chunk == 0 || samples_per_chunk == 0 {
            return Err(TrimError::Corrupt("invalid stsc entry".into()));
        }
        entries.push((first_chunk, samples_per_chunk));
        pos += 12;
    }
    if chunk_count == 0 {
        return Ok(Vec::new());
    }
    if entries.first().map(|e| e.0) != Some(1) {
        return Err(TrimError::Corrupt(
            "first stsc entry must start at chunk 1".into(),
        ));
    }

    let mut out = Vec::with_capacity(chunk_count);
    let mut entry_idx = 0usize;
    for chunk_number in 1..=chunk_count as u32 {
        while entry_idx + 1 < entries.len() && entries[entry_idx + 1].0 <= chunk_number {
            entry_idx += 1;
        }
        out.push(entries[entry_idx].1);
    }
    Ok(out)
}

fn records_from_tables(
    input: &[u8],
    sizes: &[u32],
    durations: &[u32],
    sync: &[bool],
    chunk_offsets: &[u64],
    samples_per_chunk: &[u32],
) -> Result<Vec<SampleRecord>, TrimError> {
    let expected: usize = samples_per_chunk.iter().map(|&n| n as usize).sum();
    if expected != sizes.len() {
        return Err(TrimError::Corrupt(format!(
            "stsc sample count {expected} does not match stsz count {}",
            sizes.len()
        )));
    }

    let mut out = Vec::with_capacity(sizes.len());
    let mut sample_index = 0usize;
    let mut start_ticks = 0u64;
    for (&chunk_offset, &chunk_samples) in chunk_offsets.iter().zip(samples_per_chunk) {
        let mut offset = usize::try_from(chunk_offset)
            .map_err(|_| TrimError::Corrupt("chunk offset too large".into()))?;
        for _ in 0..chunk_samples {
            let size = sizes[sample_index];
            let end = offset
                .checked_add(size as usize)
                .ok_or_else(|| TrimError::Corrupt("sample offset overflow".into()))?;
            if end > input.len() {
                return Err(TrimError::Corrupt(
                    "sample points outside source file".into(),
                ));
            }
            out.push(SampleRecord {
                offset,
                size,
                duration: durations[sample_index],
                is_sync: sync[sample_index],
                start_ticks,
            });
            start_ticks += durations[sample_index] as u64;
            offset = end;
            sample_index += 1;
        }
    }
    Ok(out)
}

fn child(input: &[u8], parent: &BoxInfo, fourcc: &[u8; 4]) -> Option<BoxInfo> {
    children(input, parent)
        .into_iter()
        .find(|b| &b.fourcc == fourcc)
}

fn require_child(input: &[u8], parent: &BoxInfo, fourcc: &[u8; 4]) -> Result<BoxInfo, TrimError> {
    child(input, parent, fourcc)
        .ok_or_else(|| TrimError::Unsupported(format!("missing {} box", fourcc_str(fourcc))))
}

fn find_box_between(
    input: &[u8],
    mut offset: usize,
    end: usize,
    fourcc: &[u8; 4],
) -> Result<Option<BoxInfo>, TrimError> {
    while offset + 8 <= end {
        let b = read_box_at(input, offset, end)?;
        let next = box_end(&b)?;
        if &b.fourcc == fourcc {
            return Ok(Some(b));
        }
        if next <= offset {
            return Err(TrimError::Corrupt("box parser made no progress".into()));
        }
        offset = next;
    }
    Ok(None)
}

fn read_box_at(input: &[u8], offset: usize, limit: usize) -> Result<BoxInfo, TrimError> {
    if offset + 8 > limit || offset + 8 > input.len() {
        return Err(TrimError::Corrupt("truncated box header".into()));
    }
    let size32 = read_u32(input, offset)?;
    let fourcc = read_fourcc(input, offset + 4)?;
    let (size, header) = if size32 == 1 {
        if offset + 16 > limit || offset + 16 > input.len() {
            return Err(TrimError::Corrupt("truncated largesize box header".into()));
        }
        (read_u64(input, offset + 8)?, 16u64)
    } else if size32 == 0 {
        ((limit - offset) as u64, 8u64)
    } else {
        (size32 as u64, 8u64)
    };
    let end = offset
        .checked_add(size as usize)
        .ok_or_else(|| TrimError::Corrupt("box size overflow".into()))?;
    if size < header || end > limit || end > input.len() {
        return Err(TrimError::Corrupt(format!(
            "invalid {} box size",
            fourcc_str(&fourcc)
        )));
    }
    Ok(BoxInfo {
        fourcc,
        offset: offset as u64,
        size,
        payload_offset: offset as u64 + header,
    })
}

fn box_end(b: &BoxInfo) -> Result<usize, TrimError> {
    usize::try_from(b.offset + b.size)
        .map_err(|_| TrimError::Corrupt("box end offset too large".into()))
}

fn read_slice(input: &[u8], offset: usize, len: usize, limit: usize) -> Result<&[u8], TrimError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| TrimError::Corrupt("slice offset overflow".into()))?;
    if end > limit {
        return Err(TrimError::Corrupt(
            "slice extends past containing box".into(),
        ));
    }
    input
        .get(offset..end)
        .ok_or_else(|| TrimError::Corrupt("slice extends past file".into()))
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16, TrimError> {
    Ok(u16::from_be_bytes(
        input
            .get(offset..offset + 2)
            .ok_or_else(|| TrimError::Corrupt("truncated u16".into()))?
            .try_into()
            .unwrap(),
    ))
}

fn read_u32(input: &[u8], offset: usize) -> Result<u32, TrimError> {
    Ok(u32::from_be_bytes(
        input
            .get(offset..offset + 4)
            .ok_or_else(|| TrimError::Corrupt("truncated u32".into()))?
            .try_into()
            .unwrap(),
    ))
}

fn read_u64(input: &[u8], offset: usize) -> Result<u64, TrimError> {
    Ok(u64::from_be_bytes(
        input
            .get(offset..offset + 8)
            .ok_or_else(|| TrimError::Corrupt("truncated u64".into()))?
            .try_into()
            .unwrap(),
    ))
}

fn read_fourcc(input: &[u8], offset: usize) -> Result<[u8; 4], TrimError> {
    let mut out = [0u8; 4];
    out.copy_from_slice(
        input
            .get(offset..offset + 4)
            .ok_or_else(|| TrimError::Corrupt("truncated fourcc".into()))?,
    );
    Ok(out)
}

fn fourcc_str(fourcc: &[u8; 4]) -> String {
    String::from_utf8_lossy(fourcc).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioTrackConfig, VideoTrackConfig};

    fn tracks() -> Vec<TrackConfig> {
        vec![
            TrackConfig::Video(VideoTrackConfig::h264(
                128,
                72,
                90_000,
                vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
                vec![0x68, 0xEE, 0x38, 0x80],
            )),
            TrackConfig::Audio(AudioTrackConfig {
                channels: 2,
                sample_rate: 48_000,
                pre_skip: 312,
            }),
        ]
    }

    fn video_gop(start: u32) -> Vec<FragSample> {
        (0..10)
            .map(|i| FragSample {
                data: format!("V{:05}", start + i).into_bytes(),
                duration: 9_000,
                is_sync: i == 0,
            })
            .collect()
    }

    fn audio_packets(start: u32) -> Vec<FragSample> {
        (0..50)
            .map(|i| FragSample {
                data: format!("A{:05}", start + i).into_bytes(),
                duration: 960,
                is_sync: true,
            })
            .collect()
    }

    fn clipline_fixture() -> Vec<u8> {
        let mut w = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), tracks()).unwrap();
        for second in 0..3 {
            let v = video_gop(second * 10);
            let a = audio_packets(second * 50);
            w.write_fragment_multi(&[&v, &a]).unwrap();
        }
        w.finalize().unwrap().into_inner()
    }

    #[test]
    fn parse_clipline_mp4_recovers_tracks_and_samples() {
        let movie = parse_movie(&clipline_fixture()).unwrap();

        assert_eq!(movie.tracks.len(), 2);
        assert_eq!(movie.tracks[0].samples.len(), 30);
        assert_eq!(movie.tracks[1].samples.len(), 150);
        assert!(movie.tracks[0].samples[0].is_sync);
        assert!(movie.tracks[0].samples[10].is_sync);
        assert!(!movie.tracks[0].samples[11].is_sync);
    }

    #[test]
    fn rejects_unfinalized_or_missing_sample_tables() {
        let mut w = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), tracks()).unwrap();
        let v = video_gop(0);
        let a = audio_packets(0);
        w.write_fragment_multi(&[&v, &a]).unwrap();
        let fragmented = w.into_inner().into_inner();

        assert!(parse_movie(&fragmented).is_err());
    }

    #[test]
    fn trims_to_previous_and_next_keyframes() {
        let (out, info) = trim_keyframe_aligned(&clipline_fixture(), 0.4, 1.2).unwrap();
        let movie = parse_movie(&out).unwrap();

        assert_eq!(info.aligned_start_s, 0.0);
        assert_eq!(info.aligned_end_s, 2.0);
        assert_eq!(movie.tracks[0].samples.len(), 20);
        assert_eq!(movie.tracks[1].samples.len(), 100);
        assert!(out.windows(6).any(|w| w == b"V00000"));
        assert!(out.windows(6).any(|w| w == b"V00019"));
        assert!(!out.windows(6).any(|w| w == b"V00020"));
    }

    // Real x265 / SVT-AV1 parameter sets (128x72) so the round-trip parses
    // genuine hvcC / av1C records, not just placeholder bytes.
    const HEVC_VPS: &[u8] = &[0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF, 0x01];
    const HEVC_SPS: &[u8] = &[
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x1E, 0xA0, 0x10, 0x20, 0x49, 0x65, 0x95, 0x9A, 0x49, 0x32, 0xBC, 0x05, 0xA0,
        0x20, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x03, 0xC1,
    ];
    const HEVC_PPS: &[u8] = &[0x44, 0x01, 0xC1, 0x72, 0xB4, 0x22, 0x40];
    const AV1_SEQ_OBU: &[u8] = &[
        0x0A, 0x0A, 0x00, 0x00, 0x00, 0x03, 0x37, 0xF8, 0xE3, 0x57, 0xCC, 0x02,
    ];

    fn single_video_fixture(codec: VideoCodecParams) -> Vec<u8> {
        let cfg = vec![TrackConfig::Video(VideoTrackConfig {
            width: 128,
            height: 72,
            timescale: 90_000,
            codec,
        })];
        let mut w = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), cfg).unwrap();
        for second in 0..3 {
            w.write_fragment_multi(&[&video_gop(second * 10)]).unwrap();
        }
        w.finalize().unwrap().into_inner()
    }

    #[test]
    fn trims_hevc_clip_recovering_parameter_sets() {
        let fixture = single_video_fixture(VideoCodecParams::Hevc {
            vps: HEVC_VPS.to_vec(),
            sps: HEVC_SPS.to_vec(),
            pps: HEVC_PPS.to_vec(),
        });
        let (out, info) = trim_keyframe_aligned(&fixture, 0.4, 1.2).unwrap();
        let movie = parse_movie(&out).unwrap();
        assert_eq!(info.aligned_start_s, 0.0);
        assert_eq!(info.aligned_end_s, 2.0);
        assert_eq!(movie.tracks[0].samples.len(), 20);
        match &movie.tracks[0].cfg {
            TrackConfig::Video(VideoTrackConfig {
                codec: VideoCodecParams::Hevc { vps, sps, pps },
                ..
            }) => {
                assert_eq!(vps.as_slice(), HEVC_VPS);
                assert_eq!(sps.as_slice(), HEVC_SPS);
                assert_eq!(pps.as_slice(), HEVC_PPS);
            }
            other => panic!("expected HEVC track, got {other:?}"),
        }
        assert!(out.windows(4).any(|w| w == b"hvc1"));
    }

    #[test]
    fn trims_av1_clip_recovering_sequence_header() {
        let fixture = single_video_fixture(VideoCodecParams::Av1 {
            sequence_header_obu: AV1_SEQ_OBU.to_vec(),
        });
        let (out, info) = trim_keyframe_aligned(&fixture, 0.4, 1.2).unwrap();
        let movie = parse_movie(&out).unwrap();
        assert_eq!(info.aligned_end_s, 2.0);
        assert_eq!(movie.tracks[0].samples.len(), 20);
        match &movie.tracks[0].cfg {
            TrackConfig::Video(VideoTrackConfig {
                codec:
                    VideoCodecParams::Av1 {
                        sequence_header_obu,
                    },
                ..
            }) => assert_eq!(sequence_header_obu.as_slice(), AV1_SEQ_OBU),
            other => panic!("expected AV1 track, got {other:?}"),
        }
        assert!(out.windows(4).any(|w| w == b"av01"));
    }

    #[test]
    fn file_trim_matches_in_memory_trim_output() {
        let input = clipline_fixture();
        let (expected, expected_info) = trim_keyframe_aligned(&input, 0.4, 1.2).unwrap();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "clipline-trim-file-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.mp4");
        let target = dir.join("target.mp4");
        std::fs::write(&source, &input).unwrap();

        let info = trim_keyframe_aligned_file(&source, &target, 0.4, 1.2).unwrap();
        let actual = std::fs::read(&target).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(info, expected_info);
        assert_eq!(actual, expected);
    }

    #[test]
    fn file_trim_rejects_same_source_and_target_without_truncating() {
        let input = clipline_fixture();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "clipline-trim-same-file-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.mp4");
        std::fs::write(&source, &input).unwrap();

        let err = trim_keyframe_aligned_file(&source, &source, 0.4, 1.2).unwrap_err();
        let after = std::fs::read(&source).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(matches!(
            err,
            TrimError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput
        ));
        assert_eq!(after, input);
    }
}
