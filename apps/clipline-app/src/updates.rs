use serde::{Deserialize, Serialize};

pub const NIGHTLY_UPDATE_ENDPOINT: &str =
    "https://github.com/dain98/clipline/releases/download/nightly/latest.json";
pub const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/dain98/clipline/releases/latest/download/latest.json";

// Flip this once Clipline has stable, non-prerelease GitHub releases.
pub const STABLE_CHANNEL_ENABLED: bool = false;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    #[default]
    Nightly,
}

impl UpdateChannel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Nightly => "Nightly",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => STABLE_UPDATE_ENDPOINT,
            Self::Nightly => NIGHTLY_UPDATE_ENDPOINT,
        }
    }

    pub fn enabled(self) -> bool {
        match self {
            Self::Stable => STABLE_CHANNEL_ENABLED,
            Self::Nightly => true,
        }
    }
}

pub fn normalize_channel(channel: UpdateChannel) -> UpdateChannel {
    if channel.enabled() {
        channel
    } else {
        UpdateChannel::Nightly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_channel_is_modeled_but_disabled_for_now() {
        assert!(!UpdateChannel::Stable.enabled());
        assert_eq!(
            normalize_channel(UpdateChannel::Stable),
            UpdateChannel::Nightly
        );
    }

    #[test]
    fn nightly_channel_points_at_fixed_github_prerelease_endpoint() {
        assert_eq!(
            UpdateChannel::Nightly.endpoint(),
            "https://github.com/dain98/clipline/releases/download/nightly/latest.json"
        );
    }
}
