use std::io::{self, Seek, Write};
use std::path::PathBuf;

use clipline_buffer::{DiskReplayRing, ReplayRing, SampleInfo, Segment, TrackSamples};
use clipline_mp4::{FragSample, HybridMp4Writer, TrackConfig};

use crate::traits::{
    AudioPacket, AudioSource, CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder,
};

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub enum ReplayStorageConfig {
    Memory { max_bytes: usize },
    Disk { max_bytes: usize, dir: PathBuf },
}

enum ReplayStorage {
    Memory(ReplayRing),
    Disk(DiskReplayRing),
}

/// The recording pipeline (ddoc §3): capture → encode → GOP-aligned
/// segments → replay ring. Synchronous pull loop; production runs it on a
/// dedicated thread.
pub struct Recorder<C: CaptureEngine, E: Encoder> {
    capture: C,
    encoder: E,
    ring: ReplayStorage,
    pending: Vec<EncodedPacket>,
    audio_sources: Vec<Box<dyn AudioSource>>,
    pending_audio: Vec<Vec<AudioPacket>>,
    /// pts of the first video packet — the recording's timeline start.
    /// Audio captured before it (engine-init lead-in) is dropped so both
    /// tracks begin together in the file.
    video_start_pts_s: Option<f64>,
}

impl<C: CaptureEngine, E: Encoder> Recorder<C, E> {
    pub fn new(capture: C, encoder: E, max_buffer_bytes: usize) -> Self {
        Self {
            capture,
            encoder,
            ring: ReplayStorage::Memory(ReplayRing::new(max_buffer_bytes)),
            pending: Vec::new(),
            audio_sources: Vec::new(),
            pending_audio: Vec::new(),
            video_start_pts_s: None,
        }
    }

    pub fn new_with_replay_storage(
        capture: C,
        encoder: E,
        storage: ReplayStorageConfig,
    ) -> io::Result<Self> {
        let ring = match storage {
            ReplayStorageConfig::Memory { max_bytes } => {
                ReplayStorage::Memory(ReplayRing::new(max_bytes))
            }
            ReplayStorageConfig::Disk { max_bytes, dir } => {
                ReplayStorage::Disk(DiskReplayRing::new(max_bytes, dir)?)
            }
        };
        Ok(Self {
            capture,
            encoder,
            ring,
            pending: Vec::new(),
            audio_sources: Vec::new(),
            pending_audio: Vec::new(),
            video_start_pts_s: None,
        })
    }

    /// Attach an audio source as the next audio track (ddoc §10:
    /// game / mic / system).
    pub fn with_audio(mut self, source: Box<dyn AudioSource>) -> Self {
        self.audio_sources.push(source);
        self.pending_audio.push(Vec::new());
        self
    }

    /// Process one captured frame (audio drain → encode → GOP sealing).
    /// `Ok(false)` = the capture source ended. Errors pass through —
    /// callers running live decide how to treat `CaptureError::Timeout`
    /// (an idle screen delivers no frames; that is not fatal).
    pub fn step(&mut self) -> Result<bool, PipelineError> {
        let Some(frame) = self.capture.next_frame()? else {
            return Ok(false);
        };
        for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
            pending.extend(src.poll_packets(frame.pts_s)?);
        }
        for pkt in self.encoder.encode(&frame)? {
            if self.video_start_pts_s.is_none() {
                self.video_start_pts_s = Some(pkt.pts_s);
            }
            if pkt.is_keyframe && !self.pending.is_empty() {
                self.seal_pending(pkt.pts_s)?;
            }
            self.pending.push(pkt);
        }
        Ok(true)
    }

    /// End of stream: drain the encoder, drain audio to the final GOP's
    /// end, seal the trailing partial GOP.
    pub fn finish_stream(&mut self) -> Result<(), PipelineError> {
        for pkt in self.encoder.finish()? {
            if pkt.is_keyframe && !self.pending.is_empty() {
                self.seal_pending(pkt.pts_s)?;
            }
            self.pending.push(pkt);
        }
        if !self.pending.is_empty() {
            let end = self
                .pending
                .last()
                .map(|p| p.pts_s + p.duration_s)
                .unwrap_or(0.0);
            for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
                pending.extend(src.poll_packets(end)?);
            }
            self.seal_pending(f64::INFINITY)?;
        }
        Ok(())
    }

    /// Drive the loop until the capture source ends, sealing a segment at
    /// every GOP boundary (a keyframe closes the previous GOP). Audio
    /// sources are drained per frame; packets ride in the segment whose
    /// GOP interval contains them.
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while self.step()? {}
        self.finish_stream()
    }

    pub fn ring(&self) -> Option<&ReplayRing> {
        match &self.ring {
            ReplayStorage::Memory(ring) => Some(ring),
            ReplayStorage::Disk(_) => None,
        }
    }

    pub fn ring_len(&self) -> usize {
        match &self.ring {
            ReplayStorage::Memory(ring) => ring.len(),
            ReplayStorage::Disk(ring) => ring.len(),
        }
    }

    pub fn ring_bytes(&self) -> usize {
        match &self.ring {
            ReplayStorage::Memory(ring) => ring.bytes(),
            ReplayStorage::Disk(ring) => ring.bytes(),
        }
    }

    pub fn buffered_span_s(&self) -> f64 {
        let mut span = 0.0f64;
        let mut first_pts = None::<f64>;
        match &self.ring {
            ReplayStorage::Memory(ring) => {
                for seg in ring.segments() {
                    first_pts.get_or_insert(seg.pts_start_s);
                    span = seg.pts_end_s() - first_pts.unwrap();
                }
            }
            ReplayStorage::Disk(ring) => {
                for seg in ring.segments() {
                    first_pts.get_or_insert(seg.pts_start_s);
                    span = seg.pts_end_s() - first_pts.unwrap();
                }
            }
        }
        span
    }

    pub fn save_window_bounds(
        &self,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> Option<(f64, f64)> {
        match &self.ring {
            ReplayStorage::Memory(ring) => {
                let segs = ring.save_window(window_s, exclude_before_s);
                Some((segs.first()?.pts_start_s, segs.last()?.pts_end_s()))
            }
            ReplayStorage::Disk(ring) => {
                let segs = ring.save_window(window_s, exclude_before_s);
                Some((segs.first()?.pts_start_s, segs.last()?.pts_end_s()))
            }
        }
    }

    pub fn encoder(&self) -> &E {
        &self.encoder
    }

    /// Save the trailing `window_s` seconds as a finalized Hybrid MP4
    /// written to `w` (ddoc §6). `exclude_before_s` is the smart
    /// no-overlap mode. Returns the writer and the end pts of the saved
    /// footage — pass it back as `exclude_before_s` next time.
    ///
    /// Erroring (rather than writing an empty file) when no new footage
    /// exists lets the hotkey handler tell the user "nothing new to save".
    pub fn save_replay<W: Write + Seek>(
        &self,
        w: W,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> io::Result<(W, f64)> {
        let segments = self.save_window_segments(window_s, exclude_before_s)?;
        if segments.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no new footage in window",
            ));
        }
        let video_cfg = self.encoder.track_config();
        let video_ts = video_cfg.timescale as f64;
        let audio_cfgs: Vec<_> = self
            .audio_sources
            .iter()
            .map(|s| s.track_config())
            .collect();
        let mut track_cfgs = vec![TrackConfig::Video(video_cfg)];
        for cfg in &audio_cfgs {
            track_cfgs.push(TrackConfig::Audio(cfg.clone()));
        }
        let mut writer = HybridMp4Writer::new_multi(w, track_cfgs)?;
        for seg in &segments {
            let video: Vec<FragSample> = seg
                .sample_slices()
                .zip(&seg.samples)
                .map(|(slice, info)| FragSample {
                    data: slice.to_vec(),
                    duration: (info.duration_s * video_ts).round() as u32,
                    is_sync: info.is_sync,
                })
                .collect();
            let mut per_track: Vec<Vec<FragSample>> = vec![video];
            for (track, cfg) in seg.audio.iter().zip(&audio_cfgs) {
                let ts = cfg.sample_rate as f64;
                per_track.push(
                    track
                        .sample_slices()
                        .zip(&track.samples)
                        .map(|(slice, info)| FragSample {
                            data: slice.to_vec(),
                            duration: (info.duration_s * ts).round() as u32,
                            is_sync: info.is_sync,
                        })
                        .collect(),
                );
            }
            // Segments recorded before an audio source was attached have
            // fewer audio tracks; pad with empty runs to keep alignment.
            per_track.resize_with(1 + audio_cfgs.len(), Vec::new);
            let slices: Vec<&[FragSample]> = per_track.iter().map(|v| v.as_slice()).collect();
            writer.write_fragment_multi(&slices)?;
        }
        let end_pts = segments.last().expect("non-empty").pts_end_s();
        Ok((writer.finalize()?, end_pts))
    }

    fn save_window_segments(
        &self,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> io::Result<Vec<Segment>> {
        match &self.ring {
            ReplayStorage::Memory(ring) => Ok(ring
                .save_window(window_s, exclude_before_s)
                .into_iter()
                .cloned()
                .collect()),
            ReplayStorage::Disk(ring) => ring
                .save_window(window_s, exclude_before_s)
                .into_iter()
                .map(|seg| seg.load())
                .collect(),
        }
    }

    fn seal_pending(&mut self, boundary_pts_s: f64) -> Result<(), PipelineError> {
        let packets = std::mem::take(&mut self.pending);
        let pts_start_s = packets[0].pts_s;
        let starts_with_keyframe = packets[0].is_keyframe;
        // ddoc §6: the timeline follows capture stamps, not encoder cadence
        // claims. Each sample lasts until the next pts; the sealing
        // keyframe's pts closes the GOP exactly; only the final seal
        // (boundary = ∞) trusts the encoder's own duration.
        let durations: Vec<f64> = (0..packets.len())
            .map(|i| {
                let next_pts = packets
                    .get(i + 1)
                    .map(|p| p.pts_s)
                    .unwrap_or(boundary_pts_s);
                if next_pts.is_finite() {
                    (next_pts - packets[i].pts_s).max(1e-4)
                } else {
                    packets[i].duration_s
                }
            })
            .collect();
        let duration_s: f64 = durations.iter().sum();
        let mut data = Vec::new();
        let mut samples = Vec::with_capacity(packets.len());
        for (p, &d) in packets.iter().zip(&durations) {
            samples.push(SampleInfo {
                size: p.data.len() as u32,
                duration_s: d,
                is_sync: p.is_keyframe,
            });
            data.extend_from_slice(&p.data);
        }

        // Audio captured before the first video packet is engine-init
        // lead-in: drop it, or video plays early by that offset.
        let timeline_start = self.video_start_pts_s.unwrap_or(pts_start_s);
        for pending in &mut self.pending_audio {
            pending.retain(|p| p.pts_s + p.duration_s > timeline_start + 1e-9);
        }
        // Audio packets ending at or before the boundary belong to this GOP.
        let mut audio = Vec::with_capacity(self.pending_audio.len());
        for pending in &mut self.pending_audio {
            let split = pending
                .iter()
                .position(|p| p.pts_s + p.duration_s > boundary_pts_s + 1e-9)
                .unwrap_or(pending.len());
            let mut track = TrackSamples::default();
            for p in pending.drain(..split) {
                track.samples.push(SampleInfo {
                    size: p.data.len() as u32,
                    duration_s: p.duration_s,
                    is_sync: true, // every Opus packet is independently decodable
                });
                track.data.extend_from_slice(&p.data);
            }
            audio.push(track);
        }

        let seg = Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
            audio,
        };
        match &mut self.ring {
            ReplayStorage::Memory(ring) => ring.push(seg),
            ReplayStorage::Disk(ring) => ring.push(seg)?,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockCapture, MockEncoder};
    use std::path::PathBuf;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-pipeline-{name}-{}-{unique}",
                std::process::id()
            ));
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn groups_packets_into_gop_aligned_segments() {
        // 90 frames at 30 fps, GOP 30 → exactly 3 keyframe-led segments.
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 3);
        for seg in ring.segments() {
            assert!(seg.starts_with_keyframe);
            assert_eq!(seg.samples.len(), 30);
            assert!((seg.duration_s - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn byte_budget_evicts_oldest_gop() {
        // Each MockEncoder sample is 64–70 bytes → a GOP of 30 ≈ ~2 KB.
        // Budget for ~2 GOPs: the first of three must be evicted.
        let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), 4 * 1024);
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 2);
        let first = ring.segments().next().unwrap();
        assert!((first.pts_start_s - 1.0).abs() < 1e-6, "GOP at t=0 evicted");
    }

    #[test]
    fn audio_packets_land_in_their_gop_segments() {
        use crate::mock::MockAudioSource;
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 3);
        for (i, seg) in ring.segments().enumerate() {
            assert_eq!(seg.audio.len(), 1, "one audio track");
            // 1 s GOP at 20 ms packets = 50 packets per segment.
            assert_eq!(seg.audio[0].samples.len(), 50, "segment {i}");
        }
        // First packet of the second segment starts at its GOP boundary.
        let seg2 = ring.segments().nth(1).unwrap();
        assert_eq!(&seg2.audio[0].data[..6], b"P00050");
    }

    #[test]
    fn save_replay_preserves_multiple_audio_tracks() {
        use crate::mock::MockAudioSource;
        use clipline_mp4::walker::{children, find, walk};

        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)))
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));

        rec.run_to_end().unwrap();
        for seg in rec.ring().unwrap().segments() {
            assert_eq!(seg.audio.len(), 2, "system plus microphone tracks");
        }

        let (buf, _) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, e)| (w.into_inner(), e))
            .expect("multi-audio save");
        let boxes = walk(&buf);
        let moov = find(&boxes, b"moov").expect("moov");
        let kids = children(&buf, moov);
        let traks = kids.iter().filter(|b| &b.fourcc == b"trak").count();
        assert_eq!(traks, 3, "video plus two audio tracks");
    }

    #[test]
    fn disk_replay_storage_saves_same_bytes_as_memory_storage() {
        let mut ram = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        ram.run_to_end().unwrap();
        let (ram_buf, _) = ram
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, end)| (w.into_inner(), end))
            .unwrap();

        let dir = TestDir::new("disk-equivalence");
        let mut disk = Recorder::new_with_replay_storage(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            ReplayStorageConfig::Disk {
                max_bytes: usize::MAX,
                dir: dir.0.clone(),
            },
        )
        .unwrap();
        disk.run_to_end().unwrap();
        let (disk_buf, _) = disk
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, end)| (w.into_inner(), end))
            .unwrap();

        assert_eq!(disk.ring_len(), ram.ring_len());
        assert_eq!(disk.ring_bytes(), ram.ring_bytes());
        assert_eq!(disk_buf, ram_buf);
    }

    /// Wraps MockEncoder but holds back the latest packet until finish() —
    /// models real encoders' internal buffering.
    struct OneFrameLatency {
        inner: MockEncoder,
        held: Option<crate::traits::EncodedPacket>,
    }

    impl Encoder for OneFrameLatency {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut out = self.inner.encode(frame)?;
            let newly = out.pop();
            let released = self.held.take();
            self.held = newly;
            Ok(released.into_iter().collect())
        }

        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }

        fn finish(
            &mut self,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            Ok(self.held.take().into_iter().collect())
        }
    }

    /// Encoder echoing nominal durations while pts jitters (VRR-style):
    /// the sealed timeline must follow the STAMPS (ddoc §6), not the echo.
    struct JitteryEncoder {
        inner: MockEncoder,
    }

    impl Encoder for JitteryEncoder {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut pkts = self.inner.encode(frame)?;
            for p in &mut pkts {
                // Stamps: frames alternate 10 ms / 30 ms apart, while the
                // encoder still claims a flat 1/30 s duration.
                let idx = (p.pts_s * 30.0).round();
                p.pts_s = (idx / 2.0).floor() * 0.04 + if idx % 2.0 == 1.0 { 0.01 } else { 0.0 };
            }
            Ok(pkts)
        }
        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }
    }

    #[test]
    fn sealed_durations_come_from_pts_deltas_not_encoder_claims() {
        // GOP of 4 over 8 frames → two segments, boundary at frame 4.
        let enc = JitteryEncoder {
            inner: MockEncoder::new(4, 30),
        };
        let mut rec = Recorder::new(MockCapture::new(8, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        let segs: Vec<_> = rec.ring().unwrap().segments().collect();
        assert_eq!(segs.len(), 2);
        // Within a GOP: 10/30/10 ms gaps, NOT the encoder's flat 33.3 ms.
        let d: Vec<f64> = segs[0].samples.iter().map(|s| s.duration_s).collect();
        assert!((d[0] - 0.01).abs() < 1e-9, "got {d:?}");
        assert!((d[1] - 0.03).abs() < 1e-9, "got {d:?}");
        assert!((d[2] - 0.01).abs() < 1e-9, "got {d:?}");
        // Boundary: last sample of GOP 1 closes exactly at GOP 2's keyframe.
        let gop2_start = segs[1].pts_start_s;
        assert!(
            (segs[0].pts_end_s() - gop2_start).abs() < 1e-9,
            "no gap, no overlap"
        );
        // Final seal falls back to the encoder duration for the last sample.
        let last = segs[1].samples.last().unwrap();
        assert!((last.duration_s - 1.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn run_to_end_drains_encoder_via_finish() {
        let enc = OneFrameLatency {
            inner: MockEncoder::new(30, 30),
            held: None,
        };
        let mut rec = Recorder::new(MockCapture::new(30, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        // All 30 frames present despite the encoder's one-frame latency.
        let total: usize = rec
            .ring()
            .unwrap()
            .segments()
            .map(|s| s.samples.len())
            .sum();
        assert_eq!(total, 30);
    }

    #[test]
    fn save_replay_works_between_steps_while_recording() {
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        // Two GOPs in: a save must succeed without ending the recording.
        for _ in 0..60 {
            assert!(rec.step().unwrap());
        }
        let (buf, end) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, e)| (w.into_inner(), e))
            .expect("mid-recording save");
        assert!(!buf.is_empty());
        assert!(
            (end - 1.0).abs() < 1e-6,
            "one sealed GOP at save time (second pending)"
        );
        // Recording continues; smart mode skips the already-saved second.
        for _ in 0..30 {
            assert!(rec.step().unwrap());
        }
        assert!(!rec.step().unwrap(), "source exhausted");
        rec.finish_stream().unwrap();
        let (_, end2) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, Some(end))
            .expect("post-finish save");
        assert!((end2 - 3.0).abs() < 1e-6, "everything sealed after finish");
        // run_to_end equivalence: same segment layout as the stepped path.
        let mut whole = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        whole.run_to_end().unwrap();
        assert_eq!(whole.ring().unwrap().len(), rec.ring().unwrap().len());
    }

    /// Shifts a capture source's pts later — models the real lead-in
    /// between clock creation and the first WGC frame.
    struct OffsetCapture {
        inner: MockCapture,
        offset_s: f64,
    }

    impl crate::traits::CaptureEngine for OffsetCapture {
        fn next_frame(
            &mut self,
        ) -> Result<Option<crate::traits::Frame>, crate::traits::CaptureError> {
            Ok(self.inner.next_frame()?.map(|mut f| {
                f.pts_s += self.offset_s;
                f
            }))
        }
    }

    #[test]
    fn audio_lead_in_before_first_video_frame_is_dropped() {
        // Video starts 0.5 s after the shared clock origin; audio has been
        // capturing (and silence-filling) since t=0. The pre-video audio
        // must not ride in the file or video plays early by the lead-in.
        use crate::mock::MockAudioSource;
        let cap = OffsetCapture {
            inner: MockCapture::new(60, 30),
            offset_s: 0.5,
        };
        let mut rec = Recorder::new(cap, MockEncoder::new(30, 30), usize::MAX)
            .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        let segs: Vec<_> = rec.ring().unwrap().segments().collect();
        assert_eq!(segs.len(), 2);
        // First segment: audio coverage matches video duration within one
        // 20 ms packet (the packet straddling the boundary is dropped).
        let covered: f64 = segs[0].audio[0].samples.iter().map(|s| s.duration_s).sum();
        assert!(
            (covered - segs[0].duration_s).abs() <= 0.02 + 1e-9,
            "lead-in dropped: covered {covered}, video {}",
            segs[0].duration_s
        );
        // And the first kept packet starts at/after the video start.
        // (MockAudioSource stamps pts; we can't read them back from the
        // sealed track, but coverage bounds above imply it.)
    }

    #[test]
    fn trailing_partial_gop_is_sealed_at_end() {
        // 45 frames, GOP 30 → one full GOP + one 15-frame partial.
        let mut rec = Recorder::new(
            MockCapture::new(45, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let counts: Vec<usize> = rec
            .ring()
            .unwrap()
            .segments()
            .map(|s| s.samples.len())
            .collect();
        assert_eq!(counts, vec![30, 15]);
    }
}
