use clipline_mp4::VideoTrackConfig;

use crate::traits::{
    CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder, Frame, FrameData,
};

/// Deterministic frame source: `total` frames at `fps`.
pub struct MockCapture {
    total: u64,
    fps: u32,
    produced: u64,
}

impl MockCapture {
    pub fn new(total: u64, fps: u32) -> Self {
        Self { total, fps, produced: 0 }
    }
}

impl CaptureEngine for MockCapture {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        if self.produced >= self.total {
            return Ok(None);
        }
        let pts_s = self.produced as f64 / self.fps as f64;
        self.produced += 1;
        Ok(Some(Frame { pts_s, data: FrameData::Cpu(vec![0u8; 16]) }))
    }
}

/// Deterministic "encoder": one packet per frame, keyframe every `gop_len`
/// frames, recognizable payload bytes for muxer round-trip assertions.
pub struct MockEncoder {
    gop_len: u64,
    fps: u32,
    count: u64,
}

impl MockEncoder {
    pub fn new(gop_len: u64, fps: u32) -> Self {
        Self { gop_len, fps, count: 0 }
    }
}

impl Encoder for MockEncoder {
    fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError> {
        let FrameData::Cpu(_) = &frame.data;
        let idx = self.count;
        self.count += 1;
        let mut data = format!("F{idx:06}").into_bytes();
        data.resize(64 + (idx % 7) as usize, 0xEE); // mildly varying sizes
        Ok(vec![EncodedPacket {
            data,
            pts_s: frame.pts_s,
            duration_s: 1.0 / self.fps as f64,
            is_keyframe: idx.is_multiple_of(self.gop_len),
        }])
    }

    fn track_config(&self) -> VideoTrackConfig {
        VideoTrackConfig {
            width: 128,
            height: 128,
            timescale: 90_000,
            sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            pps: vec![0x68, 0xEE, 0x38, 0x80],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{CaptureEngine, Encoder};

    #[test]
    fn mock_capture_produces_n_frames_then_ends() {
        let mut cap = MockCapture::new(3, 30);
        let f0 = cap.next_frame().unwrap().unwrap();
        assert_eq!(f0.pts_s, 0.0);
        let f1 = cap.next_frame().unwrap().unwrap();
        assert!((f1.pts_s - 1.0 / 30.0).abs() < 1e-9);
        cap.next_frame().unwrap().unwrap();
        assert!(cap.next_frame().unwrap().is_none(), "source ended");
    }

    #[test]
    fn mock_encoder_emits_keyframes_on_gop_boundaries() {
        let mut cap = MockCapture::new(5, 30);
        let mut enc = MockEncoder::new(2, 30); // GOP length 2
        let mut keys = Vec::new();
        while let Some(frame) = cap.next_frame().unwrap() {
            for pkt in enc.encode(&frame).unwrap() {
                keys.push(pkt.is_keyframe);
            }
        }
        assert_eq!(keys, vec![true, false, true, false, true]);
    }

    #[test]
    fn mock_encoder_packets_carry_pts_and_duration() {
        let mut enc = MockEncoder::new(30, 30);
        let frame = Frame { pts_s: 1.5, data: FrameData::Cpu(vec![0; 4]) };
        let pkts = enc.encode(&frame).unwrap();
        assert_eq!(pkts.len(), 1);
        assert_eq!(pkts[0].pts_s, 1.5);
        assert!((pkts[0].duration_s - 1.0 / 30.0).abs() < 1e-9);
        assert!(!pkts[0].data.is_empty());
    }

    #[test]
    fn mock_encoder_provides_a_muxable_track_config() {
        let enc = MockEncoder::new(30, 30);
        let cfg = enc.track_config();
        assert!(cfg.timescale > 0);
        assert!(!cfg.sps.is_empty());
        assert!(!cfg.pps.is_empty());
    }
}
