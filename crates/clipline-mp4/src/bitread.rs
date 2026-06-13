//! Minimal MSB-first bit reader for codec parameter-set parsing
//! (HEVC SPS profile_tier_level → hvcC, AV1 sequence header → av1C).
//! Pure byte math — platform-neutral and unit tested on every OS.

/// MSB-first bit cursor. All reads return `None` past the end of data —
/// parameter-set parsers treat truncation as a parse failure, never a panic.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize, // bit position
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn bit(&mut self) -> Option<u32> {
        let byte = self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(bit as u32)
    }

    /// Read `n` bits (n ≤ 32) MSB-first.
    pub(crate) fn bits(&mut self, n: u32) -> Option<u32> {
        debug_assert!(n <= 32);
        let mut out = 0u32;
        for _ in 0..n {
            out = (out << 1) | self.bit()?;
        }
        Some(out)
    }

    /// Unsigned Exp-Golomb (H.26x `ue(v)`; bit-identical to AV1 `uvlc()`
    /// for the value ranges parameter sets use).
    pub(crate) fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None; // corrupt: no sane parameter set needs more
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }
}

/// Strip H.26x emulation-prevention bytes: `00 00 03` → `00 00`.
pub(crate) fn unescape_rbsp(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0usize;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue; // emulation-prevention byte
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bits_msb_first() {
        let mut r = BitReader::new(&[0b1011_0001, 0b1000_0000]);
        assert_eq!(r.bit(), Some(1));
        assert_eq!(r.bits(3), Some(0b011));
        assert_eq!(r.bits(5), Some(0b0_0011));
        assert_eq!(r.bits(7), Some(0));
        assert_eq!(r.bit(), None, "past the end");
    }

    #[test]
    fn exp_golomb_decodes_small_values() {
        // Bits "1 010 011 0" decode as ue=0, ue=1, ue=2 (trailing 0 unused).
        let mut r = BitReader::new(&[0b1010_0110]);
        assert_eq!(r.ue(), Some(0));
        assert_eq!(r.ue(), Some(1));
        assert_eq!(r.ue(), Some(2));
        // 128 = 7 leading zeros, then the 8-bit suffix "1000_0001".
        let mut r = BitReader::new(&[0b0000_0001, 0b0000_0010]);
        assert_eq!(r.ue(), Some(128));
    }

    #[test]
    fn exp_golomb_rejects_truncation_and_runaway() {
        // 7 leading zeros promise 7 suffix bits, but the byte ends first.
        assert_eq!(
            BitReader::new(&[0b0000_0001]).ue(),
            None,
            "truncated suffix"
        );
        assert_eq!(BitReader::new(&[0; 8]).ue(), None, "all-zero runaway");
    }

    #[test]
    fn unescape_strips_emulation_prevention() {
        assert_eq!(unescape_rbsp(&[0, 0, 3, 0, 1]), vec![0, 0, 0, 1]);
        // The 3 only escapes after exactly-two zeros; others pass through.
        assert_eq!(unescape_rbsp(&[0, 3, 0, 0, 3, 3]), vec![0, 3, 0, 0, 3]);
        assert_eq!(unescape_rbsp(&[1, 2, 3]), vec![1, 2, 3]);
    }
}
