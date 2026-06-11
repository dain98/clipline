//! PCM continuity between QPC-stamped WASAPI chunks and exact Opus frames.
//! Loopback goes quiet when nothing renders; the MP4 audio timeline is
//! duration-cumulative, so gaps MUST become silence or A/V desyncs.

use crate::opus::{FRAME_DURATION_S, FRAME_LEN};

const SAMPLE_RATE: f64 = 48_000.0;
/// Gaps shorter than half a frame are treated as device jitter.
const GAP_TOLERANCE_S: f64 = FRAME_DURATION_S / 2.0;

#[derive(Default)]
pub struct LoopbackAssembler {
    /// pts of the first sample ever pushed; frame N pops at base + N*0.02.
    base_pts_s: Option<f64>,
    buffered: Vec<f32>, // interleaved stereo
    frames_popped: u64,
}

impl LoopbackAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// `pts_s` stamps the first sample of `interleaved` (stereo pairs).
    pub fn push_chunk(&mut self, pts_s: f64, interleaved: &[f32]) {
        let base = *self.base_pts_s.get_or_insert(pts_s);
        let expected = base
            + self.frames_popped as f64 * FRAME_DURATION_S
            + (self.buffered.len() / 2) as f64 / SAMPLE_RATE;
        let gap = pts_s - expected;
        if gap > GAP_TOLERANCE_S {
            let missing_pairs = (gap * SAMPLE_RATE).round() as usize;
            self.buffered.extend(std::iter::repeat(0.0).take(missing_pairs * 2));
        }
        self.buffered.extend_from_slice(interleaved);
    }

    /// One 20 ms frame once enough samples are buffered.
    pub fn pop_frame(&mut self) -> Option<(f64, Vec<f32>)> {
        if self.buffered.len() < FRAME_LEN {
            return None;
        }
        let pts = self.base_pts_s? + self.frames_popped as f64 * FRAME_DURATION_S;
        let frame: Vec<f32> = self.buffered.drain(..FRAME_LEN).collect();
        self.frames_popped += 1;
        Some((pts, frame))
    }
}

/// First two channels of N-channel interleaved float PCM (mono duplicates).
pub fn extract_stereo(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mut out = Vec::with_capacity(samples.len() / ch * 2);
    for frame in samples.chunks_exact(ch) {
        out.push(frame[0]);
        out.push(if ch >= 2 { frame[1] } else { frame[0] });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opus::{FRAME_DURATION_S, FRAME_LEN};

    fn pairs(n: usize, v: f32) -> Vec<f32> {
        vec![v; n * 2]
    }

    #[test]
    fn slices_contiguous_chunks_into_frames() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(1.0, &pairs(960, 0.5)); // exactly one frame
        asm.push_chunk(1.02, &pairs(480, 0.25)); // half a frame
        let (pts, frame) = asm.pop_frame().expect("one full frame");
        assert_eq!(pts, 1.0);
        assert_eq!(frame.len(), FRAME_LEN);
        assert!(frame.iter().all(|&s| s == 0.5));
        assert!(asm.pop_frame().is_none(), "half frame still pending");
        asm.push_chunk(1.03, &pairs(480, 0.25));
        let (pts2, _) = asm.pop_frame().expect("second frame");
        assert!((pts2 - (1.0 + FRAME_DURATION_S)).abs() < 1e-9);
    }

    #[test]
    fn fills_gaps_with_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        // 40 ms gap (nothing rendered): next chunk stamps at 0.06.
        asm.push_chunk(0.06, &pairs(960, 1.0));
        let mut frames = Vec::new();
        while let Some(f) = asm.pop_frame() {
            frames.push(f);
        }
        assert_eq!(frames.len(), 4, "audio + 2 silence frames + audio");
        assert!(frames[0].1.iter().all(|&s| s == 1.0));
        assert!(frames[1].1.iter().all(|&s| s == 0.0), "gap became silence");
        assert!(frames[2].1.iter().all(|&s| s == 0.0), "gap became silence");
        assert!(frames[3].1.iter().all(|&s| s == 1.0), "audio resumes");
        // pts stays continuous despite the gap: the resumed audio lands at 0.06.
        assert!((frames[3].0 - 0.06).abs() < 1e-9);
    }

    #[test]
    fn small_jitter_does_not_insert_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        // 5 ms late — within tolerance, treated as contiguous.
        asm.push_chunk(0.025, &pairs(960, 1.0));
        let mut n = 0;
        while asm.pop_frame().is_some() {
            n += 1;
        }
        assert_eq!(n, 2, "no silence frames inserted");
    }

    #[test]
    fn extract_stereo_handles_channel_counts() {
        // Stereo passes through.
        assert_eq!(extract_stereo(&[1.0, 2.0, 3.0, 4.0], 2), vec![1.0, 2.0, 3.0, 4.0]);
        // 5.1 keeps front L/R.
        let six: Vec<f32> = (0..12).map(|i| i as f32).collect();
        assert_eq!(extract_stereo(&six, 6), vec![0.0, 1.0, 6.0, 7.0]);
        // Mono duplicates.
        assert_eq!(extract_stereo(&[0.5, 0.7], 1), vec![0.5, 0.5, 0.7, 0.7]);
    }
}
