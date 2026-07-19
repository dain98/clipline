#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxHeaderError {
    MissingLargeSize,
    SizeOverflow,
    InvalidExtent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodedBoxHeader {
    pub size: u64,
    pub payload_offset: u64,
    pub end: u64,
}

pub(crate) fn uses_large_size(size32: u32) -> bool {
    size32 == 1
}

pub(crate) fn decode_box_header(
    size32: u32,
    large_size: Option<u64>,
    offset: u64,
    limit: u64,
) -> Result<DecodedBoxHeader, BoxHeaderError> {
    let (size, header_len) = if uses_large_size(size32) {
        (large_size.ok_or(BoxHeaderError::MissingLargeSize)?, 16_u64)
    } else if size32 == 0 {
        (
            limit
                .checked_sub(offset)
                .ok_or(BoxHeaderError::InvalidExtent)?,
            8_u64,
        )
    } else {
        (u64::from(size32), 8_u64)
    };

    let payload_offset = offset
        .checked_add(header_len)
        .ok_or(BoxHeaderError::SizeOverflow)?;
    let end = offset
        .checked_add(size)
        .ok_or(BoxHeaderError::SizeOverflow)?;
    if size < header_len || end > limit {
        return Err(BoxHeaderError::InvalidExtent);
    }

    Ok(DecodedBoxHeader {
        size,
        payload_offset,
        end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_normal_large_and_terminal_boxes() {
        assert_eq!(decode_box_header(12, None, 4, 20).unwrap().end, 16);
        assert_eq!(
            decode_box_header(1, Some(24), 4, 28)
                .unwrap()
                .payload_offset,
            20
        );
        assert_eq!(decode_box_header(0, None, 4, 20).unwrap().size, 16);
    }

    #[test]
    fn rejects_overflow_and_invalid_extents() {
        assert_eq!(
            decode_box_header(16, None, u64::MAX - 7, u64::MAX),
            Err(BoxHeaderError::SizeOverflow)
        );
        assert_eq!(
            decode_box_header(7, None, 0, 20),
            Err(BoxHeaderError::InvalidExtent)
        );
        assert_eq!(
            decode_box_header(1, None, 0, 20),
            Err(BoxHeaderError::MissingLargeSize)
        );
    }
}
