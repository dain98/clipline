use std::io::{self, Seek, SeekFrom, Write};

use crate::boxes::{full_box, mp4_box, Payload};
use crate::fragment::{fragment_multi, FragSample, TrackRun};
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

        let frag = fragment_multi(self.next_sequence, &runs);
        let frag_start = self.w.stream_position()?;
        let total_payload: usize = runs
            .iter()
            .flat_map(|r| r.samples.iter())
            .map(|s| s.data.len())
            .sum();
        let moof_len = frag.len() - (8 + total_payload);
        self.w.write_all(&frag)?;

        // Record chunk offsets in run order (the mdat layout order).
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
