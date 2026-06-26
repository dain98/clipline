#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
#[derive(Clone)]
pub struct FallbackToken(String);

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
impl FallbackToken {
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 16];
        fill_random_bytes(&mut seed)?;
        Ok(Self(base64_url_no_pad(&seed)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, candidate: &str) -> bool {
        fixed_length_bytes_match(self.0.as_bytes(), candidate.as_bytes())
    }

    #[cfg(test)]
    fn generate_for_tests(seed: u64) -> Self {
        Self(base64_url_no_pad(&seed.to_le_bytes()))
    }
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!(
            "generate fallback token: BCryptGenRandom failed with {status:#x}"
        ));
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn fixed_length_bytes_match(expected: &[u8], candidate: &[u8]) -> bool {
    if expected.len() != candidate.len() {
        return false;
    }

    let mut diff = 0u8;
    for (&expected_byte, &candidate_byte) in expected.iter().zip(candidate) {
        diff |= expected_byte ^ candidate_byte;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_url_safe_and_unique() {
        let first = FallbackToken::generate_for_tests(0x1234_5678_9abc_def0);
        let second = FallbackToken::generate_for_tests(0xfedc_ba98_7654_3210);

        assert_ne!(first.as_str(), second.as_str());
        assert!(
            first
                .as_str()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert!(first.matches(first.as_str()));
        assert!(!first.matches(second.as_str()));
    }

    #[cfg(windows)]
    #[test]
    fn generated_tokens_use_real_rng_and_expected_shape() {
        let first = FallbackToken::generate().expect("generate first fallback token");
        let second = FallbackToken::generate().expect("generate second fallback token");

        assert_eq!(first.as_str().len(), 22);
        assert!(
            first
                .as_str()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(first.as_str(), second.as_str());
    }
}
