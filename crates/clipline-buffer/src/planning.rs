use std::collections::VecDeque;
use std::sync::Arc;

use crate::{DiskSegment, Segment};

pub(crate) trait ReplayWindowSegment {
    fn starts_with_keyframe(&self) -> bool;
    fn pts_start_s(&self) -> f64;
    fn pts_end_s(&self) -> f64;
}

impl<T: ReplayWindowSegment> ReplayWindowSegment for Arc<T> {
    fn starts_with_keyframe(&self) -> bool {
        self.as_ref().starts_with_keyframe()
    }

    fn pts_start_s(&self) -> f64 {
        self.as_ref().pts_start_s()
    }

    fn pts_end_s(&self) -> f64 {
        self.as_ref().pts_end_s()
    }
}

impl ReplayWindowSegment for Segment {
    fn starts_with_keyframe(&self) -> bool {
        self.starts_with_keyframe
    }

    fn pts_start_s(&self) -> f64 {
        self.pts_start_s
    }

    fn pts_end_s(&self) -> f64 {
        self.pts_end_s()
    }
}

impl ReplayWindowSegment for DiskSegment {
    fn starts_with_keyframe(&self) -> bool {
        self.starts_with_keyframe
    }

    fn pts_start_s(&self) -> f64 {
        self.pts_start_s
    }

    fn pts_end_s(&self) -> f64 {
        self.pts_end_s()
    }
}

pub(crate) fn replay_window_start_index<T: ReplayWindowSegment>(
    segments: &VecDeque<T>,
    window_s: f64,
    exclude_before_s: Option<f64>,
) -> Option<usize> {
    let last = segments.back()?;
    let mut start_target = last.pts_end_s() - window_s;
    if let Some(exclude) = exclude_before_s {
        start_target = start_target.max(exclude);
    }

    let mut start_idx = segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| {
            segment.starts_with_keyframe() && segment.pts_start_s() <= start_target
        })
        .map(|(index, _)| index)
        .next_back()
        .or_else(|| {
            segments
                .iter()
                .position(ReplayWindowSegment::starts_with_keyframe)
        })?;

    if let Some(exclude) = exclude_before_s {
        while start_idx < segments.len() && segments[start_idx].pts_end_s() <= exclude {
            start_idx += 1;
        }
        while start_idx < segments.len() && !segments[start_idx].starts_with_keyframe() {
            start_idx += 1;
        }
    }
    (start_idx < segments.len()).then_some(start_idx)
}

pub(crate) fn eviction_count(
    existing_sizes: impl IntoIterator<Item = usize>,
    current_bytes: usize,
    incoming_bytes: usize,
    max_bytes: usize,
) -> usize {
    let mut committed = current_bytes.saturating_add(incoming_bytes);
    let mut count = 0;
    for size in existing_sizes {
        if committed <= max_bytes {
            break;
        }
        committed = committed.saturating_sub(size);
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_never_discards_the_incoming_segment() {
        assert_eq!(eviction_count([], 0, 100, 10), 0);
        assert_eq!(eviction_count([40, 40], 80, 100, 10), 2);
        assert_eq!(eviction_count([40, 40], 80, 10, 50), 1);
    }
}
