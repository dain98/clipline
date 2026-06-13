/// One parsed box header. Offsets are absolute within the parsed buffer.
#[derive(Debug, Clone)]
pub struct BoxInfo {
    pub fourcc: [u8; 4],
    pub offset: u64,
    /// Total box size including header.
    pub size: u64,
    /// Absolute offset where the payload begins (8 or 16 past `offset`).
    pub payload_offset: u64,
}

/// Parse consecutive boxes starting at `buf[0]`. Stops at truncation.
pub fn walk(buf: &[u8]) -> Vec<BoxInfo> {
    walk_range(buf, 0, buf.len() as u64)
}

/// Parse the children of a pure container box (moov/trak/moof/…).
pub fn children(buf: &[u8], parent: &BoxInfo) -> Vec<BoxInfo> {
    walk_range(buf, parent.payload_offset, parent.offset + parent.size)
}

/// First box with the given fourcc, if any.
pub fn find<'a>(boxes: &'a [BoxInfo], fourcc: &[u8; 4]) -> Option<&'a BoxInfo> {
    boxes.iter().find(|b| &b.fourcc == fourcc)
}

/// Movie duration from moov/mvhd (version 0 or 1). None when the buffer
/// has no finalized moov (still-fragmented recording or foreign data).
pub fn movie_duration_s(buf: &[u8]) -> Option<f64> {
    let top = walk(buf);
    let moov = find(&top, b"moov")?.clone();
    let kids = children(buf, &moov);
    let mvhd = find(&kids, b"mvhd")?;
    let p = mvhd.payload_offset as usize;
    let version = *buf.get(p)?;
    // v0: ver/flags(4) ctime(4) mtime(4) timescale(4) duration(4)
    // v1: ver/flags(4) ctime(8) mtime(8) timescale(4) duration(8)
    let (ts_off, dur_off, dur_is_64) = match version {
        0 => (p + 12, p + 16, false),
        1 => (p + 20, p + 24, true),
        _ => return None,
    };
    let timescale = u32::from_be_bytes(buf.get(ts_off..ts_off + 4)?.try_into().ok()?) as f64;
    let duration = if dur_is_64 {
        u64::from_be_bytes(buf.get(dur_off..dur_off + 8)?.try_into().ok()?) as f64
    } else {
        u32::from_be_bytes(buf.get(dur_off..dur_off + 4)?.try_into().ok()?) as f64
    };
    (timescale > 0.0).then(|| duration / timescale)
}

fn walk_range(buf: &[u8], mut pos: u64, end: u64) -> Vec<BoxInfo> {
    let mut out = Vec::new();
    while pos + 8 <= end && (pos + 8) as usize <= buf.len() {
        let p = pos as usize;
        let size32 = u32::from_be_bytes(buf[p..p + 4].try_into().unwrap());
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&buf[p + 4..p + 8]);
        let (size, header) = if size32 == 1 {
            if (pos + 16) as usize > buf.len() {
                break;
            }
            let large = u64::from_be_bytes(buf[p + 8..p + 16].try_into().unwrap());
            (large, 16u64)
        } else if size32 == 0 {
            (end - pos, 8u64) // box extends to end
        } else {
            (size32 as u64, 8u64)
        };
        if size < header || pos + size > end {
            break; // truncated/corrupt — stop, return what we have
        }
        out.push(BoxInfo {
            fourcc,
            offset: pos,
            size,
            payload_offset: pos + header,
        });
        pos += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boxes::mp4_box;

    #[test]
    fn walks_top_level_boxes() {
        let mut buf = mp4_box(*b"ftyp", vec![0; 4]);
        buf.extend(mp4_box(*b"free", vec![0; 8]));
        let boxes = walk(&buf);
        assert_eq!(boxes.len(), 2);
        assert_eq!(&boxes[0].fourcc, b"ftyp");
        assert_eq!(boxes[0].offset, 0);
        assert_eq!(boxes[0].size, 12);
        assert_eq!(&boxes[1].fourcc, b"free");
        assert_eq!(boxes[1].offset, 12);
        assert_eq!(boxes[1].payload_offset, 12 + 8);
    }

    #[test]
    fn handles_largesize_boxes() {
        // size=1 → u64 largesize follows fourcc (16-byte header).
        let payload = vec![7u8; 4];
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(b"mdat");
        buf.extend_from_slice(&(16u64 + 4).to_be_bytes());
        buf.extend_from_slice(&payload);
        let boxes = walk(&buf);
        assert_eq!(boxes.len(), 1);
        assert_eq!(&boxes[0].fourcc, b"mdat");
        assert_eq!(boxes[0].size, 20);
        assert_eq!(boxes[0].payload_offset, 16);
    }

    #[test]
    fn children_walks_container_payload() {
        let inner = mp4_box(*b"mvhd", vec![0; 4]);
        let outer = mp4_box(*b"moov", inner);
        let top = walk(&outer);
        let kids = children(&outer, &top[0]);
        assert_eq!(kids.len(), 1);
        assert_eq!(&kids[0].fourcc, b"mvhd");
        assert_eq!(kids[0].offset, 8); // absolute within buf
    }

    #[test]
    fn find_locates_by_fourcc() {
        let buf = mp4_box(*b"ftyp", vec![]);
        let boxes = walk(&buf);
        assert!(find(&boxes, b"ftyp").is_some());
        assert!(find(&boxes, b"moov").is_none());
    }

    /// Finalized writer output with `n` samples of `dur` ticks at 90 kHz.
    fn finalized_file_with(n: u32, dur: u32) -> Vec<u8> {
        use crate::{FragSample, HybridMp4Writer, VideoTrackConfig};
        let cfg = VideoTrackConfig {
            width: 64,
            height: 64,
            timescale: 90_000,
            sps: vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
            pps: vec![0x68, 0xEE, 0x38, 0x80],
        };
        let mut w = HybridMp4Writer::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
        let samples: Vec<FragSample> = (0..n)
            .map(|i| FragSample {
                data: vec![0xAB; 8],
                duration: dur,
                is_sync: i == 0,
            })
            .collect();
        w.write_fragment(&samples).unwrap();
        w.finalize().unwrap().into_inner()
    }

    #[test]
    fn movie_duration_reads_mvhd() {
        // 60 samples × 1500 ticks at 90 kHz = exactly 1.0 s.
        let buf = finalized_file_with(60, 1_500);
        let d = movie_duration_s(&buf).expect("mvhd present");
        assert!((d - 1.0).abs() < 1e-6, "got {d}");
    }

    #[test]
    fn movie_duration_none_without_moov() {
        assert!(movie_duration_s(b"not an mp4").is_none());
        let frag_only = mp4_box(*b"ftyp", vec![0; 4]);
        assert!(movie_duration_s(&frag_only).is_none());
    }
}
