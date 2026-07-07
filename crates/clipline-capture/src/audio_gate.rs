#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAudioSource;

    #[test]
    fn game_mode_passes_inner_packets_through() {
        let state = AudioPrivacyState::new_game();
        let mut gate =
            PrivacyAudioGate::new(Box::new(MockAudioSource::new(48_000, 20)), state).expect("gate");

        let packets = gate.poll_packets(0.04).unwrap();

        assert_eq!(packets.len(), 2);
        assert!(packets[0].data.starts_with(b"P00000"));
        assert!(packets[1].data.starts_with(b"P00001"));
    }

    #[test]
    fn slate_mode_drains_inner_and_emits_silence() {
        let state = AudioPrivacyState::new_game();
        let mut gate =
            PrivacyAudioGate::new(Box::new(MockAudioSource::new(48_000, 20)), state.clone())
                .expect("gate");
        state.set_slate(true);

        let packets = gate.poll_packets(0.06).unwrap();

        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0].pts_s, 0.0);
        assert_eq!(packets[1].pts_s, 0.02);
        assert_eq!(packets[2].pts_s, 0.04);
        assert!(packets.iter().all(|packet| !packet.data.starts_with(b"P")));

        state.set_slate(false);
        let resumed = gate.poll_packets(0.08).unwrap();
        assert_eq!(resumed.len(), 1);
        assert!(resumed[0].pts_s >= 0.06);
    }

    #[test]
    fn slate_mode_caps_silence_backfill_after_long_gap() {
        let state = AudioPrivacyState::new_game();
        let mut gate =
            PrivacyAudioGate::new(Box::new(MockAudioSource::new(48_000, 20)), state.clone())
                .expect("gate");
        state.set_slate(true);

        let packets = gate.poll_packets(30.0).unwrap();

        assert!(packets.len() <= 101, "silence backfill should stay bounded");
        assert!(packets[0].pts_s >= 30.0 - MAX_SILENCE_BACKFILL_S - 1e-9);
        assert!(packets.iter().all(|packet| !packet.data.starts_with(b"P")));
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clipline_mp4::AudioTrackConfig;

use crate::opus::{OpusFrameEncoder, FRAME_DURATION_S, FRAME_LEN};
use crate::traits::{AudioPacket, AudioSource, CaptureError};

const MAX_SILENCE_BACKFILL_S: f64 = 2.0;

#[derive(Clone, Debug)]
pub struct AudioPrivacyState {
    slate: Arc<AtomicBool>,
}

impl AudioPrivacyState {
    pub fn new_game() -> Self {
        Self {
            slate: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_slate(&self, slate: bool) {
        self.slate.store(slate, Ordering::Release);
    }

    pub fn is_slate(&self) -> bool {
        self.slate.load(Ordering::Acquire)
    }
}

pub struct PrivacyAudioGate {
    inner: Box<dyn AudioSource>,
    state: AudioPrivacyState,
    opus: OpusFrameEncoder,
    silence_frame: Vec<f32>,
    next_silence_pts_s: f64,
}

impl PrivacyAudioGate {
    pub fn new(
        inner: Box<dyn AudioSource>,
        state: AudioPrivacyState,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            inner,
            state,
            opus: OpusFrameEncoder::new()
                .map_err(|e| CaptureError::Init(format!("opus silence: {e}")))?,
            silence_frame: vec![0.0; FRAME_LEN],
            next_silence_pts_s: 0.0,
        })
    }
}

impl AudioSource for PrivacyAudioGate {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        let inner_packets = self.inner.poll_packets(until_pts_s)?;
        if !self.state.is_slate() {
            if let Some(last) = inner_packets.last() {
                self.next_silence_pts_s = last.pts_s + last.duration_s;
            }
            return Ok(inner_packets);
        }

        if let Some(first) = inner_packets.first() {
            self.next_silence_pts_s = self.next_silence_pts_s.max(first.pts_s);
        }
        self.next_silence_pts_s = self
            .next_silence_pts_s
            .max((until_pts_s - MAX_SILENCE_BACKFILL_S).max(0.0));
        let mut out = Vec::new();
        while self.next_silence_pts_s + FRAME_DURATION_S <= until_pts_s + 1e-9 {
            let data = self
                .opus
                .encode_frame(&self.silence_frame)
                .map_err(|e| CaptureError::DeviceLost(format!("opus silence encode: {e}")))?;
            out.push(AudioPacket {
                data,
                pts_s: self.next_silence_pts_s,
                duration_s: FRAME_DURATION_S,
            });
            self.next_silence_pts_s += FRAME_DURATION_S;
        }
        Ok(out)
    }

    fn track_config(&self) -> AudioTrackConfig {
        self.inner.track_config()
    }
}
