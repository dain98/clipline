/// Metadata for one encoded sample inside a segment's `data`.
#[derive(Debug, Clone, Copy)]
pub struct SampleInfo {
    /// Byte length within `Segment::data`.
    pub size: u32,
    pub duration_s: f64,
    pub is_sync: bool,
}

/// One encoded, GOP-aligned media segment (ddoc §6). `data` is the opaque
/// concatenation of encoded samples; `samples` indexes it so a saved
/// window can be sliced back into muxer samples.
#[derive(Debug, Clone)]
pub struct Segment {
    /// True when the segment begins with a keyframe (IDR). Saved clips must
    /// start at such a segment so they decode cleanly.
    pub starts_with_keyframe: bool,
    /// Presentation start, seconds since recording t0.
    pub pts_start_s: f64,
    pub duration_s: f64,
    pub data: Vec<u8>,
    pub samples: Vec<SampleInfo>,
}

impl Segment {
    pub fn pts_end_s(&self) -> f64 {
        self.pts_start_s + self.duration_s
    }

    /// Iterate `data` sliced per the sample index.
    pub fn sample_slices(&self) -> impl Iterator<Item = &[u8]> {
        let mut offset = 0usize;
        self.samples.iter().map(move |s| {
            let start = offset;
            offset += s.size as usize;
            &self.data[start..offset]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_index_slices_data_back_into_samples() {
        let seg = Segment {
            starts_with_keyframe: true,
            pts_start_s: 0.0,
            duration_s: 0.1,
            data: b"AAAABBBCC".to_vec(),
            samples: vec![
                SampleInfo { size: 4, duration_s: 0.04, is_sync: true },
                SampleInfo { size: 3, duration_s: 0.03, is_sync: false },
                SampleInfo { size: 2, duration_s: 0.03, is_sync: false },
            ],
        };
        let slices: Vec<&[u8]> = seg.sample_slices().collect();
        assert_eq!(slices, vec![b"AAAA".as_slice(), b"BBB".as_slice(), b"CC".as_slice()]);
    }
}
