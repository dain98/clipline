//! HEVC (H.265) Annex B ↔ MP4 (hvcC) stream-format conversion. Mirrors
//! `annexb.rs` but for HEVC's 6-bit NAL type and parameter-set layout:
//! `clipline-mp4` writes hvcC with 4-byte length prefixes; FFmpeg's HEVC
//! encoders emit Annex B start codes. Pure byte math — platform-neutral
//! and unit tested on every OS. NAL splitting reuses `annexb::split_annexb`
//! (start codes are codec-independent).

use crate::annexb::split_annexb;

/// HEVC nal_unit_type — bits 1..6 of the first header byte
/// (`(byte >> 1) & 0x3F`). 32 = VPS, 33 = SPS, 34 = PPS, 35 = AUD,
/// 39/40 = PREFIX/SUFFIX SEI, 19/20 = IDR, 21 = CRA.
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| (b >> 1) & 0x3F).unwrap_or(0)
}

/// True if the access unit contains an IRAP picture (IDR/CRA/BLA, types
/// 16..=23) — i.e. a decoder refresh point the muxer can mark as a sync
/// sample. (Encoders also flag keyframes directly; this lets neutral code
/// double-check.)
pub fn is_keyframe(annexb: &[u8]) -> bool {
    split_annexb(annexb)
        .iter()
        .any(|u| matches!(nal_type(u), 16..=23))
}

/// Pull (VPS, SPS, PPS) out of an Annex B blob — works on a parameter-set
/// header and on full access units with in-band parameter sets.
pub fn extract_vps_sps_pps(annexb: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let units = split_annexb(annexb);
    let vps = units.iter().find(|u| nal_type(u) == 32)?;
    let sps = units.iter().find(|u| nal_type(u) == 33)?;
    let pps = units.iter().find(|u| nal_type(u) == 34)?;
    Some((vps.to_vec(), sps.to_vec(), pps.to_vec()))
}

/// Convert one Annex B access unit to 4-byte-length-prefixed sample data.
/// VPS/SPS/PPS (32–34) and AUD/EOS/EOB/FD (35–38) are dropped: parameter
/// sets travel in hvcC, and MP4 needs no access-unit delimiters. SEI
/// (39/40) and slice NALs are kept.
pub fn annexb_to_hvcc_samples(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for unit in split_annexb(annexb) {
        if matches!(nal_type(unit), 32..=38) {
            continue;
        }
        out.extend_from_slice(&(unit.len() as u32).to_be_bytes());
        out.extend_from_slice(unit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Structurally-real HEVC NALs: 2-byte header (type in bits 1..6 of
    // byte 0) + payload.
    const VPS: &[u8] = &[0x40, 0x01, 0x0C, 0x01];
    const SPS: &[u8] = &[0x42, 0x01, 0x01, 0x60];
    const PPS: &[u8] = &[0x44, 0x01, 0xC1, 0x72];
    const AUD: &[u8] = &[0x46, 0x01, 0x50];
    const SEI: &[u8] = &[0x4E, 0x01, 0x05, 0x04]; // PREFIX_SEI (39)
    const IDR: &[u8] = &[0x26, 0x01, 0xAF, 0x00]; // IDR_W_RADL (19)
    const TRAIL: &[u8] = &[0x02, 0x01, 0xD0, 0x09]; // TRAIL_R (1)

    fn annexb(units: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, u) in units.iter().enumerate() {
            out.extend_from_slice(if i % 2 == 0 { &[0, 0, 0, 1][..] } else { &[0, 0, 1][..] });
            out.extend_from_slice(u);
        }
        out
    }

    #[test]
    fn nal_types_decode() {
        assert_eq!(nal_type(VPS), 32);
        assert_eq!(nal_type(SPS), 33);
        assert_eq!(nal_type(PPS), 34);
        assert_eq!(nal_type(AUD), 35);
        assert_eq!(nal_type(SEI), 39);
        assert_eq!(nal_type(IDR), 19);
        assert_eq!(nal_type(TRAIL), 1);
    }

    #[test]
    fn keyframe_detection_finds_irap() {
        assert!(is_keyframe(&annexb(&[VPS, SPS, PPS, IDR])));
        assert!(!is_keyframe(&annexb(&[TRAIL])));
    }

    #[test]
    fn extracts_parameter_sets() {
        let hdr = annexb(&[VPS, SPS, PPS]);
        let (vps, sps, pps) = extract_vps_sps_pps(&hdr).expect("all three present");
        assert_eq!((vps.as_slice(), sps.as_slice(), pps.as_slice()), (VPS, SPS, PPS));
        // Also from a full access unit with in-band parameter sets.
        let au = annexb(&[AUD, VPS, SPS, PPS, SEI, IDR]);
        assert!(extract_vps_sps_pps(&au).is_some());
        // Missing one → None.
        assert!(extract_vps_sps_pps(&annexb(&[SPS, PPS])).is_none());
    }

    #[test]
    fn sample_conversion_length_prefixes_and_strips_metadata() {
        let au = annexb(&[AUD, VPS, SPS, PPS, SEI, IDR]);
        let samples = annexb_to_hvcc_samples(&au);
        let mut expected = Vec::new();
        for u in [SEI, IDR] {
            expected.extend_from_slice(&(u.len() as u32).to_be_bytes());
            expected.extend_from_slice(u);
        }
        assert_eq!(samples, expected);
    }
}
