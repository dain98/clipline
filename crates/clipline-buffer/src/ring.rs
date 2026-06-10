use std::collections::VecDeque;

use crate::segment::Segment;

/// Byte-budgeted ring of encoded segments (ddoc §6). Eviction is
/// oldest-first and whole-segment; segments are GOP-aligned so dropping
/// from the front never strands a partial GOP.
#[derive(Debug)]
pub struct ReplayRing {
    max_bytes: usize,
    segments: VecDeque<Segment>,
    bytes: usize,
}

impl ReplayRing {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes, segments: VecDeque::new(), bytes: 0 }
    }

    pub fn push(&mut self, seg: Segment) {
        self.bytes += seg.data.len();
        self.segments.push_back(seg);
        while self.bytes > self.max_bytes && self.segments.len() > 1 {
            if let Some(front) = self.segments.pop_front() {
                self.bytes -= front.data.len();
            }
        }
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter()
    }

    /// Segments for a Save Replay of the trailing `window_s` seconds
    /// (ddoc §6): starts at the latest keyframe segment at-or-before
    /// `end − window` so the clip decodes cleanly and covers the window.
    ///
    /// `exclude_before_s` is the smart no-overlap mode: footage at or
    /// before that point (the previous save's end) is never re-included.
    pub fn save_window(&self, window_s: f64, exclude_before_s: Option<f64>) -> Vec<&Segment> {
        let Some(last) = self.segments.back() else {
            return Vec::new();
        };
        let mut start_target = last.pts_end_s() - window_s;
        if let Some(x) = exclude_before_s {
            start_target = start_target.max(x);
        }

        // Latest keyframe segment starting at or before the target…
        let mut start_idx = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.starts_with_keyframe && s.pts_start_s <= start_target)
            .map(|(i, _)| i)
            .next_back();
        // …or, if the buffer is shorter than the window, the first keyframe.
        if start_idx.is_none() {
            start_idx = self.segments.iter().position(|s| s.starts_with_keyframe);
        }
        let Some(mut idx) = start_idx else {
            return Vec::new();
        };

        // Smart mode: drop segments wholly at/before the exclusion point,
        // then re-align to the next keyframe.
        if let Some(x) = exclude_before_s {
            while idx < self.segments.len() && self.segments[idx].pts_end_s() <= x {
                idx += 1;
            }
            while idx < self.segments.len() && !self.segments[idx].starts_with_keyframe {
                idx += 1;
            }
        }

        self.segments.iter().skip(idx).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::Segment;

    fn seg(pts: f64, dur: f64, bytes: usize, key: bool) -> Segment {
        Segment {
            starts_with_keyframe: key,
            pts_start_s: pts,
            duration_s: dur,
            data: vec![0u8; bytes],
        }
    }

    #[test]
    fn evicts_oldest_when_over_byte_budget() {
        let mut ring = ReplayRing::new(250);
        ring.push(seg(0.0, 2.0, 100, true));
        ring.push(seg(2.0, 2.0, 100, true));
        ring.push(seg(4.0, 2.0, 100, true)); // 300 bytes > 250 → evict front
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.bytes(), 200);
        assert_eq!(ring.segments().next().unwrap().pts_start_s, 2.0);
    }

    #[test]
    fn never_evicts_the_only_segment() {
        let mut ring = ReplayRing::new(10);
        ring.push(seg(0.0, 2.0, 100, true)); // oversized but alone
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn save_window_starts_at_covering_keyframe() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, true));
        ring.push(seg(4.0, 2.0, 10, true));
        // Window of 3s from end (6.0) → target 3.0, covered by seg@2.0.
        let saved = ring.save_window(3.0, None);
        let starts: Vec<f64> = saved.iter().map(|s| s.pts_start_s).collect();
        assert_eq!(starts, vec![2.0, 4.0]);
    }

    #[test]
    fn save_window_skips_non_keyframe_lead_in() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, false)); // continuation of GOP at 0.0
        ring.push(seg(4.0, 2.0, 10, true));
        // Target 3.0: latest keyframe at/before is 0.0 → include from 0.0
        // so the clip covers the full window and starts decodable.
        let saved = ring.save_window(3.0, None);
        assert_eq!(saved[0].pts_start_s, 0.0);
        assert_eq!(saved.len(), 3);
    }

    #[test]
    fn smart_mode_never_resaves_already_saved_footage() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, true));
        ring.push(seg(4.0, 2.0, 10, true));
        // Previous save consumed up to t=4.0 → only the last segment now.
        let saved = ring.save_window(6.0, Some(4.0));
        let starts: Vec<f64> = saved.iter().map(|s| s.pts_start_s).collect();
        assert_eq!(starts, vec![4.0]);
    }

    #[test]
    fn save_window_on_empty_ring_is_empty() {
        let ring = ReplayRing::new(100);
        assert!(ring.save_window(5.0, None).is_empty());
    }
}
