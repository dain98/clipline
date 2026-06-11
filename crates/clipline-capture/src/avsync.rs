//! Tolerance-based A/V timeline validation (ddoc §6, handoff milestone 4:
//! the mock tests pinned GOP-boundary behavior exactly; real clocks get
//! tolerances, not exact equality). Runs over sealed segments — from the
//! live ring or a saved window — and either reports the timeline's health
//! or names the first violation.

use clipline_buffer::Segment;

pub struct SyncTolerances {
    /// Max allowed |gap| between consecutive segments (s).
    pub max_video_gap_s: f64,
    /// Max |audio coverage − segment duration| per segment (s).
    pub max_audio_segment_skew_s: f64,
    /// Max |cumulative audio − cumulative video| at the end (s).
    pub max_total_drift_s: f64,
}

impl Default for SyncTolerances {
    fn default() -> Self {
        Self {
            max_video_gap_s: 0.005,
            // Two 20 ms opus frames + scheduling jitter.
            max_audio_segment_skew_s: 0.045,
            max_total_drift_s: 0.045,
        }
    }
}

#[derive(Debug)]
pub struct SyncReport {
    pub segments: usize,
    pub video_duration_s: f64,
    /// Cumulative audio coverage per track.
    pub audio_duration_s: Vec<f64>,
    pub max_video_gap_s: f64,
    pub max_audio_segment_skew_s: f64,
    /// audio − video at the end, per track (negative: audio short).
    pub total_drift_s: Vec<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncViolation {
    #[error("segment {segment} does not start with a keyframe")]
    NotKeyframeLed { segment: usize },
    #[error("segment {segment} starts {gap_s:.4}s away from the previous end")]
    VideoGap { segment: usize, gap_s: f64 },
    #[error("segment {segment} audio track {track} skewed {skew_s:.4}s vs video")]
    AudioSegmentSkew { segment: usize, track: usize, skew_s: f64 },
    #[error("audio track {track} drifted {drift_s:.4}s vs video over the recording")]
    TotalDrift { track: usize, drift_s: f64 },
}

pub fn validate_timeline(
    segments: &[&Segment],
    tol: &SyncTolerances,
) -> Result<SyncReport, SyncViolation> {
    let tracks = segments.first().map(|s| s.audio.len()).unwrap_or(0);
    let mut report = SyncReport {
        segments: segments.len(),
        video_duration_s: 0.0,
        audio_duration_s: vec![0.0; tracks],
        max_video_gap_s: 0.0,
        max_audio_segment_skew_s: 0.0,
        total_drift_s: vec![0.0; tracks],
    };
    let mut prev_end: Option<f64> = None;
    for (i, seg) in segments.iter().enumerate() {
        if !seg.starts_with_keyframe {
            return Err(SyncViolation::NotKeyframeLed { segment: i });
        }
        if let Some(end) = prev_end {
            let gap = (seg.pts_start_s - end).abs();
            report.max_video_gap_s = report.max_video_gap_s.max(gap);
            if gap > tol.max_video_gap_s {
                return Err(SyncViolation::VideoGap { segment: i, gap_s: seg.pts_start_s - end });
            }
        }
        prev_end = Some(seg.pts_end_s());
        report.video_duration_s += seg.duration_s;

        let last = i + 1 == segments.len();
        for (t, track) in seg.audio.iter().enumerate() {
            let covered: f64 = track.samples.iter().map(|s| s.duration_s).sum();
            report.audio_duration_s[t] += covered;
            let skew = covered - seg.duration_s;
            // The final segment's audio may legitimately run short (the
            // last poll lags video); it must never run long.
            let violating = if last { skew > tol.max_audio_segment_skew_s } else {
                skew.abs() > tol.max_audio_segment_skew_s
            };
            if violating {
                return Err(SyncViolation::AudioSegmentSkew { segment: i, track: t, skew_s: skew });
            }
            if !last {
                report.max_audio_segment_skew_s = report.max_audio_segment_skew_s.max(skew.abs());
            }
        }
    }
    for t in 0..tracks {
        let drift = report.audio_duration_s[t] - report.video_duration_s;
        report.total_drift_s[t] = drift;
        // Audio short at the tail is acceptable; long is not, and short
        // beyond tolerance is only ok in the final-segment lag sense —
        // bound the magnitude either way.
        if drift.abs() > tol.max_total_drift_s {
            return Err(SyncViolation::TotalDrift { track: t, drift_s: drift });
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockAudioSource, MockCapture, MockEncoder};
    use crate::pipeline::Recorder;
    use clipline_buffer::{SampleInfo, Segment, TrackSamples};

    fn mock_recording() -> Vec<Segment> {
        let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), usize::MAX)
            .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        rec.ring().segments().cloned().collect()
    }

    #[test]
    fn mock_recording_validates_clean() {
        let segs = mock_recording();
        let refs: Vec<&Segment> = segs.iter().collect();
        let report = validate_timeline(&refs, &SyncTolerances::default()).expect("clean");
        assert_eq!(report.segments, 3);
        assert!((report.video_duration_s - 3.0).abs() < 1e-6);
        assert!(report.max_video_gap_s < 1e-9, "mock GOPs are seamless");
        assert!(report.max_audio_segment_skew_s < 1e-9);
        assert!(report.total_drift_s[0].abs() < 1e-9);
    }

    fn video_seg(pts: f64, n: usize, frame_s: f64, key: bool) -> Segment {
        Segment {
            starts_with_keyframe: key,
            pts_start_s: pts,
            duration_s: n as f64 * frame_s,
            data: vec![0; n],
            samples: (0..n)
                .map(|_| SampleInfo { size: 1, duration_s: frame_s, is_sync: false })
                .collect(),
            audio: Vec::new(),
        }
    }

    #[test]
    fn rejects_non_keyframe_led_segment() {
        let segs = vec![video_seg(0.0, 30, 1.0 / 30.0, false)];
        let refs: Vec<&Segment> = segs.iter().collect();
        match validate_timeline(&refs, &SyncTolerances::default()) {
            Err(SyncViolation::NotKeyframeLed { segment: 0 }) => {}
            other => panic!("expected NotKeyframeLed, got {other:?}"),
        }
    }

    #[test]
    fn rejects_inter_segment_video_gap() {
        // Second segment starts 50 ms after the first ends.
        let segs =
            vec![video_seg(0.0, 30, 1.0 / 30.0, true), video_seg(1.05, 30, 1.0 / 30.0, true)];
        let refs: Vec<&Segment> = segs.iter().collect();
        match validate_timeline(&refs, &SyncTolerances::default()) {
            Err(SyncViolation::VideoGap { segment: 1, gap_s }) => {
                assert!((gap_s - 0.05).abs() < 1e-9);
            }
            other => panic!("expected VideoGap, got {other:?}"),
        }
    }

    #[test]
    fn rejects_audio_short_of_a_mid_segment() {
        // 1 s video segment carrying only 0.8 s of audio, followed by
        // another segment (so it is not the lenient final one).
        let mut first = video_seg(0.0, 30, 1.0 / 30.0, true);
        let mut track = TrackSamples::default();
        for _ in 0..40 {
            track.samples.push(SampleInfo { size: 1, duration_s: 0.02, is_sync: true });
            track.data.push(0);
        }
        first.audio.push(track);
        let mut second = video_seg(1.0, 30, 1.0 / 30.0, true);
        second.audio.push(TrackSamples::default()); // empty: also skewed, but seg 0 fails first
        let segs = vec![first, second];
        let refs: Vec<&Segment> = segs.iter().collect();
        match validate_timeline(&refs, &SyncTolerances::default()) {
            Err(SyncViolation::AudioSegmentSkew { segment: 0, track: 0, skew_s }) => {
                assert!((skew_s + 0.2).abs() < 1e-9, "audio 200 ms short, got {skew_s}");
            }
            other => panic!("expected AudioSegmentSkew, got {other:?}"),
        }
    }

    #[test]
    fn final_segment_tail_may_run_short_but_not_long() {
        // Final segment: audio 200 ms short is fine (poll lag)…
        let mut seg = video_seg(0.0, 30, 1.0 / 30.0, true);
        let mut track = TrackSamples::default();
        for _ in 0..40 {
            track.samples.push(SampleInfo { size: 1, duration_s: 0.02, is_sync: true });
            track.data.push(0);
        }
        seg.audio.push(track);
        let segs = vec![seg];
        let refs: Vec<&Segment> = segs.iter().collect();
        // Total drift tolerance must absorb the 200 ms shortfall for this test.
        let tol = SyncTolerances { max_total_drift_s: 0.3, ..Default::default() };
        validate_timeline(&refs, &tol).expect("short tail tolerated");
        // …but audio LONGER than the segment is a violation even at the end.
        let mut seg2 = video_seg(0.0, 30, 1.0 / 30.0, true);
        let mut long = TrackSamples::default();
        for _ in 0..60 {
            long.samples.push(SampleInfo { size: 1, duration_s: 0.02, is_sync: true });
            long.data.push(0);
        }
        seg2.audio.push(long);
        let segs2 = vec![seg2];
        let refs2: Vec<&Segment> = segs2.iter().collect();
        assert!(matches!(
            validate_timeline(&refs2, &tol),
            Err(SyncViolation::AudioSegmentSkew { .. })
        ));
    }
}
