/// Estimated RAM for a replay buffer of `duration_s` at `bitrate_bps`
/// (ddoc §6/§7). CBR makes this exact; CQP/VBR makes it an upper-ish bound
/// the UI presents before suggesting disk-spill.
pub fn estimate_buffer_bytes(bitrate_bps: u64, duration_s: f64) -> u64 {
    ((bitrate_bps as f64 / 8.0) * duration_s) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ddoc_example() {
        // ddoc §7: 1080p60 @ ~40 Mbps ≈ ~1.5 GB for 5 min.
        assert_eq!(estimate_buffer_bytes(40_000_000, 300.0), 1_500_000_000);
    }

    #[test]
    fn zero_duration_is_zero() {
        assert_eq!(estimate_buffer_bytes(40_000_000, 0.0), 0);
    }
}
