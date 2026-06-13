//! Streaming access-unit framers for the FFmpeg subprocess encoder. An
//! `ffmpeg` child writes a continuous elementary stream to its stdout; these
//! split that byte stream back into per-frame access units as bytes arrive.
//! Pure state machines — platform-neutral and unit tested on every OS.
//!
//! - [`AnnexBFramer`] splits H.264/HEVC by start codes, ending an access
//!   unit at each VCL (slice) NAL. It assumes one slice per picture (the
//!   default for Clipline's hardware encoders at replay resolutions).
//! - [`IvfFramer`] reads the IVF container FFmpeg wraps AV1 in, yielding one
//!   temporal unit per IVF frame.

/// A stateful framer: feed stdout bytes, get complete access units; `flush`
/// releases the final unit at end of stream (no trailing delimiter).
pub trait AccessUnitFramer: Send {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>>;
    fn flush(&mut self) -> Option<Vec<u8>>;
}

/// Hard ceiling on a framer's pending buffer. A single access unit / temporal
/// unit at Clipline's resolutions and bitrates is well under this; exceeding it
/// means the subprocess output is malformed (no NAL boundary, or a bogus IVF
/// frame length), so the buffer is dropped to avoid unbounded growth on the
/// long-lived reader thread (an availability guard, not a normal path).
const MAX_FRAMER_BUFFER: usize = 32 * 1024 * 1024;

/// H.264/HEVC Annex B framer. An access unit is the run of NALs ending at a
/// VCL NAL; parameter sets / SEI that follow belong to the next unit.
pub struct AnnexBFramer {
    buf: Vec<u8>,
    /// True if the NAL (given its first payload byte) is a VCL/slice NAL.
    is_vcl: fn(u8) -> bool,
}

impl AnnexBFramer {
    pub fn h264() -> Self {
        Self {
            buf: Vec::new(),
            is_vcl: h264_is_vcl,
        }
    }

    pub fn hevc() -> Self {
        Self {
            buf: Vec::new(),
            is_vcl: hevc_is_vcl,
        }
    }
}

fn h264_is_vcl(first_byte: u8) -> bool {
    matches!(first_byte & 0x1F, 1..=5)
}

fn hevc_is_vcl(first_byte: u8) -> bool {
    ((first_byte >> 1) & 0x3F) <= 31
}

/// Start-code scan: each entry is (code_start, payload_start), handling both
/// 3- and 4-byte start codes (a 4-byte code is a 3-byte code preceded by 0).
fn start_codes(buf: &[u8]) -> Vec<(usize, usize)> {
    let mut codes = Vec::new();
    let mut i = 0;
    while i + 2 < buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            let code_start = if i > 0 && buf[i - 1] == 0 { i - 1 } else { i };
            codes.push((code_start, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    codes
}

impl AccessUnitFramer for AnnexBFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let codes = start_codes(&self.buf);
        let mut units = Vec::new();
        // The current access unit begins at the first start code in the buffer.
        let mut au_start = match codes.first() {
            Some(&(code_start, _)) => code_start,
            None => return units,
        };
        // A NAL is complete only once the *next* start code is in the buffer,
        // so we never frame a half-received slice. Walk complete NALs.
        for w in codes.windows(2) {
            let (_, payload_start) = w[0];
            let (next_code_start, _) = w[1];
            let first_byte = self.buf.get(payload_start).copied().unwrap_or(0);
            if (self.is_vcl)(first_byte) {
                units.push(self.buf[au_start..next_code_start].to_vec());
                au_start = next_code_start;
            }
        }
        if au_start > 0 {
            self.buf.drain(..au_start);
        }
        if self.buf.len() > MAX_FRAMER_BUFFER {
            self.buf.clear(); // malformed: no AU boundary in a huge window
        }
        units
    }

    fn flush(&mut self) -> Option<Vec<u8>> {
        if self.buf.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.buf))
    }
}

const IVF_FILE_HEADER_LEN: usize = 32;
const IVF_FRAME_HEADER_LEN: usize = 12;

/// IVF container framer for AV1: one temporal unit per IVF frame.
pub struct IvfFramer {
    buf: Vec<u8>,
    header_consumed: bool,
}

impl IvfFramer {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            header_consumed: false,
        }
    }
}

impl Default for IvfFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl AccessUnitFramer for IvfFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut units = Vec::new();
        if !self.header_consumed {
            if self.buf.len() < IVF_FILE_HEADER_LEN {
                return units;
            }
            self.buf.drain(..IVF_FILE_HEADER_LEN);
            self.header_consumed = true;
        }
        loop {
            if self.buf.len() < IVF_FRAME_HEADER_LEN {
                break;
            }
            let size = u32::from_le_bytes(self.buf[0..4].try_into().unwrap()) as usize;
            if size > MAX_FRAMER_BUFFER {
                self.buf.clear(); // bogus frame length: malformed, can't resync
                break;
            }
            let total = IVF_FRAME_HEADER_LEN + size;
            if self.buf.len() < total {
                break;
            }
            units.push(self.buf[IVF_FRAME_HEADER_LEN..total].to_vec());
            self.buf.drain(..total);
        }
        units
    }

    fn flush(&mut self) -> Option<Vec<u8>> {
        None // IVF frames are length-prefixed; no trailing partial unit.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc4(nal: &[u8]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 1];
        v.extend_from_slice(nal);
        v
    }

    #[test]
    fn annexb_frames_one_access_unit_per_vcl() {
        let mut f = AnnexBFramer::h264();
        // SPS(7) PPS(8) IDR(5) | non-IDR(1) | non-IDR(1) — two more start
        // codes than units so the trailing slice stays buffered.
        let mut stream = Vec::new();
        stream.extend(sc4(&[0x67, 1]));
        stream.extend(sc4(&[0x68, 2]));
        stream.extend(sc4(&[0x65, 3])); // IDR → ends AU #1
        stream.extend(sc4(&[0x41, 4])); // P   → ends AU #2
        stream.extend(sc4(&[0x41, 5])); // P   (not yet terminated)
        let units = f.push(&stream);
        assert_eq!(units.len(), 2, "third slice waits for the next start code");
        // AU #1 carries SPS+PPS+IDR; AU #2 is the lone P slice.
        assert_eq!(
            units[0],
            [sc4(&[0x67, 1]), sc4(&[0x68, 2]), sc4(&[0x65, 3])].concat()
        );
        assert_eq!(units[1], sc4(&[0x41, 4]));
        // flush releases the still-buffered final slice.
        assert_eq!(f.flush(), Some(sc4(&[0x41, 5])));
    }

    #[test]
    fn annexb_reassembles_across_chunk_boundaries() {
        let mut f = AnnexBFramer::h264();
        let mut stream = Vec::new();
        stream.extend(sc4(&[0x65, 0xAA, 0xBB])); // IDR
        stream.extend(sc4(&[0x41, 0xCC])); // P terminates the IDR's AU
                                           // Split mid-NAL to exercise the streaming buffer.
        let mut out = f.push(&stream[..6]);
        out.extend(f.push(&stream[6..]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], sc4(&[0x65, 0xAA, 0xBB]));
    }

    #[test]
    fn hevc_vcl_predicate_matches_slice_types() {
        // HEVC type = (byte>>1)&0x3F: 0x26 → 19 (IDR, VCL); 0x40 → 32 (VPS).
        assert!(hevc_is_vcl(0x26));
        assert!(!hevc_is_vcl(0x40));
    }

    fn ivf_file_header() -> Vec<u8> {
        let mut h = vec![0u8; IVF_FILE_HEADER_LEN];
        h[0..4].copy_from_slice(b"DKIF");
        h
    }

    fn ivf_frame(payload: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        f.extend_from_slice(&0u64.to_le_bytes()); // timestamp (unused)
        f.extend_from_slice(payload);
        f
    }

    #[test]
    fn ivf_yields_one_unit_per_frame_skipping_the_file_header() {
        let mut f = IvfFramer::new();
        let mut stream = ivf_file_header();
        stream.extend(ivf_frame(&[0xAA, 0xBB]));
        stream.extend(ivf_frame(&[0xCC]));
        let units = f.push(&stream);
        assert_eq!(units, vec![vec![0xAA, 0xBB], vec![0xCC]]);
    }

    #[test]
    fn ivf_buffers_partial_frames_across_chunks() {
        let mut f = IvfFramer::new();
        let mut stream = ivf_file_header();
        stream.extend(ivf_frame(&[1, 2, 3, 4]));
        // Header + part of the frame header first.
        let mut out = f.push(&stream[..IVF_FILE_HEADER_LEN + 6]);
        assert!(out.is_empty(), "frame not complete yet");
        out.extend(f.push(&stream[IVF_FILE_HEADER_LEN + 6..]));
        assert_eq!(out, vec![vec![1, 2, 3, 4]]);
    }
}
