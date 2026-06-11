//! Capture timing (ddoc §6 "Clocking & A/V sync"): all capture clocks are
//! expressed as 100 ns ticks on the QPC timebase (WGC `SystemRelativeTime`
//! and WASAPI QPC positions both arrive in those units) and diffed against
//! one capture-start origin to produce `pts_s`.

/// Maps absolute 100 ns ticks to seconds since a fixed origin.
#[derive(Debug, Clone, Copy)]
pub struct RelativeClock {
    origin_ticks_100ns: i64,
}

impl RelativeClock {
    pub fn new(origin_ticks_100ns: i64) -> Self {
        Self { origin_ticks_100ns }
    }

    /// Seconds since the origin; ticks before the origin clamp to 0.0
    /// (a frame already in flight when capture started).
    pub fn pts_s(&self, ticks_100ns: i64) -> f64 {
        (ticks_100ns - self.origin_ticks_100ns).max(0) as f64 / 1e7
    }
}

/// Convert a raw QPC counter reading to 100 ns ticks. Widens to i128 so
/// `counter * 10^7` cannot overflow at large uptimes.
pub fn qpc_to_ticks_100ns(counter: i64, frequency: i64) -> i64 {
    (counter as i128 * 10_000_000 / frequency as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_zero() {
        let clock = RelativeClock::new(5_000_000);
        assert_eq!(clock.pts_s(5_000_000), 0.0);
    }

    #[test]
    fn ticks_after_origin_convert_at_100ns_per_tick() {
        let clock = RelativeClock::new(1_000);
        // 15_000_000 ticks of 100 ns = 1.5 s.
        assert!((clock.pts_s(1_000 + 15_000_000) - 1.5).abs() < 1e-12);
    }

    #[test]
    fn pre_origin_ticks_clamp_to_zero() {
        // A frame stamped before capture start (in-flight at session start)
        // must not produce a negative pts.
        let clock = RelativeClock::new(10_000_000);
        assert_eq!(clock.pts_s(9_999_999), 0.0);
    }

    #[test]
    fn qpc_converts_to_100ns_ticks() {
        // 10 MHz QPC frequency (the common modern value): 1 count = 100 ns.
        assert_eq!(qpc_to_ticks_100ns(123_456_789, 10_000_000), 123_456_789);
        // 3 MHz: 3_000_000 counts = 1 s = 10_000_000 ticks.
        assert_eq!(qpc_to_ticks_100ns(3_000_000, 3_000_000), 10_000_000);
    }

    #[test]
    fn qpc_conversion_does_not_overflow_large_uptimes() {
        // ~30 days of uptime at 10 MHz: counter * 10^7 overflows i64 — the
        // conversion must widen internally.
        let counter = 30 * 24 * 3600 * 10_000_000_i64;
        assert_eq!(qpc_to_ticks_100ns(counter, 10_000_000), counter);
    }
}
