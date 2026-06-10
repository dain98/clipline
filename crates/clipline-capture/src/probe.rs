/// Hardware/software encoder backends in deterministic priority order
/// (ddoc §3: NVENC → AMF → QuickSync → x264 software fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EncoderBackend {
    Nvenc,
    Amf,
    QuickSync,
    X264,
}

/// Codec preference order (ddoc §3: AV1 → HEVC → H.264).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Codec {
    Av1,
    Hevc,
    H264,
}

/// What one backend reported during startup probing.
#[derive(Debug, Clone)]
pub struct EncoderCapability {
    pub backend: EncoderBackend,
    pub codecs: Vec<Codec>,
}

/// Pick the encoder: highest-priority backend that offers any codec, then
/// the most-preferred codec within it. Derived `Ord` on the enums encodes
/// the ddoc §3 priority (declaration order).
pub fn select_encoder(available: &[EncoderCapability]) -> Option<(EncoderBackend, Codec)> {
    available
        .iter()
        .filter(|c| !c.codecs.is_empty())
        .min_by_key(|c| c.backend)
        .map(|c| (c.backend, *c.codecs.iter().min().expect("non-empty")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_nvenc_over_other_backends() {
        let caps = vec![
            EncoderCapability { backend: EncoderBackend::Amf, codecs: vec![Codec::Av1] },
            EncoderCapability {
                backend: EncoderBackend::Nvenc,
                codecs: vec![Codec::H264, Codec::Hevc],
            },
        ];
        // Backend priority wins even when a lower backend has a better codec.
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::Nvenc, Codec::Hevc)));
    }

    #[test]
    fn prefers_av1_within_a_backend() {
        let caps = vec![EncoderCapability {
            backend: EncoderBackend::QuickSync,
            codecs: vec![Codec::H264, Codec::Av1, Codec::Hevc],
        }];
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::QuickSync, Codec::Av1)));
    }

    #[test]
    fn falls_back_to_software_x264() {
        let caps = vec![EncoderCapability {
            backend: EncoderBackend::X264,
            codecs: vec![Codec::H264],
        }];
        assert_eq!(select_encoder(&caps), Some((EncoderBackend::X264, Codec::H264)));
    }

    #[test]
    fn no_encoders_means_none() {
        assert_eq!(select_encoder(&[]), None);
        let empty_codecs = vec![EncoderCapability {
            backend: EncoderBackend::Nvenc,
            codecs: vec![],
        }];
        assert_eq!(select_encoder(&empty_codecs), None);
    }
}
