use std::io::{self, Seek, Write};

use clipline_buffer::{ReplayRing, SampleInfo, Segment};
use clipline_mp4::{FragSample, HybridMp4Writer};

use crate::traits::{CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder};

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
}

impl<C: CaptureEngine, E: Encoder> Recorder<C, E> {
    pub fn new(capture: C, encoder: E, max_buffer_bytes: usize) -> Self {
        Self {
            capture,
            encoder,
            ring: ReplayRing::new(max_buffer_bytes),
            pending: Vec::new(),
        }
    }

    /// Drive the loop until the capture source ends, sealing a segment at
    /// every GOP boundary (a keyframe closes the previous GOP).
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while let Some(frame) = self.capture.next_frame()? {
            for pkt in self.encoder.encode(&frame)? {
                if pkt.is_keyframe && !self.pending.is_empty() {
                    self.seal_pending();
                }
                self.pending.push(pkt);
            }
        }
        if !self.pending.is_empty() {
            self.seal_pending();
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
        let cfg = self.encoder.track_config();
        let timescale = cfg.timescale as f64;
        let mut writer = HybridMp4Writer::new(w, cfg)?;
        for seg in &segments {
            let samples: Vec<FragSample> = seg
                .sample_slices()
                .zip(&seg.samples)
                .map(|(slice, info)| FragSample {
                    data: slice.to_vec(),
                    duration: (info.duration_s * timescale).round() as u32,
                    is_sync: info.is_sync,
                })
                .collect();
            writer.write_fragment(&samples)?;
        }
        let end_pts = segments.last().expect("non-empty").pts_end_s();
        Ok((writer.finalize()?, end_pts))
    }

    fn seal_pending(&mut self) {
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
        self.ring.push(Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
            audio: Vec::new(),
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
