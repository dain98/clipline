use clipline_mp4::{AudioTrackConfig, VideoTrackConfig};

use crate::traits::{
    AudioPacket, AudioSource, CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder,
    Frame, FrameData,
};

/// Deterministic frame source: `total` frames at `fps`.
pub struct MockCapture {
    total: u64,
    fps: u32,
    produced: u64,
}

impl MockCapture {
    pub fn new(total: u64, fps: u32) -> Self {
        Self {
            total,
            fps,
            produced: 0,
        }
    }
}

impl CaptureEngine for MockCapture {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        if self.produced >= self.total {
            return Ok(None);
        }
        let pts_s = self.produced as f64 / self.fps as f64;
        self.produced += 1;
        Ok(Some(Frame {
            pts_s,
            data: FrameData::Cpu(vec![0u8; 16]),
        }))
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
        Self {
            gop_len,
            fps,
            count: 0,
        }
    }
}

impl Encoder for MockEncoder {
    fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError> {
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
        VideoTrackConfig::h264(
            128,
            128,
            90_000,
            vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            vec![0x68, 0xEE, 0x38, 0x80],
        )
    }
}

/// Bounds an endless capture source to N frames so
/// `Recorder::run_to_end` terminates (WGC never ends on its own).
pub struct LimitedCapture<C: CaptureEngine> {
    inner: C,
    remaining: u64,
}

impl<C: CaptureEngine> LimitedCapture<C> {
    pub fn new(inner: C, frames: u64) -> Self {
        Self {
            inner,
            remaining: frames,
        }
    }
}

impl<C: CaptureEngine> CaptureEngine for LimitedCapture<C> {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let frame = self.inner.next_frame()?;
        if frame.is_some() {
            self.remaining -= 1;
        }
        Ok(frame)
    }
}

/// Deterministic audio source: fixed-size packets every `packet_ms`.
pub struct MockAudioSource {
    sample_rate: u32,
    packet_ms: u32,
    next_index: u64,
}

impl MockAudioSource {
    pub fn new(sample_rate: u32, packet_ms: u32) -> Self {
        Self {
            sample_rate,
            packet_ms,
            next_index: 0,
        }
    }
}

impl AudioSource for MockAudioSource {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        let dur = self.packet_ms as f64 / 1000.0;
        let mut out = Vec::new();
        loop {
            let pts = self.next_index as f64 * dur;
            if pts + dur > until_pts_s + 1e-9 {
                break;
            }
            let mut data = format!("P{:05}", self.next_index).into_bytes();
            data.resize(40, 0xAA);
            out.push(AudioPacket {
                data,
                pts_s: pts,
                duration_s: dur,
            });
            self.next_index += 1;
        }
        Ok(out)
    }

    fn track_config(&self) -> AudioTrackConfig {
        AudioTrackConfig {
            channels: 2,
            sample_rate: self.sample_rate,
            pre_skip: 312,
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
        let frame = Frame {
            pts_s: 1.5,
            data: FrameData::Cpu(vec![0; 4]),
        };
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
        match &cfg.codec {
            clipline_mp4::VideoCodecParams::H264 { sps, pps } => {
                assert!(!sps.is_empty() && !pps.is_empty());
            }
            other => panic!("mock encoder is H.264, got {other:?}"),
        }
    }

    #[test]
    fn limited_capture_truncates_an_endless_source() {
        // MockCapture would produce 100; the limiter ends the stream at 3.
        let mut cap = LimitedCapture::new(MockCapture::new(100, 30), 3);
        let mut n = 0;
        while cap.next_frame().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
    }

    #[test]
    fn mock_audio_source_drains_packets_up_to_pts() {
        use crate::traits::AudioSource;
        let mut src = MockAudioSource::new(48_000, 20);
        // Packets are 20 ms; "up to 0.05 s" = the two ending at 0.02/0.04.
        let batch = src.poll_packets(0.05).unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].pts_s, 0.0);
        assert!((batch[1].pts_s - 0.02).abs() < 1e-9);
        assert!((batch[0].duration_s - 0.02).abs() < 1e-9);
        // Next drain continues where the last left off; packet at 0.04
        // (ending 0.06) arrives once 0.06 is reachable.
        let batch2 = src.poll_packets(0.06).unwrap();
        assert_eq!(batch2.len(), 1);
        assert!((batch2[0].pts_s - 0.04).abs() < 1e-9);
    }

    #[test]
    fn mock_audio_source_has_a_muxable_config() {
        use crate::traits::AudioSource;
        let src = MockAudioSource::new(48_000, 20);
        let cfg = src.track_config();
        assert_eq!(cfg.sample_rate, 48_000);
        assert_eq!(cfg.channels, 2);
    }
}
