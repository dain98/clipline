//! H.264 Annex B ↔ MP4 (AVCC) stream-format conversion. clipline-mp4
//! writes avcC with 4-byte length prefixes; MFTs emit Annex B start codes
//! (handoff "sharp edges"). Pure byte math — platform-neutral and unit
//! tested on every OS.

/// Split an Annex B stream into NAL units (start codes removed). Handles
/// 3- and 4-byte start codes; bytes before the first start code are
/// ignored (there is no NAL to attribute them to).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new(); // (payload_start, code_start)
    let mut i = 0;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            let code_start = if i > 0 && data[i - 1] == 0 { i - 1 } else { i };
            starts.push((i + 3, code_start));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (idx, &(payload, _)) in starts.iter().enumerate() {
        let end = starts
            .get(idx + 1)
            .map(|&(_, code)| code)
            .unwrap_or(data.len());
        if payload < end {
            units.push(&data[payload..end]);
        }
    }
    units
}

/// H.264 nal_unit_type (low 5 bits of the first byte).
/// 1 = non-IDR slice, 5 = IDR, 6 = SEI, 7 = SPS, 8 = PPS, 9 = AUD.
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1F).unwrap_or(0)
}

/// Classify an H.264 access unit from the encoded bitstream itself. Hardware
/// MFTs do not consistently attach `MFSampleExtension_CleanPoint` to every
/// IDR, so callers must be able to recognize the authoritative NAL type.
pub fn is_keyframe(annexb: &[u8]) -> bool {
    split_annexb(annexb).iter().any(|unit| nal_type(unit) == 5)
}

/// Pull (SPS, PPS) out of an Annex B blob — works on
/// `MF_MT_MPEG_SEQUENCE_HEADER` and on full access units with in-band
/// parameter sets.
pub fn extract_sps_pps(annexb: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let units = split_annexb(annexb);
    let sps = units.iter().find(|u| nal_type(u) == 7)?;
    let pps = units.iter().find(|u| nal_type(u) == 8)?;
    Some((sps.to_vec(), pps.to_vec()))
}

/// Convert one Annex B access unit to 4-byte-length-prefixed AVCC sample
/// data. AUD/SPS/PPS are dropped: parameter sets travel in avcC, and MP4
/// needs no access-unit delimiters.
pub fn annexb_to_avcc(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for unit in split_annexb(annexb) {
        if matches!(nal_type(unit), 7..=9) {
            continue;
        }
        out.extend_from_slice(&(unit.len() as u32).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

/// Encoders (and NV12 itself) need even dimensions; round down, floor 2.
pub fn even_dimensions(width: u32, height: u32) -> (u32, u32) {
    ((width & !1).max(2), (height & !1).max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tiny but structurally-real NALs: type byte (nal_ref_idc<<5 | type) + payload.
    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x0A, 0xAC];
    const PPS: &[u8] = &[0x68, 0xEE, 0x38, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x33];
    const NON_IDR: &[u8] = &[0x41, 0x9A, 0x02];
    const SEI: &[u8] = &[0x06, 0x05, 0x04];
    const AUD: &[u8] = &[0x09, 0x10];

    fn annexb(units: &[&[u8]]) -> Vec<u8> {
        // Alternate 4-byte and 3-byte start codes to exercise both.
        let mut out = Vec::new();
        for (i, u) in units.iter().enumerate() {
            out.extend_from_slice(if i % 2 == 0 {
                &[0, 0, 0, 1][..]
            } else {
                &[0, 0, 1][..]
            });
            out.extend_from_slice(u);
        }
        out
    }

    #[test]
    fn splits_mixed_start_codes() {
        let data = annexb(&[SPS, PPS, IDR]);
        let units = split_annexb(&data);
        assert_eq!(units, vec![SPS, PPS, IDR]);
    }

    #[test]
    fn split_handles_no_leading_code_gracefully() {
        assert!(split_annexb(b"junk without start codes").is_empty());
        assert!(split_annexb(&[]).is_empty());
    }

    #[test]
    fn nal_types_decode() {
        assert_eq!(nal_type(SPS), 7);
        assert_eq!(nal_type(PPS), 8);
        assert_eq!(nal_type(IDR), 5);
        assert_eq!(nal_type(NON_IDR), 1);
        assert_eq!(nal_type(AUD), 9);
    }

    #[test]
    fn keyframe_classification_uses_encoded_idr_nals() {
        assert!(is_keyframe(&annexb(&[AUD, SPS, PPS, IDR])));
        assert!(!is_keyframe(&annexb(&[AUD, NON_IDR])));
    }

    #[test]
    fn extracts_sps_pps_from_sequence_header() {
        let hdr = annexb(&[SPS, PPS]);
        let (sps, pps) = extract_sps_pps(&hdr).expect("both present");
        assert_eq!(sps, SPS);
        assert_eq!(pps, PPS);
        // Also from a full access unit (in-band parameter sets).
        let au = annexb(&[AUD, SPS, PPS, SEI, IDR]);
        let (sps2, pps2) = extract_sps_pps(&au).expect("in-band");
        assert_eq!((sps2.as_slice(), pps2.as_slice()), (SPS, PPS));
        assert!(
            extract_sps_pps(&annexb(&[IDR])).is_none(),
            "no parameter sets"
        );
    }

    #[test]
    fn avcc_conversion_length_prefixes_and_strips_metadata_nals() {
        // AUD/SPS/PPS are carried out-of-band (avcC); slices + SEI stay.
        let au = annexb(&[AUD, SPS, PPS, SEI, IDR]);
        let avcc = annexb_to_avcc(&au);
        let mut expected = Vec::new();
        for u in [SEI, IDR] {
            expected.extend_from_slice(&(u.len() as u32).to_be_bytes());
            expected.extend_from_slice(u);
        }
        assert_eq!(avcc, expected);
    }

    #[test]
    fn avcc_conversion_of_plain_frame() {
        let au = annexb(&[NON_IDR]);
        let avcc = annexb_to_avcc(&au);
        assert_eq!(&avcc[..4], &(NON_IDR.len() as u32).to_be_bytes());
        assert_eq!(&avcc[4..], NON_IDR);
    }

    #[test]
    fn even_rounds_dimensions_down() {
        assert_eq!(even_dimensions(2290, 1288), (2290, 1288));
        assert_eq!(even_dimensions(2291, 1289), (2290, 1288));
        assert_eq!(even_dimensions(1, 1), (2, 2), "minimum sane size");
    }
}
