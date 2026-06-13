//! HEVCDecoderConfigurationRecord (`hvcC`) construction. The record's
//! summary fields come from parsing the SPS profile_tier_level — pure bit
//! math, platform-neutral, unit tested on real x265 parameter sets.

use crate::bitread::{unescape_rbsp, BitReader};
use crate::boxes::{mp4_box, Payload};

/// The SPS fields hvcC repeats outside the raw NAL arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HevcSpsInfo {
    pub general_profile_space: u8,
    pub general_tier_flag: u8,
    pub general_profile_idc: u8,
    pub general_profile_compatibility_flags: u32,
    pub general_constraint_indicator_flags: u64, // 48 bits
    pub general_level_idc: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    /// sps_max_sub_layers_minus1 + 1.
    pub num_temporal_layers: u8,
    pub temporal_id_nested: u8,
}

/// Parse one raw HEVC SPS NAL (2-byte NAL header, no start code). Returns
/// `None` on truncated/corrupt input — callers fall back to a zeroed
/// record rather than failing the save.
pub(crate) fn parse_sps(sps: &[u8]) -> Option<HevcSpsInfo> {
    let rbsp = unescape_rbsp(sps.get(2..)?);
    let mut r = BitReader::new(&rbsp);
    r.bits(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.bits(3)?;
    let temporal_id_nested = r.bit()? as u8;

    // profile_tier_level(1, max_sub_layers_minus1) — general fields.
    let general_profile_space = r.bits(2)? as u8;
    let general_tier_flag = r.bit()? as u8;
    let general_profile_idc = r.bits(5)? as u8;
    let general_profile_compatibility_flags = r.bits(32)?;
    let general_constraint_indicator_flags = ((r.bits(32)? as u64) << 16) | r.bits(16)? as u64;
    let general_level_idc = r.bits(8)? as u8;
    // Sub-layer presence flags + ordering padding + per-layer data.
    let mut profile_present = [false; 8];
    let mut level_present = [false; 8];
    for i in 0..max_sub_layers_minus1 as usize {
        profile_present[i] = r.bit()? == 1;
        level_present[i] = r.bit()? == 1;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            r.bits(2)?; // reserved_zero_2bits
        }
    }
    for i in 0..max_sub_layers_minus1 as usize {
        if profile_present[i] {
            r.bits(32)?;
            r.bits(32)?;
            r.bits(24)?; // 88 bits of sub-layer profile data
        }
        if level_present[i] {
            r.bits(8)?;
        }
    }

    r.ue()?; // sps_seq_parameter_set_id
    let chroma_format_idc = r.ue()?;
    if chroma_format_idc == 3 {
        r.bit()?; // separate_colour_plane_flag
    }
    r.ue()?; // pic_width_in_luma_samples
    r.ue()?; // pic_height_in_luma_samples
    if r.bit()? == 1 {
        // conformance_window: left/right/top/bottom offsets
        for _ in 0..4 {
            r.ue()?;
        }
    }
    let bit_depth_luma_minus8 = r.ue()?;
    let bit_depth_chroma_minus8 = r.ue()?;

    Some(HevcSpsInfo {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_constraint_indicator_flags,
        general_level_idc,
        chroma_format_idc: u8::try_from(chroma_format_idc).ok()?,
        bit_depth_luma_minus8: u8::try_from(bit_depth_luma_minus8).ok()?,
        bit_depth_chroma_minus8: u8::try_from(bit_depth_chroma_minus8).ok()?,
        num_temporal_layers: max_sub_layers_minus1 as u8 + 1,
        temporal_id_nested,
    })
}

/// Build the `hvcC` box from raw VPS/SPS/PPS NALs. A corrupt SPS yields a
/// zeroed summary (the raw NAL arrays still let decoders configure) —
/// mirroring avcC's tolerance of short parameter sets.
pub(crate) fn hvcc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let info = parse_sps(sps).unwrap_or(HevcSpsInfo {
        general_profile_space: 0,
        general_tier_flag: 0,
        general_profile_idc: 0,
        general_profile_compatibility_flags: 0,
        general_constraint_indicator_flags: 0,
        general_level_idc: 0,
        chroma_format_idc: 0,
        bit_depth_luma_minus8: 0,
        bit_depth_chroma_minus8: 0,
        num_temporal_layers: 1,
        temporal_id_nested: 0,
    });
    let mut p = Payload::new();
    p.u8(1) // configurationVersion
        .u8((info.general_profile_space << 6)
            | (info.general_tier_flag << 5)
            | info.general_profile_idc)
        .u32(info.general_profile_compatibility_flags)
        .u16((info.general_constraint_indicator_flags >> 32) as u16)
        .u32(info.general_constraint_indicator_flags as u32)
        .u8(info.general_level_idc)
        .u16(0xF000) // reserved 1111 + min_spatial_segmentation_idc 0
        .u8(0xFC) // reserved 111111 + parallelismType 0 (unknown)
        .u8(0xFC | info.chroma_format_idc)
        .u8(0xF8 | info.bit_depth_luma_minus8)
        .u8(0xF8 | info.bit_depth_chroma_minus8)
        .u16(0) // avgFrameRate 0 (unspecified)
        // constantFrameRate 0 | numTemporalLayers | temporalIdNested |
        // lengthSizeMinusOne 3 (4-byte length prefixes). numTemporalLayers is
        // the actual layer count (1-7) per ISO/IEC 14496-15, not a minus-1.
        .u8((info.num_temporal_layers << 3) | (info.temporal_id_nested << 2) | 3)
        .u8(3); // numOfArrays: VPS, SPS, PPS
    for (nal_type, nal) in [(32u8, vps), (33, sps), (34, pps)] {
        // hvcC nalUnitLength is u16 by spec; real parameter sets are far smaller.
        debug_assert!(
            nal.len() <= u16::MAX as usize,
            "HEVC NAL exceeds hvcC u16 length"
        );
        p.u8(0x80 | nal_type) // array_completeness=1
            .u16(1) // numNalus
            .u16(nal.len() as u16)
            .bytes(nal);
    }
    mp4_box(*b"hvcC", p.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real x265 parameter sets (128x72, Main profile, level 1.0); the SPS
    // contains emulation-prevention bytes, exercising the unescape path.
    pub(crate) const VPS: &[u8] = &[
        0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x03, 0x00, 0x1E, 0x95, 0x98, 0x09,
    ];
    pub(crate) const SPS: &[u8] = &[
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x90, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x1E, 0xA0, 0x10, 0x20, 0x49, 0x65, 0x95, 0x9A, 0x49, 0x32, 0xBC, 0x05, 0xA0,
        0x20, 0x00, 0x00, 0x03, 0x00, 0x20, 0x00, 0x00, 0x03, 0x03, 0xC1,
    ];
    pub(crate) const PPS: &[u8] = &[0x44, 0x01, 0xC1, 0x72, 0xB4, 0x22, 0x40];

    #[test]
    fn parses_real_x265_sps() {
        let info = parse_sps(SPS).expect("valid SPS");
        assert_eq!(info.general_profile_space, 0);
        assert_eq!(info.general_tier_flag, 0);
        assert_eq!(info.general_profile_idc, 1, "Main profile");
        assert_eq!(info.general_profile_compatibility_flags, 0x6000_0000);
        assert_eq!(info.general_constraint_indicator_flags, 0x9000_0000_0000);
        assert_eq!(info.general_level_idc, 30, "level 1.0");
        assert_eq!(info.chroma_format_idc, 1, "4:2:0");
        assert_eq!(info.bit_depth_luma_minus8, 0);
        assert_eq!(info.bit_depth_chroma_minus8, 0);
        assert_eq!(info.num_temporal_layers, 1);
        assert_eq!(info.temporal_id_nested, 1);
    }

    #[test]
    fn parse_rejects_truncated_sps() {
        assert_eq!(parse_sps(&SPS[..6]), None);
        assert_eq!(parse_sps(&[]), None);
    }

    #[test]
    fn hvcc_embeds_summary_and_all_three_nals() {
        let buf = hvcc(VPS, SPS, PPS);
        assert_eq!(&buf[4..8], b"hvcC");
        let p = &buf[8..];
        assert_eq!(p[0], 1, "configurationVersion");
        assert_eq!(p[1], 0x01, "space 0 | tier 0 | profile_idc 1");
        assert_eq!(&p[2..6], &0x6000_0000u32.to_be_bytes());
        assert_eq!(&p[6..12], &[0x90, 0, 0, 0, 0, 0], "constraint flags");
        assert_eq!(p[12], 30, "level");
        assert_eq!(p[15], 0xFC, "parallelismType unknown");
        assert_eq!(p[16], 0xFC | 1, "chroma_format_idc 1");
        assert_eq!(p[17], 0xF8, "8-bit luma");
        assert_eq!(p[18], 0xF8, "8-bit chroma");
        assert_eq!(p[21], (1 << 3) | (1 << 2) | 3, "layers/nested/length-1");
        assert_eq!(p[22], 3, "numOfArrays");
        // Each raw NAL must be embedded verbatim.
        for nal in [VPS, SPS, PPS] {
            assert!(buf.windows(nal.len()).any(|w| w == nal));
        }
        // Array headers carry the right NAL types with completeness set.
        assert_eq!(p[23], 0x80 | 32);
    }
}
