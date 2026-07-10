//! PCM continuity between QPC-stamped WASAPI chunks and exact Opus frames.
//! Loopback goes quiet when nothing renders; the MP4 audio timeline is
//! duration-cumulative, so gaps MUST become silence or A/V desyncs.

use crate::opus::{FRAME_DURATION_S, FRAME_LEN};

use std::collections::VecDeque;

const SAMPLE_RATE: f64 = 48_000.0;
/// Gaps shorter than half a frame are treated as device jitter.
const GAP_TOLERANCE_S: f64 = FRAME_DURATION_S / 2.0;
/// A bogus device timestamp must not allocate unbounded silence.
const MAX_GAP_FILL_S: f64 = 5.0;
const MIX_FRAME_EPSILON_S: f64 = FRAME_DURATION_S / 2.0;
const MISSING_SOURCE_GRACE_S: f64 = FRAME_DURATION_S * 3.0;

pub type PcmFrame = (f64, Vec<f32>);

#[derive(Default)]
pub struct LoopbackAssembler {
    /// pts of the first sample ever pushed; frame N pops at base + N*0.02.
    base_pts_s: Option<f64>,
    /// Expected WASAPI timestamp for the next finite chunk.
    next_chunk_pts_s: Option<f64>,
    buffered: Vec<f32>, // interleaved stereo
    frames_popped: u64,
}

impl LoopbackAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// `pts_s` stamps the first sample of `interleaved` (stereo pairs).
    pub fn push_chunk(&mut self, pts_s: f64, interleaved: &[f32]) {
        if !pts_s.is_finite() {
            self.push_contiguous_chunk(interleaved);
            return;
        }
        self.base_pts_s.get_or_insert(pts_s);
        let expected = self.next_chunk_pts_s.unwrap_or(pts_s);
        let gap = pts_s - expected;
        let mut samples = interleaved;
        if gap > GAP_TOLERANCE_S {
            let missing_pairs = (gap.min(MAX_GAP_FILL_S) * SAMPLE_RATE).round() as usize;
            self.buffered
                .extend(std::iter::repeat_n(0.0, missing_pairs * 2));
        } else if gap < -GAP_TOLERANCE_S {
            let overlap_pairs = ((expected - pts_s) * SAMPLE_RATE).round() as usize;
            if overlap_pairs >= interleaved.len() / 2 {
                return;
            }
            samples = &interleaved[overlap_pairs * 2..];
        }
        self.buffered.extend_from_slice(samples);
        let chunk_duration_s = (samples.len() / 2) as f64 / SAMPLE_RATE;
        self.next_chunk_pts_s = Some(if gap > GAP_TOLERANCE_S {
            pts_s + chunk_duration_s
        } else {
            expected + chunk_duration_s
        });
    }

    /// Extend an anchored timeline with stereo silence up to an absolute PTS.
    /// Non-finite or non-forward targets are ignored. One call is bounded by
    /// the same limit used for timestamp-discovered gaps.
    pub fn advance_with_silence(&mut self, target_pts_s: f64) {
        if !target_pts_s.is_finite() {
            return;
        }
        let Some(expected_pts_s) = self.next_chunk_pts_s else {
            return;
        };
        let gap_s = target_pts_s - expected_pts_s;
        if gap_s <= 0.0 {
            return;
        }
        let missing_pairs = (gap_s.min(MAX_GAP_FILL_S) * SAMPLE_RATE).round() as usize;
        self.buffered
            .extend(std::iter::repeat_n(0.0, missing_pairs * 2));
        self.next_chunk_pts_s = Some(expected_pts_s + missing_pairs as f64 / SAMPLE_RATE);
    }

    /// Append a chunk when the device explicitly marked its timestamp invalid.
    pub fn push_contiguous_chunk(&mut self, interleaved: &[f32]) {
        self.base_pts_s.get_or_insert(0.0);
        self.buffered.extend_from_slice(interleaved);
        if let Some(next_chunk_pts_s) = &mut self.next_chunk_pts_s {
            *next_chunk_pts_s += (interleaved.len() / 2) as f64 / SAMPLE_RATE;
        }
    }

    /// One 20 ms frame once enough samples are buffered.
    pub fn pop_frame(&mut self) -> Option<PcmFrame> {
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

/// Average every source channel into a centered stereo pair. A stereo mic
/// that only uses one side becomes centered at half amplitude by design.
pub fn extract_mono_centered(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mut out = Vec::with_capacity(samples.len() / ch * 2);
    for frame in samples.chunks_exact(ch) {
        let mono = frame.iter().sum::<f32>() / ch as f32;
        out.push(mono);
        out.push(mono);
    }
    out
}

pub fn apply_gain(samples: &mut [f32], gain: f32) {
    for sample in samples {
        *sample = (*sample * gain).clamp(-1.0, 1.0);
    }
}

#[derive(Debug)]
pub struct StereoResampler {
    from_rate: u32,
    to_rate: u32,
    next_source_frame: f64,
    input_frames_seen: u64,
    last_frame: Option<[f32; 2]>,
}

impl StereoResampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            from_rate,
            to_rate,
            next_source_frame: 0.0,
            input_frames_seen: 0,
            last_frame: None,
        }
    }

    pub fn resample(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() || self.from_rate == 0 || self.to_rate == 0 {
            return Vec::new();
        }
        let in_frames = samples.len() / 2;
        if in_frames == 0 {
            return Vec::new();
        }
        if self.from_rate == self.to_rate {
            self.input_frames_seen += in_frames as u64;
            self.next_source_frame = self.input_frames_seen as f64;
            self.last_frame = Some(last_stereo_frame(samples, in_frames));
            return samples[..in_frames * 2].to_vec();
        }

        let chunk_start = self.input_frames_seen as f64;
        let chunk_end = chunk_start + in_frames as f64;
        let step = self.from_rate as f64 / self.to_rate as f64;
        let mut out = Vec::new();

        while self.next_source_frame < chunk_end - 1e-12 {
            let relative_pos = self.next_source_frame - chunk_start;
            let [left, right] = self.sample_at(samples, in_frames, relative_pos);
            out.push(left);
            out.push(right);
            self.next_source_frame += step;
        }

        self.input_frames_seen += in_frames as u64;
        self.last_frame = Some(last_stereo_frame(samples, in_frames));
        out
    }

    fn sample_at(&self, samples: &[f32], in_frames: usize, relative_pos: f64) -> [f32; 2] {
        if relative_pos < 0.0 {
            let current = [samples[0], samples[1]];
            let previous = self.last_frame.unwrap_or(current);
            let t = (relative_pos + 1.0).clamp(0.0, 1.0) as f32;
            return [
                previous[0] + (current[0] - previous[0]) * t,
                previous[1] + (current[1] - previous[1]) * t,
            ];
        }

        let a = relative_pos.floor() as usize;
        if a + 1 >= in_frames {
            return last_stereo_frame(samples, in_frames);
        }
        let b = a + 1;
        let t = (relative_pos - a as f64) as f32;
        [
            samples[a * 2] + (samples[b * 2] - samples[a * 2]) * t,
            samples[a * 2 + 1] + (samples[b * 2 + 1] - samples[a * 2 + 1]) * t,
        ]
    }
}

fn last_stereo_frame(samples: &[f32], in_frames: usize) -> [f32; 2] {
    [
        samples[(in_frames - 1) * 2],
        samples[(in_frames - 1) * 2 + 1],
    ]
}

pub fn resample_stereo_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    StereoResampler::new(from_rate, to_rate).resample(samples)
}

#[derive(Debug)]
pub struct PcmFrameMixer {
    pending: Vec<VecDeque<PcmFrame>>,
    source_ready_until_s: Vec<f64>,
    mixed_until_s: f64,
}

impl PcmFrameMixer {
    pub fn new(source_count: usize) -> Self {
        Self {
            pending: (0..source_count).map(|_| VecDeque::new()).collect(),
            source_ready_until_s: vec![0.0; source_count],
            mixed_until_s: 0.0,
        }
    }

    pub fn push_source_frames(
        &mut self,
        source_index: usize,
        frames: impl IntoIterator<Item = PcmFrame>,
    ) {
        let pending = self
            .pending
            .get_mut(source_index)
            .expect("source index must match mixer source count");
        let ready_until_s = self
            .source_ready_until_s
            .get_mut(source_index)
            .expect("source index must match mixer source count");
        for frame in frames {
            *ready_until_s = ready_until_s.max(frame.0 + FRAME_DURATION_S);
            pending.push_back(frame);
        }
    }

    pub fn pop_mixed_frames(&mut self, until_pts_s: f64) -> Vec<PcmFrame> {
        let mut mixed = Vec::new();
        while self.mixed_until_s + FRAME_DURATION_S <= until_pts_s + 1e-9 {
            if !self.pending.iter().any(|pending| pending.front().is_some()) {
                break;
            }
            let target_pts_s = self.mixed_until_s;
            let all_sources_ready = self.pending.iter().zip(&self.source_ready_until_s).all(
                |(pending, &ready_until_s)| {
                    source_ready_for_mix(pending, ready_until_s, target_pts_s, until_pts_s)
                },
            );
            if !all_sources_ready {
                break;
            }

            let mut frames = Vec::with_capacity(self.pending.len());
            for pending in &mut self.pending {
                while pending
                    .front()
                    .is_some_and(|(pts_s, _)| *pts_s < target_pts_s - MIX_FRAME_EPSILON_S)
                {
                    pending.pop_front();
                }
                let should_pop = pending
                    .front()
                    .is_some_and(|(pts_s, _)| (*pts_s - target_pts_s).abs() <= MIX_FRAME_EPSILON_S);
                frames.push(should_pop.then(|| pending.pop_front().expect("checked front").1));
            }

            mixed.push((target_pts_s, mix_optional_frames(&frames)));
            self.mixed_until_s += FRAME_DURATION_S;
        }
        mixed
    }
}

fn source_ready_for_mix(
    pending: &VecDeque<PcmFrame>,
    ready_until_s: f64,
    target_pts_s: f64,
    until_pts_s: f64,
) -> bool {
    if pending.front().is_some() {
        return true;
    }
    ready_until_s >= target_pts_s + FRAME_DURATION_S - 1e-9
        || until_pts_s >= target_pts_s + MISSING_SOURCE_GRACE_S
}

pub fn mix_optional_frames(frames: &[Option<Vec<f32>>]) -> Vec<f32> {
    let mut mixed = vec![0.0; FRAME_LEN];
    for frame in frames.iter().filter_map(|frame| frame.as_ref()) {
        for (out, sample) in mixed.iter_mut().zip(frame.iter().copied()) {
            *out += sample;
        }
    }
    for sample in &mut mixed {
        *sample = sample.clamp(-1.0, 1.0);
    }
    mixed
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
    fn anchored_assembler_advances_through_device_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &[]);

        asm.advance_with_silence(0.5);

        let mut frames = Vec::new();
        while let Some(frame) = asm.pop_frame() {
            frames.push(frame);
        }
        assert_eq!(frames.len(), 25);
        for (index, (pts_s, frame)) in frames.iter().enumerate() {
            assert!((*pts_s - index as f64 * FRAME_DURATION_S).abs() < 1e-9);
            assert!(frame.iter().all(|&sample| sample == 0.0));
        }
    }

    #[test]
    fn silence_advancement_is_monotonic_and_idempotent() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &[]);

        asm.advance_with_silence(0.04);
        asm.advance_with_silence(0.02);
        asm.advance_with_silence(0.04);

        assert_eq!(std::iter::from_fn(|| asm.pop_frame()).count(), 2);
    }

    #[test]
    fn non_finite_silence_horizon_does_not_advance() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &[]);

        asm.advance_with_silence(f64::INFINITY);
        asm.advance_with_silence(f64::NAN);

        assert!(asm.pop_frame().is_none());
    }

    #[test]
    fn caps_huge_timestamp_gaps() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        asm.push_chunk(3600.0, &pairs(960, 1.0));

        let max_missing_pairs = (MAX_GAP_FILL_S * SAMPLE_RATE).round() as usize;
        assert_eq!(asm.buffered.len(), (960 + max_missing_pairs + 960) * 2);
    }

    #[test]
    fn capped_timestamp_gap_does_not_repeat_for_following_chunks() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        asm.push_chunk(3600.0, &pairs(960, 0.5));
        asm.push_chunk(3600.02, &pairs(960, 0.25));

        let max_missing_pairs = (MAX_GAP_FILL_S * SAMPLE_RATE).round() as usize;
        assert_eq!(
            asm.buffered.len(),
            (960 + max_missing_pairs + 960 + 960) * 2
        );
    }

    #[test]
    fn invalid_timestamp_chunks_append_contiguously() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &pairs(960, 1.0));
        asm.push_chunk(f64::NAN, &pairs(960, 0.5));

        let first = asm.pop_frame().expect("first frame");
        let second = asm.pop_frame().expect("second frame");
        assert!(first.1.iter().all(|&s| s == 1.0));
        assert!(second.1.iter().all(|&s| s == 0.5));
        assert!(asm.pop_frame().is_none());
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
    fn late_chunk_keeps_only_suffix_after_synthesized_silence() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &[]);
        asm.advance_with_silence(0.10);

        asm.push_chunk(0.08, &pairs(1_920, 0.75));

        let mut frames = Vec::new();
        while let Some(frame) = asm.pop_frame() {
            frames.push(frame);
        }
        assert_eq!(frames.len(), 6);
        assert!(frames[..5]
            .iter()
            .all(|(_, frame)| frame.iter().all(|&sample| sample == 0.0)));
        assert!((frames[5].0 - 0.10).abs() < 1e-9);
        assert!(frames[5].1.iter().all(|&sample| sample == 0.75));
    }

    #[test]
    fn fully_overlapped_late_chunk_does_not_extend_timeline() {
        let mut asm = LoopbackAssembler::new();
        asm.push_chunk(0.0, &[]);
        asm.advance_with_silence(0.10);

        asm.push_chunk(0.04, &pairs(960, 0.75));

        assert_eq!(std::iter::from_fn(|| asm.pop_frame()).count(), 5);
    }

    #[test]
    fn seeded_assemblers_share_a_frame_grid_despite_phase_offsets() {
        let mut a = LoopbackAssembler::new();
        let mut b = LoopbackAssembler::new();
        a.push_chunk(0.0, &[]);
        b.push_chunk(0.0, &[]);

        a.push_chunk(0.004, &pairs(960, 1.0));
        b.push_chunk(0.012, &pairs(960, 1.0));

        let (a_pts, _) = a.pop_frame().expect("source a frame");
        let (b_pts, _) = b.pop_frame().expect("source b frame");
        assert_eq!(a_pts, 0.0);
        assert_eq!(b_pts, 0.0);
    }

    #[test]
    fn extract_stereo_handles_channel_counts() {
        // Stereo passes through.
        assert_eq!(
            extract_stereo(&[1.0, 2.0, 3.0, 4.0], 2),
            vec![1.0, 2.0, 3.0, 4.0]
        );
        // 5.1 keeps front L/R.
        let six: Vec<f32> = (0..12).map(|i| i as f32).collect();
        assert_eq!(extract_stereo(&six, 6), vec![0.0, 1.0, 6.0, 7.0]);
        // Mono duplicates.
        assert_eq!(extract_stereo(&[0.5, 0.7], 1), vec![0.5, 0.5, 0.7, 0.7]);
    }

    #[test]
    fn extract_mono_centered_averages_channels() {
        assert_eq!(
            extract_mono_centered(&[0.2, 0.6, -0.2, 0.2], 2),
            vec![0.4, 0.4, 0.0, 0.0]
        );
        assert_eq!(
            extract_mono_centered(&[0.5, 0.7], 1),
            vec![0.5, 0.5, 0.7, 0.7]
        );
    }

    #[test]
    fn apply_gain_scales_and_clamps_samples() {
        let mut samples = vec![-0.6, 0.25, 0.75];
        apply_gain(&mut samples, 2.0);
        assert_eq!(samples, vec![-1.0, 0.5, 1.0]);
    }

    #[test]
    fn resample_stereo_linear_preserves_stereo_pairs() {
        let samples = vec![0.0, 1.0, 0.5, 0.5, 1.0, 0.0, 0.5, -0.5];
        assert_eq!(resample_stereo_linear(&samples, 48_000, 48_000), samples);

        let down = resample_stereo_linear(&samples, 48_000, 24_000);
        assert_eq!(down, vec![0.0, 1.0, 1.0, 0.0]);

        let up = resample_stereo_linear(&[0.0, 1.0, 1.0, 0.0], 24_000, 48_000);
        assert_eq!(up.len(), 8);
        assert_eq!(&up[..4], &[0.0, 1.0, 0.5, 0.5]);
    }

    #[test]
    fn stateful_resampler_carries_fractional_position_across_chunks() {
        let mut resampler = StereoResampler::new(44_100, 48_000);
        let chunk = pairs(100, 0.25);
        let mut output_frames = 0usize;
        for _ in 0..1000 {
            output_frames += resampler.resample(&chunk).len() / 2;
        }

        let input_frames = 100_000f64;
        let expected = (input_frames * 48_000.0 / 44_100.0).ceil() as usize;
        assert_eq!(output_frames, expected);
    }

    #[test]
    fn mixer_outputs_one_frame_per_grid_slot_for_offset_sources() {
        let mut mixer = PcmFrameMixer::new(2);
        mixer.push_source_frames(
            0,
            [
                (0.0, pairs(960, 0.25)),
                (0.02, pairs(960, 0.25)),
                (0.04, pairs(960, 0.25)),
            ],
        );
        mixer.push_source_frames(
            1,
            [
                (0.006, pairs(960, 0.5)),
                (0.026, pairs(960, 0.5)),
                (0.046, pairs(960, 0.5)),
            ],
        );

        let frames = mixer.pop_mixed_frames(0.06);

        assert_eq!(frames.len(), 3);
        for (i, (pts_s, frame)) in frames.iter().enumerate() {
            assert!((*pts_s - i as f64 * FRAME_DURATION_S).abs() < 1e-9);
            assert!(frame.iter().all(|&sample| (sample - 0.75).abs() < 1e-6));
        }
    }

    #[test]
    fn mixer_nearest_pairs_late_phase_frames_without_overlap_packets() {
        let mut mixer = PcmFrameMixer::new(2);
        mixer.push_source_frames(
            0,
            [
                (0.0, pairs(960, 0.25)),
                (0.02, pairs(960, 0.25)),
                (0.04, pairs(960, 0.25)),
            ],
        );
        mixer.push_source_frames(
            1,
            [
                (0.015, pairs(960, 0.5)),
                (0.035, pairs(960, 0.5)),
                (0.055, pairs(960, 0.5)),
            ],
        );

        let frames = mixer.pop_mixed_frames(0.06);

        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].0, 0.0);
        assert!(frames[0]
            .1
            .iter()
            .all(|&sample| (sample - 0.25).abs() < 1e-6));
        assert_eq!(frames[1].0, 0.02);
        assert!(frames[1]
            .1
            .iter()
            .all(|&sample| (sample - 0.75).abs() < 1e-6));
        assert_eq!(frames[2].0, 0.04);
        assert!(frames[2]
            .1
            .iter()
            .all(|&sample| (sample - 0.75).abs() < 1e-6));
    }

    #[test]
    fn mixer_waits_briefly_for_missing_sources() {
        let mut mixer = PcmFrameMixer::new(2);
        mixer.push_source_frames(0, [(0.0, pairs(960, 0.25))]);

        assert!(
            mixer.pop_mixed_frames(0.02).is_empty(),
            "missing source should not become silence immediately"
        );

        let frames = mixer.pop_mixed_frames(0.08);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, 0.0);
        assert!(frames[0]
            .1
            .iter()
            .all(|&sample| (sample - 0.25).abs() < 1e-6));
    }
}
