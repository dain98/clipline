//! Data model types for persisted settings: capture, audio, video quality,
//! replay storage. Each type owns its `Default`, serde defaults, and pure
//! conversion methods. Field-extractor-based loading lives here because the
//! types own their field mapping; the extractors themselves are in
//! `super::persistence`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::service::{
    AudioChannelMode, AudioOptions, CaptureRegion, OutputResolution, ReplayStorageOptions,
};

use super::persistence::{
    bool_field, clamp_u32, deserialize_field, f64_field, i32_field, integer_field,
    normalize_replay_cache_dir, optional_string_field, replay_cache_quota_bytes_from_gb,
};
use super::validation::{MAX_AUDIO_VOLUME, MAX_CAPTURE_REGION_SIDE, MIN_CAPTURE_REGION_SIDE};

pub const MAX_ICON_DATA_URL_LEN: usize = 256 * 1024;

fn default_enabled() -> bool {
    true
}

fn default_volume() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    PrimaryMonitor,
    WindowTitle,
    DisplayRegion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureRegionSettings {
    pub display_id: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for CaptureRegionSettings {
    fn default() -> Self {
        Self {
            display_id: None,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }
    }
}

impl CaptureRegionSettings {
    pub(crate) fn load_from_value(value: Option<&Value>) -> Self {
        let defaults = Self::default();
        let Some(object) = value.and_then(Value::as_object) else {
            return defaults;
        };

        Self {
            display_id: optional_string_field(object, "display_id").unwrap_or(defaults.display_id),
            x: i32_field(object, "x").unwrap_or(defaults.x),
            y: i32_field(object, "y").unwrap_or(defaults.y),
            width: integer_field(object, "width")
                .map(|value| clamp_u32(value, MIN_CAPTURE_REGION_SIDE, MAX_CAPTURE_REGION_SIDE))
                .unwrap_or(defaults.width),
            height: integer_field(object, "height")
                .map(|value| clamp_u32(value, MIN_CAPTURE_REGION_SIDE, MAX_CAPTURE_REGION_SIDE))
                .unwrap_or(defaults.height),
        }
    }

    pub fn to_service_region(&self) -> CaptureRegion {
        CaptureRegion {
            display_id: self.display_id.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioSettings {
    #[serde(default = "default_enabled")]
    pub output_enabled: bool,
    #[serde(default)]
    pub output_device_id: Option<String>,
    #[serde(default = "default_volume")]
    pub output_volume: f64,
    #[serde(default)]
    pub split_output_by_process: bool,
    #[serde(default)]
    pub mic_enabled: bool,
    #[serde(default)]
    pub mic_device_id: Option<String>,
    #[serde(default = "default_volume")]
    pub mic_volume: f64,
    #[serde(default)]
    pub mic_channels: AudioChannelMode,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            output_enabled: true,
            output_device_id: None,
            output_volume: 1.0,
            split_output_by_process: false,
            mic_enabled: false,
            mic_device_id: None,
            mic_volume: 1.0,
            mic_channels: AudioChannelMode::Mono,
        }
    }
}

impl AudioSettings {
    pub(crate) fn load_from_value(value: Option<&Value>) -> Self {
        let defaults = Self::default();
        let Some(object) = value.and_then(Value::as_object) else {
            return defaults;
        };

        Self {
            output_enabled: bool_field(object, "output_enabled").unwrap_or(defaults.output_enabled),
            output_device_id: optional_string_field(object, "output_device_id")
                .unwrap_or(defaults.output_device_id),
            output_volume: f64_field(object, "output_volume")
                .map(|value| value.clamp(0.0, MAX_AUDIO_VOLUME))
                .unwrap_or(defaults.output_volume),
            split_output_by_process: bool_field(object, "split_output_by_process")
                .unwrap_or(defaults.split_output_by_process),
            mic_enabled: bool_field(object, "mic_enabled").unwrap_or(defaults.mic_enabled),
            mic_device_id: optional_string_field(object, "mic_device_id")
                .unwrap_or(defaults.mic_device_id),
            mic_volume: f64_field(object, "mic_volume")
                .map(|value| value.clamp(0.0, MAX_AUDIO_VOLUME))
                .unwrap_or(defaults.mic_volume),
            mic_channels: deserialize_field(object, "mic_channels")
                .unwrap_or(defaults.mic_channels),
        }
    }

    pub fn to_service_options(&self) -> AudioOptions {
        AudioOptions {
            output_enabled: self.output_enabled,
            output_device_id: self
                .output_device_id
                .clone()
                .filter(|id| !id.trim().is_empty()),
            output_volume: self.output_volume,
            split_output_by_process: self.split_output_by_process,
            mic_enabled: self.mic_enabled,
            mic_device_id: self
                .mic_device_id
                .clone()
                .filter(|id| !id.trim().is_empty()),
            mic_volume: self.mic_volume,
            mic_channels: self.mic_channels,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    Compact,
    #[default]
    Balanced,
    Sharp,
    Maximum,
}

impl VideoQuality {
    pub fn bitrate_mbps(self, resolution: OutputResolution) -> f64 {
        let table = match resolution {
            OutputResolution::Source | OutputResolution::P1440 => [6.0, 12.0, 24.0, 40.0],
            OutputResolution::P1080 => [4.0, 8.0, 16.0, 24.0],
            OutputResolution::P720 => [2.5, 5.0, 8.0, 12.0],
            OutputResolution::P480 => [1.5, 3.0, 5.0, 8.0],
        };
        match self {
            Self::Compact => table[0],
            Self::Balanced => table[1],
            Self::Sharp => table[2],
            Self::Maximum => table[3],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStorageMode {
    #[default]
    Memory,
    Disk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayStorageSettings {
    #[serde(default)]
    pub mode: ReplayStorageMode,
    #[serde(default)]
    pub disk_dir: String,
    #[serde(default = "default_replay_cache_quota_gb")]
    pub disk_quota_gb: f64,
    #[serde(default)]
    pub disk_acknowledged: bool,
}

fn default_replay_cache_quota_gb() -> f64 {
    super::DEFAULT_REPLAY_CACHE_QUOTA_GB
}

impl Default for ReplayStorageSettings {
    fn default() -> Self {
        Self {
            mode: ReplayStorageMode::Memory,
            disk_dir: String::new(),
            disk_quota_gb: default_replay_cache_quota_gb(),
            disk_acknowledged: false,
        }
    }
}

impl ReplayStorageSettings {
    pub fn to_service_options(&self) -> Result<ReplayStorageOptions, String> {
        match self.mode {
            ReplayStorageMode::Memory => Ok(ReplayStorageOptions::Memory),
            ReplayStorageMode::Disk => Ok(ReplayStorageOptions::Disk {
                dir: normalize_replay_cache_dir(&self.disk_dir)?,
                quota_bytes: replay_cache_quota_bytes_from_gb(self.disk_quota_gb)?,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CustomGameSettings {
    pub id: String,
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub exe_name: String,
    #[serde(default)]
    pub process_path: Option<String>,
    #[serde(default)]
    pub window_title: String,
    #[serde(default)]
    pub recording_mode: super::games::GameRecordingMode,
    #[serde(default)]
    pub icon: Option<String>,
}

impl CustomGameSettings {
    pub fn normalize(&mut self) {
        self.id = self.id.trim().to_string();
        self.name = self.name.trim().to_string();
        self.exe_name = self.exe_name.trim().to_string();
        self.window_title = self.window_title.trim().to_string();
        self.process_path = self
            .process_path
            .take()
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
        self.icon = self
            .icon
            .take()
            .filter(|icon| icon.starts_with("data:image/") && icon.len() <= MAX_ICON_DATA_URL_LEN);
    }

    pub fn has_match_identity(&self) -> bool {
        !self.exe_name.trim().is_empty()
            || self
                .process_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
            || !self.window_title.trim().is_empty()
    }
}
