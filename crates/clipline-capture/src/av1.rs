//! AV1 OBU stream handling for MP4 (av01) muxing. FFmpeg's AV1 encoders
//! emit the "low-overhead bitstream format" — a sequence of OBUs each
//! carrying `obu_has_size_field = 1`. For `av01` samples the per-frame
//! OBUs are passed through with temporal-delimiter OBUs removed (per the
//! AV1-in-ISOBMFF spec), and the sequence-header OBU is lifted out for the
//! `av1C` config record. Pure byte math — platform-neutral, unit tested on
//! every OS.

/// OBU types we care about (Section 5 of the AV1 spec).
const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;

/// One parsed OBU: its type and the full bytes (header + size field +
/// payload) as they appeared in the stream.
struct Obu<'a> {
    obu_type: u8,
    bytes: &'a [u8],
}

/// Walk a low-overhead AV1 bitstream into OBUs. Returns `None` if any OBU
/// lacks a size field (we can't find the next boundary) or is truncated —
/// callers treat that as a malformed packet rather than guessing.
fn walk_obus(data: &[u8]) -> Option<Vec<Obu<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let start = pos;
        let header = *data.get(pos)?;
        if header & 0x80 != 0 {
            return None; // obu_forbidden_bit set
        }
        let obu_type = (header >> 3) & 0x0F;
        let extension = header & 0x04 != 0;
        let has_size = header & 0x02 != 0;
        pos += 1;
        if extension {
            pos += 1; // extension header byte
        }
        if !has_size {
            return None; // need explicit sizes to delimit OBUs
        }
        let (size, consumed) = read_leb128(data.get(pos..)?)?;
        pos += consumed;
        let end = pos.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        pos = end;
        out.push(Obu { obu_type, bytes: &data[start..end] });
    }
    Some(out)
}

/// Decode an unsigned LEB128 value, returning (value, bytes_consumed).
fn read_leb128(data: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for i in 0..8 {
        let byte = *data.get(i)?;
        value |= ((byte & 0x7F) as usize) << (i * 7);
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
    }
    None // more than 8 bytes is malformed for our payload sizes
}

/// Extract the full sequence-header OBU (header + size + payload) for the
/// `av1C` config record. `None` if the stream is malformed or has none.
pub fn extract_sequence_header(data: &[u8]) -> Option<Vec<u8>> {
    walk_obus(data)?
        .into_iter()
        .find(|o| o.obu_type == OBU_SEQUENCE_HEADER)
        .map(|o| o.bytes.to_vec())
}

/// Produce `av01` sample data: every OBU except temporal delimiters,
/// concatenated verbatim (each already carries its size field). Sequence
/// headers are kept — repeating them at keyframes is spec-legal and lets
/// mid-stream seeks reconfigure. Returns the input unchanged if it can't
/// be parsed (FFmpeg always emits size fields, but never corrupt a save).
pub fn obus_to_av01_sample(data: &[u8]) -> Vec<u8> {
    let Some(obus) = walk_obus(data) else {
        return data.to_vec();
    };
    let mut out = Vec::with_capacity(data.len());
    for obu in obus {
        if obu.obu_type == OBU_TEMPORAL_DELIMITER {
            continue;
        }
        out.extend_from_slice(obu.bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // header byte | size leb128 | payload. has_size_field set (0x02).
    fn obu(obu_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![(obu_type << 3) | 0x02];
        v.push(payload.len() as u8); // single-byte leb128 for small payloads
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn leb128_decodes_multibyte() {
        assert_eq!(read_leb128(&[0x00]), Some((0, 1)));
        assert_eq!(read_leb128(&[0x7F]), Some((127, 1)));
        assert_eq!(read_leb128(&[0x80, 0x01]), Some((128, 2)));
        assert_eq!(read_leb128(&[0xFF, 0xFF, 0x03]), Some((65535, 3)));
        assert_eq!(read_leb128(&[0x80]), None, "truncated");
    }

    #[test]
    fn walks_a_temporal_unit() {
        let mut stream = Vec::new();
        stream.extend(obu(OBU_TEMPORAL_DELIMITER, &[]));
        stream.extend(obu(OBU_SEQUENCE_HEADER, &[0xAA, 0xBB]));
        stream.extend(obu(6, &[0x01, 0x02, 0x03])); // OBU_FRAME
        let obus = walk_obus(&stream).expect("valid stream");
        let types: Vec<u8> = obus.iter().map(|o| o.obu_type).collect();
        assert_eq!(types, vec![OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, 6]);
    }

    #[test]
    fn rejects_obus_without_size_field() {
        // has_size_field clear → can't delimit.
        assert!(walk_obus(&[OBU_SEQUENCE_HEADER << 3]).is_none());
        // forbidden bit set.
        assert!(walk_obus(&[0x80]).is_none());
    }

    #[test]
    fn extracts_sequence_header_obu() {
        let mut stream = Vec::new();
        stream.extend(obu(OBU_TEMPORAL_DELIMITER, &[]));
        let seq = obu(OBU_SEQUENCE_HEADER, &[0xAA, 0xBB, 0xCC]);
        stream.extend(seq.clone());
        stream.extend(obu(6, &[0x01]));
        assert_eq!(extract_sequence_header(&stream), Some(seq));
        // None when absent.
        assert_eq!(extract_sequence_header(&obu(6, &[0x01])), None);
    }

    #[test]
    fn sample_strips_only_temporal_delimiters() {
        let td = obu(OBU_TEMPORAL_DELIMITER, &[]);
        let seq = obu(OBU_SEQUENCE_HEADER, &[0xAA]);
        let frame = obu(6, &[0x01, 0x02]);
        let mut stream = Vec::new();
        stream.extend(td);
        stream.extend(seq.clone());
        stream.extend(frame.clone());
        let sample = obus_to_av01_sample(&stream);
        let mut expected = seq;
        expected.extend(frame);
        assert_eq!(sample, expected);
    }

    #[test]
    fn unparseable_input_passes_through_unchanged() {
        let junk = vec![0x80, 0x12, 0x34];
        assert_eq!(obus_to_av01_sample(&junk), junk);
    }
}
