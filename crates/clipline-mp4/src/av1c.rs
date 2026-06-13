//! AV1CodecConfigurationRecord (`av1C`) construction. Summary fields come
//! from parsing the sequence header OBU's leading fields; the full OBU is
//! embedded as configOBUs (which is what decoders actually configure
//! from). Clipline's encode path always feeds 8-bit 4:2:0 (NV12), so the
//! color fields are emitted for that layout.

use crate::bitread::BitReader;
use crate::boxes::{mp4_box, Payload};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Av1SeqInfo {
    pub seq_profile: u8,
    pub seq_level_idx_0: u8,
    pub seq_tier_0: u8,
}

/// Parse profile/level/tier from a full sequence header OBU (OBU header +
/// optional size field + payload). `None` on anything malformed.
pub(crate) fn parse_sequence_header(obu: &[u8]) -> Option<Av1SeqInfo> {
    let header = *obu.first()?;
    if header & 0x80 != 0 {
        return None; // forbidden bit
    }
    if (header >> 3) & 0x0F != 1 {
        return None; // not a sequence header OBU
    }
    let mut pos = 1usize;
    if header & 0x04 != 0 {
        pos += 1; // extension byte
    }
    if header & 0x02 != 0 {
        // leb128 size field — skip it (the payload runs to end of slice).
        loop {
            let b = *obu.get(pos)?;
            pos += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    let mut r = BitReader::new(obu.get(pos..)?);

    let seq_profile = r.bits(3)? as u8;
    r.bit()?; // still_picture
    let reduced = r.bit()? == 1;
    if reduced {
        return Some(Av1SeqInfo {
            seq_profile,
            seq_level_idx_0: r.bits(5)? as u8,
            seq_tier_0: 0,
        });
    }
    let mut decoder_model_info_present = false;
    let mut buffer_delay_length = 0u32;
    if r.bit()? == 1 {
        // timing_info()
        r.bits(32)?; // num_units_in_display_tick
        r.bits(32)?; // time_scale
        if r.bit()? == 1 {
            r.ue()?; // num_ticks_per_picture_minus_1 (uvlc)
        }
        decoder_model_info_present = r.bit()? == 1;
        if decoder_model_info_present {
            buffer_delay_length = r.bits(5)? + 1;
            r.bits(32)?; // num_units_in_decoding_tick
            r.bits(5)?; // buffer_removal_time_length_minus_1
            r.bits(5)?; // frame_presentation_time_length_minus_1
        }
    }
    let initial_display_delay_present = r.bit()? == 1;
    r.bits(5)?; // operating_points_cnt_minus_1 — only op 0 is summarized
    r.bits(12)?; // operating_point_idc[0]
    let seq_level_idx_0 = r.bits(5)? as u8;
    let seq_tier_0 = if seq_level_idx_0 > 7 {
        r.bit()? as u8
    } else {
        0
    };
    // Consume op-0 conditionals only to keep the reads honest; later
    // operating points are irrelevant to av1C.
    if decoder_model_info_present && r.bit()? == 1 {
        r.bits(buffer_delay_length)?; // decoder_buffer_delay
        r.bits(buffer_delay_length)?; // encoder_buffer_delay
        r.bit()?; // low_delay_mode_flag
    }
    if initial_display_delay_present && r.bit()? == 1 {
        r.bits(4)?;
    }
    Some(Av1SeqInfo {
        seq_profile,
        seq_level_idx_0,
        seq_tier_0,
    })
}

/// Build the `av1C` box around a full sequence header OBU. Color fields
/// state 8-bit 4:2:0 — the only layout Clipline's encoders produce.
pub(crate) fn av1c(sequence_header_obu: &[u8]) -> Vec<u8> {
    let info = parse_sequence_header(sequence_header_obu).unwrap_or(Av1SeqInfo {
        seq_profile: 0,
        seq_level_idx_0: 0,
        seq_tier_0: 0,
    });
    let mut p = Payload::new();
    p.u8(0x81) // marker 1 | version 1
        .u8((info.seq_profile << 5) | info.seq_level_idx_0)
        // tier | high_bitdepth 0 | twelve_bit 0 | monochrome 0 |
        // subsampling_x 1 | subsampling_y 1 | chroma_sample_position 00
        .u8((info.seq_tier_0 << 7) | 0x0C)
        .u8(0) // no initial_presentation_delay
        .bytes(sequence_header_obu); // configOBUs
    mp4_box(*b"av1C", p.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real SVT-AV1 sequence header (128x72, main profile, level auto→2.0).
    pub(crate) const SEQ_OBU: &[u8] = &[
        0x0A, 0x0A, 0x00, 0x00, 0x00, 0x03, 0x37, 0xF8, 0xE3, 0x57, 0xCC, 0x02,
    ];

    #[test]
    fn parses_real_svt_av1_sequence_header() {
        let info = parse_sequence_header(SEQ_OBU).expect("valid OBU");
        assert_eq!(info.seq_profile, 0, "main profile");
        assert_eq!(info.seq_level_idx_0, 0, "level 2.0");
        assert_eq!(info.seq_tier_0, 0);
    }

    #[test]
    fn parses_reduced_still_picture_header() {
        // profile=2, still=0, reduced=1, seq_level_idx_0=12.
        let obu = [0x0A, 0x02, 0b0100_1011, 0b0000_0000];
        let info = parse_sequence_header(&obu).expect("valid OBU");
        assert_eq!(info.seq_profile, 2);
        assert_eq!(info.seq_level_idx_0, 12);
        assert_eq!(info.seq_tier_0, 0);
    }

    #[test]
    fn parses_high_level_with_tier_bit() {
        // profile=1, no timing, no display delay, 1 op, level 8, tier 1.
        let obu = [0x0A, 0x04, 0x20, 0x00, 0x00, 0x44];
        let info = parse_sequence_header(&obu).expect("valid OBU");
        assert_eq!(info.seq_profile, 1);
        assert_eq!(info.seq_level_idx_0, 8);
        assert_eq!(info.seq_tier_0, 1);
    }

    #[test]
    fn rejects_non_sequence_header_obus() {
        assert_eq!(
            parse_sequence_header(&[0x32, 0x01, 0x00]),
            None,
            "frame OBU"
        );
        assert_eq!(
            parse_sequence_header(&[0x8A, 0x01, 0x00]),
            None,
            "forbidden bit"
        );
        assert_eq!(parse_sequence_header(&[]), None);
        assert_eq!(parse_sequence_header(&[0x0A, 0x02]), None, "truncated");
    }

    #[test]
    fn av1c_embeds_summary_and_config_obus() {
        let buf = av1c(SEQ_OBU);
        assert_eq!(&buf[4..8], b"av1C");
        let p = &buf[8..];
        assert_eq!(p[0], 0x81, "marker | version 1");
        assert_eq!(p[1], 0x00, "profile 0 | level 0");
        assert_eq!(p[2], 0x0C, "tier 0, 8-bit 4:2:0");
        assert_eq!(p[3], 0x00, "no initial presentation delay");
        assert_eq!(&p[4..], SEQ_OBU, "configOBUs is the verbatim OBU");
    }
}
