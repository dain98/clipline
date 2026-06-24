use clipline_capture::probe::{Codec, EncoderBackend, EncoderCandidate, EncoderPreference};

/// The user's encoder choice. `Auto` prefers H.264 for playback compatibility while
/// respecting backend merit order within a codec; the explicit variants force a
/// (backend, codec) pair (still falling back through Auto if it can't open).
/// Legacy saved values (`auto`, `nvenc_h264`, `amf_h264`, `quick_sync_h264`)
/// still deserialize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    #[default]
    Auto,
    NvencH264,
    NvencHevc,
    NvencAv1,
    AmfH264,
    AmfHevc,
    AmfAv1,
    QuickSyncH264,
    QuickSyncHevc,
    QuickSyncAv1,
    VideoToolboxH264,
    SvtAv1,
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
impl VideoEncoder {
    pub(crate) fn preference(self) -> EncoderPreference {
        let (backend, codec) = match self {
            Self::Auto => return EncoderPreference::Auto,
            Self::NvencH264 => (EncoderBackend::Nvenc, Codec::H264),
            Self::NvencHevc => (EncoderBackend::Nvenc, Codec::Hevc),
            Self::NvencAv1 => (EncoderBackend::Nvenc, Codec::Av1),
            Self::AmfH264 => (EncoderBackend::Amf, Codec::H264),
            Self::AmfHevc => (EncoderBackend::Amf, Codec::Hevc),
            Self::AmfAv1 => (EncoderBackend::Amf, Codec::Av1),
            Self::QuickSyncH264 => (EncoderBackend::QuickSync, Codec::H264),
            Self::QuickSyncHevc => (EncoderBackend::QuickSync, Codec::Hevc),
            Self::QuickSyncAv1 => (EncoderBackend::QuickSync, Codec::Av1),
            Self::VideoToolboxH264 => (EncoderBackend::VideoToolbox, Codec::H264),
            Self::SvtAv1 => (EncoderBackend::SvtAv1, Codec::Av1),
        };
        EncoderPreference::Explicit { backend, codec }
    }

    /// The settings/serde id (snake_case). Kept in lockstep with the
    /// `serde(rename_all = "snake_case")` derive by a test.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::NvencH264 => "nvenc_h264",
            Self::NvencHevc => "nvenc_hevc",
            Self::NvencAv1 => "nvenc_av1",
            Self::AmfH264 => "amf_h264",
            Self::AmfHevc => "amf_hevc",
            Self::AmfAv1 => "amf_av1",
            Self::QuickSyncH264 => "quick_sync_h264",
            Self::QuickSyncHevc => "quick_sync_hevc",
            Self::QuickSyncAv1 => "quick_sync_av1",
            Self::VideoToolboxH264 => "video_toolbox_h264",
            Self::SvtAv1 => "svt_av1",
        }
    }

    /// The explicit variant for a (backend, codec) pair, if Clipline exposes
    /// it as a user choice. `None` for combinations with no settings id
    /// (e.g. `MfSoftware`, or SvtAv1 paired with a non-AV1 codec).
    pub(crate) fn from_parts(backend: EncoderBackend, codec: Codec) -> Option<Self> {
        Some(match (backend, codec) {
            (EncoderBackend::Nvenc, Codec::H264) => Self::NvencH264,
            (EncoderBackend::Nvenc, Codec::Hevc) => Self::NvencHevc,
            (EncoderBackend::Nvenc, Codec::Av1) => Self::NvencAv1,
            (EncoderBackend::Amf, Codec::H264) => Self::AmfH264,
            (EncoderBackend::Amf, Codec::Hevc) => Self::AmfHevc,
            (EncoderBackend::Amf, Codec::Av1) => Self::AmfAv1,
            (EncoderBackend::QuickSync, Codec::H264) => Self::QuickSyncH264,
            (EncoderBackend::QuickSync, Codec::Hevc) => Self::QuickSyncHevc,
            (EncoderBackend::QuickSync, Codec::Av1) => Self::QuickSyncAv1,
            (EncoderBackend::VideoToolbox, Codec::H264) => Self::VideoToolboxH264,
            (EncoderBackend::SvtAv1, Codec::Av1) => Self::SvtAv1,
            _ => return None,
        })
    }
}

/// The settings id string for a codec, matching the frontend's decode-probe
/// keys ("h264"/"hevc"/"av1").
pub fn codec_id(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "av1",
        Codec::Hevc => "hevc",
        Codec::H264 => "h264",
    }
}

/// One selectable encoder for the Settings dropdown.
#[derive(serde::Serialize)]
pub struct EncoderOption {
    /// VideoEncoder settings id (e.g. "amf_hevc").
    pub id: String,
    /// Human label (e.g. "AMD AMF · HEVC").
    pub name: String,
    /// Codec key the frontend matches against its decode-capability probe.
    pub codec: String,
}

/// A short, human-readable label for the active encoder, shown in the
/// sidebar status (e.g. "AMD AMF · H.264" or "Software · AV1").
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn encoder_label(candidate: EncoderCandidate) -> String {
    let backend = match candidate.backend {
        EncoderBackend::Nvenc => "NVIDIA NVENC",
        EncoderBackend::Amf => "AMD AMF",
        EncoderBackend::QuickSync => "Intel Quick Sync",
        EncoderBackend::VideoToolbox => "Apple VideoToolbox",
        EncoderBackend::SvtAv1 => "Software",
        EncoderBackend::MfSoftware => "Software",
    };
    let codec = match candidate.codec {
        Codec::Av1 => "AV1",
        Codec::Hevc => "HEVC",
        Codec::H264 => "H.264",
    };
    format!("{backend} · {codec}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_encoder_id_matches_serde_serialization() {
        // The Settings dropdown sends EncoderOption.id; settings.rs maps it
        // back through VideoEncoder's snake_case serde. id() must stay in
        // lockstep with that derive, including the new codec variants.
        for enc in [
            VideoEncoder::Auto,
            VideoEncoder::NvencH264,
            VideoEncoder::NvencHevc,
            VideoEncoder::NvencAv1,
            VideoEncoder::AmfH264,
            VideoEncoder::AmfHevc,
            VideoEncoder::AmfAv1,
            VideoEncoder::QuickSyncH264,
            VideoEncoder::QuickSyncHevc,
            VideoEncoder::QuickSyncAv1,
            VideoEncoder::VideoToolboxH264,
            VideoEncoder::SvtAv1,
        ] {
            let serialized = serde_json::to_string(&enc).unwrap();
            assert_eq!(serialized, format!("\"{}\"", enc.id()));
        }
    }

    #[test]
    fn from_parts_round_trips_through_preference() {
        // Every explicit option maps back to the same (backend, codec).
        for (backend, codec) in [
            (EncoderBackend::Amf, Codec::Hevc),
            (EncoderBackend::Nvenc, Codec::Av1),
            (EncoderBackend::VideoToolbox, Codec::H264),
            (EncoderBackend::SvtAv1, Codec::Av1),
        ] {
            let enc = VideoEncoder::from_parts(backend, codec).unwrap();
            assert_eq!(
                enc.preference(),
                EncoderPreference::Explicit { backend, codec }
            );
        }
        assert!(VideoEncoder::from_parts(EncoderBackend::MfSoftware, Codec::H264).is_none());
        assert!(VideoEncoder::from_parts(EncoderBackend::SvtAv1, Codec::H264).is_none());
    }

    #[test]
    fn video_toolbox_h264_has_stable_settings_id() {
        let enc = VideoEncoder::VideoToolboxH264;
        assert_eq!(enc.id(), "video_toolbox_h264");
        assert_eq!(
            VideoEncoder::from_parts(EncoderBackend::VideoToolbox, Codec::H264),
            Some(VideoEncoder::VideoToolboxH264)
        );
    }
}
