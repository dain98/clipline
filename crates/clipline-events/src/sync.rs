use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSyncError {
    AnchorBeforeRecordingStart,
}

impl std::fmt::Display for ClockSyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AnchorBeforeRecordingStart => {
                formatter.write_str("clock anchor predates recording start")
            }
        }
    }
}

impl std::error::Error for ClockSyncError {}

/// A paired sample of the source game clock and the wall clock.
///
/// Re-sample one of these on every poll (ddoc §5): pauses and drift in the
/// game clock then self-correct, and already-placed markers are never
/// re-mapped. `sampled_at` must not be earlier than the recording's `t0`.
#[derive(Debug, Clone, Copy)]
pub struct ClockAnchor {
    /// Game clock (seconds since GameStart) at the moment of sampling.
    pub game_time_s: f64,
    /// Wall clock at the moment of sampling.
    pub sampled_at: Instant,
}

/// Map a source event time onto the recording timeline (ddoc §5):
/// `offset = (EventTime − anchor.gameTime) + (anchor.wall − t0) − latency`.
///
/// `emit_latency_s` is the small fixed kill-feed/event-emit delay that
/// nudges markers onto the visual moment. The result can be negative for
/// events that predate the recording; callers clamp as appropriate. An anchor
/// sampled before `recording_t0` is rejected rather than silently saturated.
pub fn recording_offset_s(
    event_time_s: f64,
    anchor: ClockAnchor,
    recording_t0: Instant,
    emit_latency_s: f64,
) -> Result<f64, ClockSyncError> {
    let recording_elapsed = anchor
        .sampled_at
        .checked_duration_since(recording_t0)
        .ok_or(ClockSyncError::AnchorBeforeRecordingStart)?;
    Ok((event_time_s - anchor.game_time_s) + recording_elapsed.as_secs_f64() - emit_latency_s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn maps_event_time_onto_recording_timeline() {
        // Recording starts at t0. 100s later we poll: game clock reads 95s
        // (game started 5s after the recording). An event at EventTime=90
        // therefore happened at recording offset 95s.
        let t0 = Instant::now();
        let anchor = ClockAnchor {
            game_time_s: 95.0,
            sampled_at: t0 + Duration::from_secs(100),
        };
        let off = recording_offset_s(90.0, anchor, t0, 0.0).unwrap();
        assert!((off - 95.0).abs() < 1e-9);
    }

    #[test]
    fn resampled_anchor_self_corrects_after_game_clock_pause() {
        // Game pauses for 60s at game_time=200. After the pause, wall time
        // has advanced 60s more than game time. A fresh anchor sampled at
        // wall t0+360 reads game_time=300; an event at EventTime=310 lands
        // at recording offset 370 — pause absorbed with no special-casing.
        let t0 = Instant::now();
        let post_pause_anchor = ClockAnchor {
            game_time_s: 300.0,
            sampled_at: t0 + Duration::from_secs(360),
        };
        let off = recording_offset_s(310.0, post_pause_anchor, t0, 0.0).unwrap();
        assert!((off - 370.0).abs() < 1e-9);
    }

    #[test]
    fn emit_latency_nudges_marker_earlier() {
        let t0 = Instant::now();
        let anchor = ClockAnchor {
            game_time_s: 10.0,
            sampled_at: t0 + Duration::from_secs(10),
        };
        let off = recording_offset_s(10.0, anchor, t0, 0.5).unwrap();
        assert!((off - 9.5).abs() < 1e-9);
    }

    #[test]
    fn anchor_before_recording_start_is_rejected() {
        let sampled_at = Instant::now();
        let recording_t0 = sampled_at + Duration::from_secs(1);
        let anchor = ClockAnchor {
            game_time_s: 10.0,
            sampled_at,
        };

        assert_eq!(
            recording_offset_s(10.0, anchor, recording_t0, 0.0),
            Err(ClockSyncError::AnchorBeforeRecordingStart)
        );
    }
}
