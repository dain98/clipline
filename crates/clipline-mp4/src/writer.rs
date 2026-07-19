use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::boxes::{full_box, mp4_box, Payload};
use crate::fragment::{
    fragment_moof_multi, fragment_multi, mdat_header, FragSample, FragSampleInfo, FragSampleRef,
    FragmentError, TrackRun, TrackRunInfo,
};
use crate::init::{
    audio_trak_with_tables, free_placeholder, ftyp, moov_init_multi, mvhd, video_trak_with_tables,
    TrackConfig, VideoTrackConfig, MOVIE_TIMESCALE,
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

#[derive(Debug, Clone, Copy)]
pub struct SourceSample {
    pub offset: u64,
    pub size: u32,
    pub duration: u32,
    pub is_sync: bool,
}

/// Seekable sample source used when one output contains media from more than
/// one backing file (for example source video plus a spooled mixed-audio track).
pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek + ?Sized> ReadSeek for T {}

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
                format!(
                    "expected {} track slices, got {}",
                    self.tracks.len(),
                    per_track.len()
                ),
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

        let frag = fragment_multi(self.next_sequence, &runs).map_err(fragment_io_error)?;
        let frag_start = self.w.stream_position()?;
        let total_payload =
            runs.iter()
                .flat_map(|r| r.samples.iter())
                .try_fold(0_usize, |total, sample| {
                    total.checked_add(sample.data.len()).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "fragment payload size overflow",
                        )
                    })
                })?;
        let mdat_header_len = mdat_header(total_payload as u64).len();
        let moof_len = frag
            .len()
            .checked_sub(mdat_header_len + total_payload)
            .expect("fragment layout includes its moof and mdat header");
        self.w.write_all(&frag)?;

        // Record chunk offsets in run order (the mdat layout order).
        let mut sample_offset = frag_start + moof_len as u64 + mdat_header_len as u64;
        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            let state = &mut self.tracks[idx];
            state.chunks.push((sample_offset, run.samples.len() as u32));
            for s in run.samples {
                state.sizes.push(
                    u32::try_from(s.data.len())
                        .map_err(|_| fragment_io_error(FragmentError::SampleSizeExceedsU32))?,
                );
                state.durations.push(s.duration);
                state.sync.push(s.is_sync);
                state.next_decode_time += s.duration as u64;
                sample_offset += s.data.len() as u64;
            }
        }
        self.next_sequence += 1;
        Ok(())
    }

    /// Borrowed variant for already-contiguous GOP buffers. This avoids
    /// allocating one `Vec<u8>` per sample before immediately writing it out.
    pub fn write_fragment_multi_borrowed(
        &mut self,
        per_track: &[&[FragSampleRef<'_>]],
    ) -> io::Result<()> {
        if per_track.len() != self.tracks.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "expected {} track slices, got {}",
                    self.tracks.len(),
                    per_track.len()
                ),
            ));
        }
        if per_track.iter().all(|s| s.is_empty()) {
            return Ok(());
        }

        let info_storage: Vec<Vec<FragSampleInfo>> = per_track
            .iter()
            .map(|samples| {
                samples
                    .iter()
                    .map(|s| {
                        Ok(FragSampleInfo {
                            size: u32::try_from(s.data.len()).map_err(|_| {
                                fragment_io_error(FragmentError::SampleSizeExceedsU32)
                            })?,
                            duration: s.duration,
                            is_sync: s.is_sync,
                        })
                    })
                    .collect::<io::Result<_>>()
            })
            .collect::<io::Result<_>>()?;
        let runs: Vec<TrackRunInfo<'_>> = info_storage
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_empty())
            .map(|(i, s)| TrackRunInfo {
                track_id: i as u32 + 1,
                base_decode_time: self.tracks[i].next_decode_time,
                samples: s,
            })
            .collect();

        let frag_start = self.w.stream_position()?;
        let total_payload = per_track
            .iter()
            .flat_map(|samples| samples.iter())
            .try_fold(0_u64, |total, sample| {
                total.checked_add(sample.data.len() as u64).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "fragment payload size overflow",
                    )
                })
            })?;
        let moof = fragment_moof_multi(self.next_sequence, &runs).map_err(fragment_io_error)?;
        self.w.write_all(&moof)?;
        let mdat_header = mdat_header(total_payload);
        self.w.write_all(&mdat_header)?;

        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            for sample in per_track[idx] {
                self.w.write_all(sample.data)?;
            }
        }

        let mut sample_offset = frag_start + moof.len() as u64 + mdat_header.len() as u64;
        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            let state = &mut self.tracks[idx];
            state.chunks.push((sample_offset, run.samples.len() as u32));
            for s in per_track[idx] {
                let size = s.data.len() as u32;
                state.sizes.push(size);
                state.durations.push(s.duration);
                state.sync.push(s.is_sync);
                state.next_decode_time += s.duration as u64;
                sample_offset += size as u64;
            }
        }
        self.next_sequence += 1;
        Ok(())
    }

    pub fn write_fragment_multi_from_source<R: Read + Seek>(
        &mut self,
        source: &mut R,
        per_track: &[&[SourceSample]],
    ) -> io::Result<()> {
        if per_track.len() != self.tracks.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "expected {} track slices, got {}",
                    self.tracks.len(),
                    per_track.len()
                ),
            ));
        }
        if per_track.iter().all(|s| s.is_empty()) {
            return Ok(());
        }

        let info_storage: Vec<Vec<FragSampleInfo>> = per_track
            .iter()
            .map(|samples| {
                samples
                    .iter()
                    .map(|s| FragSampleInfo {
                        size: s.size,
                        duration: s.duration,
                        is_sync: s.is_sync,
                    })
                    .collect()
            })
            .collect();
        let runs: Vec<TrackRunInfo<'_>> = info_storage
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.is_empty())
            .map(|(i, s)| TrackRunInfo {
                track_id: i as u32 + 1,
                base_decode_time: self.tracks[i].next_decode_time,
                samples: s,
            })
            .collect();

        let frag_start = self.w.stream_position()?;
        let total_payload = per_track
            .iter()
            .flat_map(|samples| samples.iter())
            .try_fold(0_u64, |total, sample| {
                total.checked_add(sample.size as u64).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "fragment payload size overflow",
                    )
                })
            })?;
        let moof = fragment_moof_multi(self.next_sequence, &runs).map_err(fragment_io_error)?;
        let mdat_header = mdat_header(total_payload);
        self.w.write_all(&moof)?;
        self.w.write_all(&mdat_header)?;

        let mut copy_buf = vec![0u8; 64 * 1024];
        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            for sample in per_track[idx] {
                source.seek(SeekFrom::Start(sample.offset))?;
                copy_exact(source, &mut self.w, sample.size as u64, &mut copy_buf)?;
            }
        }

        let mut sample_offset = frag_start + moof.len() as u64 + mdat_header.len() as u64;
        for run in &runs {
            let idx = (run.track_id - 1) as usize;
            let state = &mut self.tracks[idx];
            state.chunks.push((sample_offset, run.samples.len() as u32));
            for s in per_track[idx] {
                state.sizes.push(s.size);
                state.durations.push(s.duration);
                state.sync.push(s.is_sync);
                state.next_decode_time += s.duration as u64;
                sample_offset += s.size as u64;
            }
        }
        self.next_sequence += 1;
        Ok(())
    }

    pub fn write_fragment_multi_from_sources(
        &mut self,
        sources: &mut [&mut dyn ReadSeek],
        per_track: &[&[SourceSample]],
    ) -> io::Result<()> {
        if per_track.len() != self.tracks.len() || sources.len() != self.tracks.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "expected {} track sources and slices, got {} sources and {} slices",
                    self.tracks.len(),
                    sources.len(),
                    per_track.len()
                ),
            ));
        }
        if per_track.iter().all(|samples| samples.is_empty()) {
            return Ok(());
        }

        let info_storage: Vec<Vec<FragSampleInfo>> = per_track
            .iter()
            .map(|samples| {
                samples
                    .iter()
                    .map(|sample| FragSampleInfo {
                        size: sample.size,
                        duration: sample.duration,
                        is_sync: sample.is_sync,
                    })
                    .collect()
            })
            .collect();
        let runs: Vec<TrackRunInfo<'_>> = info_storage
            .iter()
            .enumerate()
            .filter(|(_, samples)| !samples.is_empty())
            .map(|(index, samples)| TrackRunInfo {
                track_id: index as u32 + 1,
                base_decode_time: self.tracks[index].next_decode_time,
                samples,
            })
            .collect();

        let frag_start = self.w.stream_position()?;
        let total_payload = per_track
            .iter()
            .flat_map(|samples| samples.iter())
            .try_fold(0_u64, |total, sample| {
                total.checked_add(u64::from(sample.size)).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "fragment payload size overflow",
                    )
                })
            })?;
        let moof = fragment_moof_multi(self.next_sequence, &runs).map_err(fragment_io_error)?;
        let mdat_header = mdat_header(total_payload);
        self.w.write_all(&moof)?;
        self.w.write_all(&mdat_header)?;

        let mut copy_buf = vec![0_u8; 64 * 1024];
        for run in &runs {
            let index = (run.track_id - 1) as usize;
            for sample in per_track[index] {
                sources[index].seek(SeekFrom::Start(sample.offset))?;
                copy_exact(
                    &mut sources[index],
                    &mut self.w,
                    u64::from(sample.size),
                    &mut copy_buf,
                )?;
            }
        }

        let mut sample_offset = frag_start + moof.len() as u64 + mdat_header.len() as u64;
        for run in &runs {
            let index = (run.track_id - 1) as usize;
            let state = &mut self.tracks[index];
            state.chunks.push((sample_offset, run.samples.len() as u32));
            for sample in per_track[index] {
                state.sizes.push(sample.size);
                state.durations.push(sample.duration);
                state.sync.push(sample.is_sync);
                state.next_decode_time += u64::from(sample.duration);
                sample_offset += u64::from(sample.size);
            }
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

fn copy_exact<R: Read, W: Write>(
    source: &mut R,
    dest: &mut W,
    mut remaining: u64,
    buf: &mut [u8],
) -> io::Result<()> {
    while remaining > 0 {
        let n = remaining.min(buf.len() as u64) as usize;
        source.read_exact(&mut buf[..n])?;
        dest.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

fn fragment_io_error(error: FragmentError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error)
}

fn rescale_duration(duration: u64, source_timescale: u32, target_timescale: u32) -> u64 {
    let scaled = duration as u128 * target_timescale as u128 / source_timescale as u128;
    scaled.min(u64::MAX as u128) as u64
}

impl TrackState {
    fn duration_media_ts(&self) -> u64 {
        self.durations.iter().map(|&d| d as u64).sum()
    }

    fn duration_movie_ts(&self) -> u64 {
        rescale_duration(
            self.duration_media_ts(),
            self.cfg.timescale(),
            MOVIE_TIMESCALE,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment::FragSample;
    use crate::init::{AudioTrackConfig, VideoTrackConfig};
    use crate::walker::{children, find, walk};
    use std::io::Cursor;

    fn video_cfg() -> VideoTrackConfig {
        VideoTrackConfig::h264(
            64,
            64,
            90_000,
            vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            vec![0x68, 0xEE, 0x38, 0x80],
        )
    }

    fn audio_cfg() -> AudioTrackConfig {
        AudioTrackConfig {
            channels: 2,
            sample_rate: 48_000,
            pre_skip: 312,
        }
    }

    fn gop(start: u32) -> Vec<FragSample> {
        (0..3)
            .map(|i| FragSample {
                data: format!("sample-{:04}", start + i).into_bytes(),
                duration: 3000,
                is_sync: i == 0,
            })
            .collect()
    }

    fn all_sync_gop() -> Vec<FragSample> {
        (0..4)
            .map(|_| FragSample {
                data: vec![0xAA; 10],
                duration: 960,
                is_sync: true,
            })
            .collect()
    }

    // --- TrackState helper tests ---

    fn make_track_state(cfg: TrackConfig, samples: &[(u32, u32, bool)]) -> TrackState {
        let mut state = TrackState {
            cfg,
            next_decode_time: 0,
            sizes: Vec::new(),
            durations: Vec::new(),
            sync: Vec::new(),
            chunks: Vec::new(),
        };
        for &(size, duration, is_sync) in samples {
            state.sizes.push(size);
            state.durations.push(duration);
            state.sync.push(is_sync);
            state.next_decode_time += duration as u64;
        }
        state
    }

    #[test]
    fn stts_run_length_encodes_equal_durations() {
        let state = make_track_state(
            TrackConfig::Video(video_cfg()),
            &[
                (100, 3000, true),
                (100, 3000, false),
                (100, 3000, false),
                (100, 6000, true),
                (100, 6000, false),
            ],
        );
        let stts = state.stts();
        // full_box header: 12 bytes; payload: entry_count(4) + 2 runs * 8
        let boxes = walk(&stts);
        assert_eq!(&boxes[0].fourcc, b"stts");
        let p = boxes[0].payload_offset as usize;
        let entry_count = u32::from_be_bytes(stts[p + 4..p + 8].try_into().unwrap());
        assert_eq!(entry_count, 2, "two distinct duration runs");
        // run 1: count=3, delta=3000
        assert_eq!(
            u32::from_be_bytes(stts[p + 8..p + 12].try_into().unwrap()),
            3
        );
        assert_eq!(
            u32::from_be_bytes(stts[p + 12..p + 16].try_into().unwrap()),
            3000
        );
        // run 2: count=2, delta=6000
        assert_eq!(
            u32::from_be_bytes(stts[p + 16..p + 20].try_into().unwrap()),
            2
        );
        assert_eq!(
            u32::from_be_bytes(stts[p + 20..p + 24].try_into().unwrap()),
            6000
        );
    }

    #[test]
    fn stss_none_when_all_sync() {
        let state = make_track_state(
            TrackConfig::Audio(audio_cfg()),
            &[(50, 960, true), (50, 960, true), (50, 960, true)],
        );
        assert!(state.stss().is_none(), "all-sync track omits stss per spec");
    }

    #[test]
    fn stss_lists_1_based_sync_sample_numbers() {
        let state = make_track_state(
            TrackConfig::Video(video_cfg()),
            &[
                (100, 3000, true),
                (80, 3000, false),
                (80, 3000, false),
                (100, 3000, true),
            ],
        );
        let stss = state.stss().expect("should have stss");
        let boxes = walk(&stss);
        let p = boxes[0].payload_offset as usize;
        let n = u32::from_be_bytes(stss[p + 4..p + 8].try_into().unwrap());
        assert_eq!(n, 2);
        assert_eq!(
            u32::from_be_bytes(stss[p + 8..p + 12].try_into().unwrap()),
            1,
            "first sync at sample 1"
        );
        assert_eq!(
            u32::from_be_bytes(stss[p + 12..p + 16].try_into().unwrap()),
            4,
            "second sync at sample 4"
        );
    }

    #[test]
    fn stsc_run_length_encodes_chunk_sizes() {
        let mut state = make_track_state(TrackConfig::Video(video_cfg()), &[]);
        // 3 chunks: first two have 3 samples, third has 2
        state.chunks = vec![(0, 3), (100, 3), (200, 2)];
        let stsc = state.stsc();
        let boxes = walk(&stsc);
        let p = boxes[0].payload_offset as usize;
        let entry_count = u32::from_be_bytes(stsc[p + 4..p + 8].try_into().unwrap());
        assert_eq!(entry_count, 2, "two distinct runs");
        // run 1: first_chunk=1, samples_per_chunk=3
        assert_eq!(
            u32::from_be_bytes(stsc[p + 8..p + 12].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_be_bytes(stsc[p + 12..p + 16].try_into().unwrap()),
            3
        );
        // run 2: first_chunk=3, samples_per_chunk=2
        assert_eq!(
            u32::from_be_bytes(stsc[p + 20..p + 24].try_into().unwrap()),
            3
        );
        assert_eq!(
            u32::from_be_bytes(stsc[p + 24..p + 28].try_into().unwrap()),
            2
        );
    }

    #[test]
    fn stsz_lists_every_sample_size() {
        let state = make_track_state(
            TrackConfig::Video(video_cfg()),
            &[(100, 3000, true), (80, 3000, false), (90, 3000, false)],
        );
        let stsz = state.stsz();
        let boxes = walk(&stsz);
        let p = boxes[0].payload_offset as usize;
        let sample_size = u32::from_be_bytes(stsz[p + 4..p + 8].try_into().unwrap());
        assert_eq!(sample_size, 0, "variable size mode");
        let count = u32::from_be_bytes(stsz[p + 8..p + 12].try_into().unwrap());
        assert_eq!(count, 3);
        assert_eq!(
            u32::from_be_bytes(stsz[p + 12..p + 16].try_into().unwrap()),
            100
        );
        assert_eq!(
            u32::from_be_bytes(stsz[p + 16..p + 20].try_into().unwrap()),
            80
        );
        assert_eq!(
            u32::from_be_bytes(stsz[p + 20..p + 24].try_into().unwrap()),
            90
        );
    }

    #[test]
    fn co64_lists_chunk_offsets() {
        let mut state = make_track_state(TrackConfig::Video(video_cfg()), &[]);
        state.chunks = vec![(1000, 3), (5000, 2)];
        let co64 = state.co64();
        let boxes = walk(&co64);
        let p = boxes[0].payload_offset as usize;
        let count = u32::from_be_bytes(co64[p + 4..p + 8].try_into().unwrap());
        assert_eq!(count, 2);
        assert_eq!(
            u64::from_be_bytes(co64[p + 8..p + 16].try_into().unwrap()),
            1000
        );
        assert_eq!(
            u64::from_be_bytes(co64[p + 16..p + 24].try_into().unwrap()),
            5000
        );
    }

    #[test]
    fn duration_media_ts_sums_all_durations() {
        let state = make_track_state(
            TrackConfig::Video(video_cfg()),
            &[(100, 3000, true), (80, 3000, false), (90, 3000, false)],
        );
        assert_eq!(state.duration_media_ts(), 9000);
    }

    #[test]
    fn duration_movie_ts_scales_to_movie_timescale() {
        let state = make_track_state(
            TrackConfig::Video(video_cfg()),
            // 90_000 ticks at media timescale 90_000 = 1 second = 1000 movie ts
            &[(100, 90_000, true)],
        );
        assert_eq!(state.duration_movie_ts(), 1000);
    }

    #[test]
    fn duration_rescale_uses_wide_intermediate() {
        let duration = u64::MAX;
        let expected = ((duration as u128 * MOVIE_TIMESCALE as u128) / 90_000u128) as u64;
        assert_eq!(
            rescale_duration(duration, 90_000, MOVIE_TIMESCALE),
            expected
        );
    }

    #[test]
    fn duration_rescale_saturates_only_final_overflow() {
        assert_eq!(rescale_duration(u64::MAX, 1, u32::MAX), u64::MAX);
        assert_eq!(
            rescale_duration(u64::MAX, u32::MAX, 1),
            (u64::MAX as u128 / u32::MAX as u128) as u64
        );
    }

    // --- HybridMp4Writer API tests ---

    #[test]
    fn write_fragment_multi_rejects_track_count_mismatch() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        let s1 = gop(0);
        let s2 = gop(3);
        let err = w.write_fragment_multi(&[&s1, &s2]);
        assert!(err.is_err(), "2 slices for a 1-track writer");
    }

    #[test]
    fn write_fragment_multi_no_ops_on_all_empty() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        let before = w.w.position();
        w.write_fragment_multi(&[&[]]).unwrap();
        assert_eq!(w.w.position(), before, "nothing written for empty samples");
    }

    #[test]
    fn write_fragment_multi_borrowed_rejects_track_count_mismatch() {
        let mut w = HybridMp4Writer::new_multi(
            Cursor::new(Vec::new()),
            vec![
                TrackConfig::Video(video_cfg()),
                TrackConfig::Audio(audio_cfg()),
            ],
        )
        .unwrap();
        let samples = gop(0);
        let refs: Vec<FragSampleRef<'_>> = samples
            .iter()
            .map(|s| FragSampleRef {
                data: &s.data,
                duration: s.duration,
                is_sync: s.is_sync,
            })
            .collect();
        let err = w.write_fragment_multi_borrowed(&[&refs]);
        assert!(err.is_err(), "1 slice for a 2-track writer");
    }

    #[test]
    fn write_fragment_multi_borrowed_no_ops_on_all_empty() {
        let mut w = HybridMp4Writer::new_multi(
            Cursor::new(Vec::new()),
            vec![
                TrackConfig::Video(video_cfg()),
                TrackConfig::Audio(audio_cfg()),
            ],
        )
        .unwrap();
        let before = w.w.position();
        let empty_v: &[FragSampleRef<'_>] = &[];
        let empty_a: &[FragSampleRef<'_>] = &[];
        w.write_fragment_multi_borrowed(&[empty_v, empty_a])
            .unwrap();
        assert_eq!(w.w.position(), before);
    }

    #[test]
    fn write_fragment_multi_from_source_rejects_track_count_mismatch() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        let source_data = vec![0u8; 100];
        let mut source = Cursor::new(source_data);
        let samples = [SourceSample {
            offset: 0,
            size: 10,
            duration: 3000,
            is_sync: true,
        }];
        let err = w.write_fragment_multi_from_source(&mut source, &[&samples, &samples]);
        assert!(err.is_err());
    }

    #[test]
    fn write_fragment_multi_from_source_no_ops_on_all_empty() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        let mut source = Cursor::new(Vec::<u8>::new());
        let before = w.w.position();
        let empty: &[SourceSample] = &[];
        w.write_fragment_multi_from_source(&mut source, &[empty])
            .unwrap();
        assert_eq!(w.w.position(), before);
    }

    #[test]
    fn finalize_produces_ftyp_mdat_moov_layout() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        w.write_fragment(&gop(0)).unwrap();
        let buf = w.finalize().unwrap().into_inner();
        let boxes = walk(&buf);
        let fourccs: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.fourcc).collect();
        assert_eq!(fourccs, vec![b"ftyp", b"mdat", b"moov"]);
    }

    #[test]
    fn finalize_multi_track_moov_has_one_trak_per_track() {
        let mut w = HybridMp4Writer::new_multi(
            Cursor::new(Vec::new()),
            vec![
                TrackConfig::Video(video_cfg()),
                TrackConfig::Audio(audio_cfg()),
            ],
        )
        .unwrap();
        let video = gop(0);
        let audio = all_sync_gop();
        w.write_fragment_multi(&[&video, &audio]).unwrap();
        let buf = w.finalize().unwrap().into_inner();

        let boxes = walk(&buf);
        let moov = find(&boxes, b"moov").unwrap();
        let kids = children(&buf, moov);
        let traks: Vec<_> = kids.iter().filter(|b| &b.fourcc == b"trak").collect();
        assert_eq!(traks.len(), 2, "one trak per track");
    }

    #[test]
    fn copy_exact_copies_all_bytes() {
        let src = b"hello world";
        let mut dst = Vec::new();
        let mut buf = [0u8; 4]; // small buffer forces multiple iterations
        copy_exact(&mut &src[..], &mut dst, src.len() as u64, &mut buf).unwrap();
        assert_eq!(dst, src);
    }

    #[test]
    fn sequence_number_increments_per_fragment() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        assert_eq!(w.next_sequence, 1);
        w.write_fragment(&gop(0)).unwrap();
        assert_eq!(w.next_sequence, 2);
        w.write_fragment(&gop(3)).unwrap();
        assert_eq!(w.next_sequence, 3);
    }

    #[test]
    fn next_decode_time_advances_by_sample_durations() {
        let mut w = HybridMp4Writer::new(Cursor::new(Vec::new()), video_cfg()).unwrap();
        w.write_fragment(&gop(0)).unwrap();
        // 3 samples × 3000 duration each
        assert_eq!(w.tracks[0].next_decode_time, 9000);
        w.write_fragment(&gop(3)).unwrap();
        assert_eq!(w.tracks[0].next_decode_time, 18000);
    }
}
