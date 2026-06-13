/// Build a plain ISO-BMFF box: u32 size + fourcc + payload.
pub fn mp4_box(fourcc: [u8; 4], payload: Vec<u8>) -> Vec<u8> {
    let size = 8 + payload.len() as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(&payload);
    out
}

/// Build a "full box": version byte + 24-bit flags precede the payload.
pub fn full_box(fourcc: [u8; 4], version: u8, flags: u32, payload: Vec<u8>) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + payload.len());
    p.push(version);
    p.extend_from_slice(&flags.to_be_bytes()[1..4]);
    p.extend_from_slice(&payload);
    mp4_box(fourcc, p)
}

/// Chainable big-endian byte builder for box payloads.
#[derive(Default)]
pub struct Payload(Vec<u8>);

impl Payload {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn bytes(&mut self, v: &[u8]) -> &mut Self {
        self.0.extend_from_slice(v);
        self
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_box_is_size_fourcc_payload() {
        let b = mp4_box(*b"ftyp", vec![1, 2, 3, 4]);
        assert_eq!(b.len(), 12);
        assert_eq!(&b[0..4], &12u32.to_be_bytes());
        assert_eq!(&b[4..8], b"ftyp");
        assert_eq!(&b[8..], &[1, 2, 3, 4]);
    }

    #[test]
    fn full_box_prepends_version_and_flags() {
        let b = full_box(*b"tfdt", 1, 0x000002, vec![9]);
        assert_eq!(b.len(), 13);
        assert_eq!(b[8], 1); // version
        assert_eq!(&b[9..12], &[0x00, 0x00, 0x02]); // 24-bit flags
        assert_eq!(b[12], 9);
    }

    #[test]
    fn payload_builder_emits_big_endian() {
        let mut p = Payload::new();
        p.u8(1).u16(2).u32(3).u64(4).bytes(b"ab").i32(-1);
        assert_eq!(
            p.into_vec(),
            vec![1, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 4, b'a', b'b', 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }
}
