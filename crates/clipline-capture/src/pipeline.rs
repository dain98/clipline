use std::io::{self, Seek, Write};

use clipline_buffer::{ReplayRing, SampleInfo, Segment, TrackSamples};
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
}

/// The recording pipeline (ddoc §3): capture → encode → GOP-aligned
/// segments → replay ring. Synchronous pull loop; production runs it on a
/// dedicated thread.
pub struct Recorder<C: CaptureEngine, E: Encoder> {
    capture: C,
    encoder: E,
    ring: ReplayRing,
    pending: Vec<EncodedPacket>,
    audio_sources: Vec<Box<dyn AudioSource>>,
    pending_audio: Vec<Vec<AudioPacket>>,
}

impl<C: CaptureEngine, E: Encoder> Recorder<C, E> {
    pub fn new(capture: C, encoder: E, max_buffer_bytes: usize) -> Self {
        Self {
            capture,
            encoder,
            ring: ReplayRing::new(max_buffer_bytes),
            pending: Vec::new(),
            audio_sources: Vec::new(),
            pending_audio: Vec::new(),
        }
    }

    /// Attach an audio source as the next audio track (ddoc §10:
    /// game / mic / system).
    pub fn with_audio(mut self, source: Box<dyn AudioSource>) -> Self {
        self.audio_sources.push(source);
        self.pending_audio.push(Vec::new());
        self
    }

    /// Drive the loop until the capture source ends, sealing a segment at
    /// every GOP boundary (a keyframe closes the previous GOP). Audio
    /// sources are drained per frame; packets ride in the segment whose
    /// GOP interval contains them.
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while let Some(frame) = self.capture.next_frame()? {
            for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
                pending.extend(src.poll_packets(frame.pts_s)?);
            }
            for pkt in self.encoder.encode(&frame)? {
                if pkt.is_keyframe && !self.pending.is_empty() {
                    self.seal_pending(pkt.pts_s);
                }
                self.pending.push(pkt);
            }
        }
        for pkt in self.encoder.finish()? {
            if pkt.is_keyframe && !self.pending.is_empty() {
                self.seal_pending(pkt.pts_s);
            }
            self.pending.push(pkt);
        }
        if !self.pending.is_empty() {
            // Drain any audio still buffered in the sources to the end of
            // the final GOP, then seal everything.
            let end = self.pending.last().map(|p| p.pts_s + p.duration_s).unwrap_or(0.0);
            for (src, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
                pending.extend(src.poll_packets(end)?);
            }
            self.seal_pending(f64::INFINITY);
        }
        Ok(())
    }

    pub fn ring(&self) -> &ReplayRing {
        &self.ring
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
        let segments = self.ring.save_window(window_s, exclude_before_s);
        if segments.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "no new footage in window"));
        }
        let video_cfg = self.encoder.track_config();
        let video_ts = video_cfg.timescale as f64;
        let audio_cfgs: Vec<_> = self.audio_sources.iter().map(|s| s.track_config()).collect();
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

    fn seal_pending(&mut self, boundary_pts_s: f64) {
        let packets = std::mem::take(&mut self.pending);
        let pts_start_s = packets[0].pts_s;
        let duration_s: f64 = packets.iter().map(|p| p.duration_s).sum();
        let starts_with_keyframe = packets[0].is_keyframe;
        let mut data = Vec::new();
        let mut samples = Vec::with_capacity(packets.len());
        for p in &packets {
            samples.push(SampleInfo {
                size: p.data.len() as u32,
                duration_s: p.duration_s,
                is_sync: p.is_keyframe,
            });
            data.extend_from_slice(&p.data);
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

        self.ring.push(Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
            audio,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockCapture, MockEncoder};

    #[test]
    fn groups_packets_into_gop_aligned_segments() {
        // 90 frames at 30 fps, GOP 30 → exactly 3 keyframe-led segments.
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring();
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
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            4 * 1024,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring();
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
        let ring = rec.ring();
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

    #[test]
    fn run_to_end_drains_encoder_via_finish() {
        let enc = OneFrameLatency { inner: MockEncoder::new(30, 30), held: None };
        let mut rec = Recorder::new(MockCapture::new(30, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        // All 30 frames present despite the encoder's one-frame latency.
        let total: usize = rec.ring().segments().map(|s| s.samples.len()).sum();
        assert_eq!(total, 30);
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
        let counts: Vec<usize> =
            rec.ring().segments().map(|s| s.samples.len()).collect();
        assert_eq!(counts, vec![30, 15]);
    }
}
