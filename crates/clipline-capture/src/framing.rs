//! Streaming access-unit framers for the FFmpeg subprocess encoder. An
//! `ffmpeg` child writes a continuous elementary stream to its stdout; these
//! split that byte stream back into per-frame access units as bytes arrive.
//! Pure state machines — platform-neutral and unit tested on every OS.
//!
//! - [`AnnexBFramer`] splits H.264/HEVC by start codes, using AUD NALs and
//!   each picture's first-slice flag to keep multi-slice pictures together.
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

/// H.264/HEVC Annex B framer. An access unit is the run of NALs belonging to
/// one picture; parameter sets / SEI that follow belong to the next unit.
pub struct AnnexBFramer {
    buf: Vec<u8>,
    /// First byte not yet proven unable to begin a three-byte start code.
    scan_pos: usize,
    /// Start of the access unit currently being accumulated.
    au_start: Option<usize>,
    /// Most recent start code; its NAL is incomplete until another arrives.
    pending_code: Option<(usize, usize)>,
    pending_classified: bool,
    current_has_vcl: bool,
    /// First non-VCL after a completed picture; prefix NALs from here belong
    /// to the next picture once its first slice arrives.
    next_au_start: Option<usize>,
    classify_nal: fn(&[u8], bool) -> Option<NalKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NalKind {
    Aud,
    Vcl { first_slice: bool },
    Other,
}

impl AnnexBFramer {
    pub fn h264() -> Self {
        Self {
            buf: Vec::new(),
            scan_pos: 0,
            au_start: None,
            pending_code: None,
            pending_classified: false,
            current_has_vcl: false,
            next_au_start: None,
            classify_nal: classify_h264_nal,
        }
    }

    pub fn hevc() -> Self {
        Self {
            buf: Vec::new(),
            scan_pos: 0,
            au_start: None,
            pending_code: None,
            pending_classified: false,
            current_has_vcl: false,
            next_au_start: None,
            classify_nal: classify_hevc_nal,
        }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.scan_pos = 0;
        self.au_start = None;
        self.pending_code = None;
        self.pending_classified = false;
        self.current_has_vcl = false;
        self.next_au_start = None;
    }

    fn apply_nal_kind(&mut self, kind: NalKind, code_start: usize, units: &mut Vec<Vec<u8>>) {
        match kind {
            NalKind::Aud => {
                if self.current_has_vcl {
                    let start = self.au_start.unwrap_or(code_start);
                    units.push(self.buf[start..code_start].to_vec());
                    self.au_start = Some(code_start);
                }
                self.current_has_vcl = false;
                self.next_au_start = None;
            }
            NalKind::Vcl { first_slice } => {
                if first_slice && self.current_has_vcl {
                    let boundary = self.next_au_start.take().unwrap_or(code_start);
                    let start = self.au_start.unwrap_or(boundary);
                    units.push(self.buf[start..boundary].to_vec());
                    self.au_start = Some(boundary);
                }
                self.current_has_vcl = true;
            }
            NalKind::Other => {
                if self.current_has_vcl && self.next_au_start.is_none() {
                    self.next_au_start = Some(code_start);
                }
            }
        }
    }

    fn classify_pending(&mut self, end: usize, complete: bool, units: &mut Vec<Vec<u8>>) {
        if self.pending_classified {
            return;
        }
        let Some((code_start, payload_start)) = self.pending_code else {
            return;
        };
        if let Some(kind) = (self.classify_nal)(
            self.buf.get(payload_start..end).unwrap_or_default(),
            complete,
        ) {
            self.apply_nal_kind(kind, code_start, units);
            self.pending_classified = true;
        }
    }

    fn scan_new_start_codes(&mut self) -> Vec<Vec<u8>> {
        let mut units = Vec::new();
        let mut i = self.scan_pos.min(self.buf.len());
        while i + 2 < self.buf.len() {
            if self.buf[i] == 0 && self.buf[i + 1] == 0 && self.buf[i + 2] == 1 {
                let code_start = if i > 0 && self.buf[i - 1] == 0 {
                    i - 1
                } else {
                    i
                };
                let payload_start = i + 3;
                if self.au_start.is_none() {
                    self.au_start = Some(code_start);
                }
                self.classify_pending(code_start, true, &mut units);
                self.pending_code = Some((code_start, payload_start));
                self.pending_classified = false;
                i += 3;
            } else {
                i += 1;
            }
        }
        self.scan_pos = i;
        self.classify_pending(self.buf.len(), false, &mut units);

        // Drop junk before the first code and bytes belonging to units that
        // were already emitted. Keep all state relative to the retained AU.
        if let Some(drain_len) = self.au_start.filter(|start| *start > 0) {
            self.buf.drain(..drain_len);
            self.scan_pos = self.scan_pos.saturating_sub(drain_len);
            self.au_start = Some(0);
            self.pending_code = self
                .pending_code
                .map(|(code, payload)| (code - drain_len, payload - drain_len));
            self.next_au_start = self.next_au_start.map(|start| start - drain_len);
        }
        units
    }
}

fn h264_is_vcl(first_byte: u8) -> bool {
    matches!(first_byte & 0x1F, 1..=5)
}

fn classify_h264_nal(nal: &[u8], complete: bool) -> Option<NalKind> {
    let Some(&header) = nal.first() else {
        return complete.then_some(NalKind::Other);
    };
    let nal_type = header & 0x1F;
    if nal_type == 9 {
        return Some(NalKind::Aud);
    }
    if h264_is_vcl(header) {
        // first_mb_in_slice is the first unsigned Exp-Golomb value in the
        // slice header. Its value is zero exactly when its first bit is one.
        let first_slice = match nal.get(1) {
            Some(byte) => byte & 0x80 != 0,
            None if complete => true,
            None => return None,
        };
        return Some(NalKind::Vcl { first_slice });
    }
    Some(NalKind::Other)
}

fn hevc_is_vcl(first_byte: u8) -> bool {
    ((first_byte >> 1) & 0x3F) <= 31
}

fn classify_hevc_nal(nal: &[u8], complete: bool) -> Option<NalKind> {
    let Some(&header) = nal.first() else {
        return complete.then_some(NalKind::Other);
    };
    let nal_type = (header >> 1) & 0x3F;
    if nal_type == 35 {
        return Some(NalKind::Aud);
    }
    if hevc_is_vcl(header) {
        // The first slice-header bit after HEVC's two-byte NAL header is
        // first_slice_segment_in_pic_flag.
        let first_slice = match nal.get(2) {
            Some(byte) => byte & 0x80 != 0,
            None if complete => true,
            None => return None,
        };
        return Some(NalKind::Vcl { first_slice });
    }
    Some(NalKind::Other)
}

impl AccessUnitFramer for AnnexBFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        if self
            .buf
            .len()
            .checked_add(bytes.len())
            .is_none_or(|total| total > MAX_FRAMER_BUFFER)
        {
            self.reset();
            return Vec::new();
        }
        self.buf.extend_from_slice(bytes);
        self.scan_new_start_codes()
    }

    fn flush(&mut self) -> Option<Vec<u8>> {
        if self.buf.is_empty() {
            return None;
        }
        let final_unit = std::mem::take(&mut self.buf);
        self.scan_pos = 0;
        self.au_start = None;
        self.pending_code = None;
        self.pending_classified = false;
        self.current_has_vcl = false;
        self.next_au_start = None;
        Some(final_unit)
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
        stream.extend(sc4(&[0x65, 0x80, 3])); // IDR first slice
        stream.extend(sc4(&[0x41, 0x80, 4])); // P first slice
        stream.extend(sc4(&[0x41, 0x80, 5])); // P first slice (not terminated)
        let units = f.push(&stream);
        assert_eq!(units.len(), 2, "third slice waits for the next start code");
        // AU #1 carries SPS+PPS+IDR; AU #2 is the lone P slice.
        assert_eq!(
            units[0],
            [sc4(&[0x67, 1]), sc4(&[0x68, 2]), sc4(&[0x65, 0x80, 3]),].concat()
        );
        assert_eq!(units[1], sc4(&[0x41, 0x80, 4]));
        // flush releases the still-buffered final slice.
        assert_eq!(f.flush(), Some(sc4(&[0x41, 0x80, 5])));
    }

    #[test]
    fn annexb_reassembles_across_chunk_boundaries() {
        let mut f = AnnexBFramer::h264();
        let mut stream = Vec::new();
        stream.extend(sc4(&[0x65, 0x80, 0xAA, 0xBB])); // IDR first slice
        stream.extend(sc4(&[0x41, 0x80, 0xCC])); // P first slice
                                                 // Split mid-NAL to exercise the streaming buffer.
        let mut out = f.push(&stream[..6]);
        out.extend(f.push(&stream[6..]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], sc4(&[0x65, 0x80, 0xAA, 0xBB]));
    }

    #[test]
    fn annexb_split_start_codes_are_detected_at_every_boundary() {
        let stream = [sc4(&[0x65, 0x80, 0xAA]), sc4(&[0x41, 0x80, 0xBB])].concat();
        for split in 1..stream.len() {
            let mut framer = AnnexBFramer::h264();
            let mut out = framer.push(&stream[..split]);
            out.extend(framer.push(&stream[split..]));
            assert_eq!(out, vec![sc4(&[0x65, 0x80, 0xAA])], "split at {split}");
        }
    }

    #[test]
    fn annexb_delimiter_free_input_is_scanned_incrementally_and_bounded() {
        let mut framer = AnnexBFramer::h264();
        let chunk = vec![0xFF; 1024];
        for expected_chunks in 1..=32 {
            assert!(framer.push(&chunk).is_empty());
            assert!(
                framer.scan_pos >= expected_chunks * chunk.len() - 2,
                "cursor should remain at the unscanned suffix"
            );
        }

        assert!(framer
            .push(&vec![0xFF; MAX_FRAMER_BUFFER - framer.buf.len() + 1])
            .is_empty());
        assert!(framer.buf.is_empty(), "oversized generation is discarded");
        assert_eq!(framer.scan_pos, 0);
    }

    #[test]
    fn annexb_reset_does_not_merge_discarded_suffix_into_a_start_code() {
        let mut framer = AnnexBFramer::h264();
        let mut malformed = vec![0xFF; MAX_FRAMER_BUFFER + 1];
        let end = malformed.len();
        malformed[end - 2..].copy_from_slice(&[0, 0]);
        assert!(framer.push(&malformed).is_empty());

        // These bytes would complete a start code only if the discarded zero
        // suffix survived the reset.
        assert!(framer.push(&[1, 0x65, 0x80, 0xAA]).is_empty());
        let valid = [sc4(&[0x65, 0x80, 0xBB]), sc4(&[0x41, 0x80, 0xCC])].concat();
        let out = framer.push(&valid);

        assert_eq!(out, vec![sc4(&[0x65, 0x80, 0xBB])]);
    }

    #[test]
    fn hevc_vcl_predicate_matches_slice_types() {
        // HEVC type = (byte>>1)&0x3F: 0x26 → 19 (IDR, VCL); 0x40 → 32 (VPS).
        assert!(hevc_is_vcl(0x26));
        assert!(!hevc_is_vcl(0x40));
    }

    #[test]
    fn h264_multislice_picture_is_one_access_unit() {
        let mut framer = AnnexBFramer::h264();
        let first_picture = [
            sc4(&[0x67, 1]),
            sc4(&[0x65, 0x80, 2]), // first_mb_in_slice = 0
            sc4(&[0x65, 0x40, 3]), // first_mb_in_slice > 0
        ]
        .concat();
        let between = sc4(&[0x06, 4]); // SEI belongs to the next AU
        let second = sc4(&[0x41, 0x80, 5]);
        let third = sc4(&[0x41, 0x80, 6]);

        let units = framer.push(
            &[
                first_picture.clone(),
                between.clone(),
                second.clone(),
                third,
            ]
            .concat(),
        );

        assert_eq!(units.len(), 2);
        assert_eq!(units[0], first_picture);
        assert_eq!(units[1], [between, second].concat());
    }

    #[test]
    fn hevc_multislice_picture_and_aud_form_picture_boundaries() {
        let mut framer = AnnexBFramer::hevc();
        let first_picture = [
            sc4(&[0x40, 0x01, 1]),
            sc4(&[0x26, 0x01, 0x80, 2]), // first_slice_segment_in_pic_flag
            sc4(&[0x26, 0x01, 0x00, 3]), // continuation slice
        ]
        .concat();
        let aud = sc4(&[35 << 1, 0x01, 4]);
        let second = sc4(&[0x02, 0x01, 0x80, 5]);
        let third = sc4(&[0x02, 0x01, 0x80, 6]);

        let units =
            framer.push(&[first_picture.clone(), aud.clone(), second.clone(), third].concat());

        assert_eq!(units.len(), 2);
        assert_eq!(units[0], first_picture);
        assert_eq!(units[1], [aud, second].concat());
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
