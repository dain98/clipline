/// One encoded, GOP-aligned media segment (ddoc §6). `data` is opaque
/// encoded bytes (video+audio interleaved by the encode pipeline).
#[derive(Debug, Clone)]
pub struct Segment {
    /// True when the segment begins with a keyframe (IDR). Saved clips must
    /// start at such a segment so they decode cleanly.
    pub starts_with_keyframe: bool,
    /// Presentation start, seconds since recording t0.
    pub pts_start_s: f64,
    pub duration_s: f64,
    pub data: Vec<u8>,
}

impl Segment {
    pub fn pts_end_s(&self) -> f64 {
        self.pts_start_s + self.duration_s
    }
}
