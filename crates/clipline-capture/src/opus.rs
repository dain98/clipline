//! Real Opus encoding (ddoc §4: AV1+Opus default; handoff: real Opus
//! before shipping). Fixed 20 ms stereo frames at 48 kHz — the shape both
//! the muxer (dOps) and the WASAPI assembler agree on.

use audiopus::coder::Encoder;
use audiopus::{Application, Channels, SampleRate};
use clipline_mp4::AudioTrackConfig;

/// Samples per channel in one 20 ms frame at 48 kHz.
pub const FRAME_SAMPLES: usize = 960;
/// Interleaved stereo length of one frame.
pub const FRAME_LEN: usize = FRAME_SAMPLES * 2;
pub const FRAME_DURATION_S: f64 = 0.02;

pub struct OpusFrameEncoder {
    encoder: Encoder,
    pre_skip: u16,
}

impl OpusFrameEncoder {
    pub fn new() -> Result<Self, audiopus::Error> {
        let encoder = Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio)?;
        let pre_skip = encoder.lookahead()? as u16;
        Ok(Self { encoder, pre_skip })
    }

    /// Encode one interleaved stereo frame (`FRAME_LEN` floats).
    pub fn encode_frame(&mut self, interleaved: &[f32]) -> Result<Vec<u8>, audiopus::Error> {
        let mut out = vec![0u8; 4000];
        let n = self.encoder.encode_float(interleaved, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Encoder lookahead in 48 kHz samples (dOps PreSkip).
    pub fn pre_skip(&self) -> u16 {
        self.pre_skip
    }

    pub fn track_config(&self) -> AudioTrackConfig {
        AudioTrackConfig { channels: 2, sample_rate: 48_000, pre_skip: self.pre_skip }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_frame() -> Vec<f32> {
        // 960 samples of 440 Hz stereo at 48 kHz, interleaved.
        (0..FRAME_SAMPLES)
            .flat_map(|i| {
                let s = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn encodes_a_20ms_stereo_frame() {
        let mut enc = OpusFrameEncoder::new().expect("opus encoder");
        let packet = enc.encode_frame(&sine_frame()).expect("encode");
        assert!(!packet.is_empty());
        assert!(packet.len() < 1500, "20ms of opus is small, got {}", packet.len());
    }

    #[test]
    fn rejects_wrong_frame_size() {
        let mut enc = OpusFrameEncoder::new().unwrap();
        assert!(enc.encode_frame(&[0.0; 100]).is_err());
    }

    #[test]
    fn exposes_a_sane_pre_skip() {
        let enc = OpusFrameEncoder::new().unwrap();
        let ps = enc.pre_skip();
        assert!(ps > 0 && ps < 1000, "lookahead in 48k samples, got {ps}");
    }

    #[test]
    fn track_config_matches_the_muxer_contract() {
        let enc = OpusFrameEncoder::new().unwrap();
        let cfg = enc.track_config();
        assert_eq!((cfg.channels, cfg.sample_rate), (2, 48_000));
        assert_eq!(cfg.pre_skip, enc.pre_skip());
    }
}
