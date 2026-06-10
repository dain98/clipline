use std::io::{self, Seek, SeekFrom, Write};

use crate::boxes::{full_box, mp4_box, Payload};
use crate::fragment::{fragment, FragSample};
use crate::init::{
    free_placeholder, ftyp, moov_init, mvhd, trak_with_tables, VideoTrackConfig,
    MOVIE_TIMESCALE,
};

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
