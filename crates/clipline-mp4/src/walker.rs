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
        out.push(BoxInfo { fourcc, offset: pos, size, payload_offset: pos + header });
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
}
