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
}
