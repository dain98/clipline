use std::time::Instant;

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
/// events that predate the recording; callers clamp as appropriate.
pub fn recording_offset_s(
    event_time_s: f64,
    anchor: ClockAnchor,
    recording_t0: Instant,
    emit_latency_s: f64,
) -> f64 {
    (event_time_s - anchor.game_time_s)
        + anchor.sampled_at.duration_since(recording_t0).as_secs_f64()
        - emit_latency_s
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
        let off = recording_offset_s(90.0, anchor, t0, 0.0);
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
        let off = recording_offset_s(310.0, post_pause_anchor, t0, 0.0);
        assert!((off - 370.0).abs() < 1e-9);
    }

    #[test]
    fn emit_latency_nudges_marker_earlier() {
        let t0 = Instant::now();
        let anchor = ClockAnchor {
            game_time_s: 10.0,
            sampled_at: t0 + Duration::from_secs(10),
        };
        let off = recording_offset_s(10.0, anchor, t0, 0.5);
        assert!((off - 9.5).abs() < 1e-9);
    }
}
