//! Persisted application settings and mapping to recorder service options.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri_plugin_global_shortcut::Shortcut;

use crate::service::{
    default_clips_dir, AudioChannelMode, AudioOptions, CaptureRegion, CaptureSource,
    OutputResolution, RecordingMode, ReplayStorageOptions, ServiceOptions, VideoEncoder,
};
use crate::updates::{normalize_channel, UpdateChannel};

const MAX_REPLAY_WINDOW_S: f64 = 120.0;
const MIN_REPLAY_WINDOW_S: f64 = 5.0;
const MIN_BUFFER_SECONDS: f64 = 10.0;
const MAX_BUFFER_SECONDS: f64 = 20.0 * 60.0;
const MIN_BITRATE_MBPS: f64 = 1.0;
const MAX_BITRATE_MBPS: f64 = 100.0;
const MIN_AUDIO_VOLUME: f64 = 0.0;
const MAX_AUDIO_VOLUME: f64 = 2.0;
const MIN_CAPTURE_REGION_SIDE: u32 = 2;
const MAX_CAPTURE_REGION_SIDE: u32 = 16_384;

/// The replay ring holds the save window plus this margin (for keyframe
/// alignment and eviction timing). Sizing the ring to the window - rather than
/// a fixed 2 minutes - keeps memory proportional to what is actually saved.
pub const BUFFER_HEADROOM_S: f64 = 15.0;
const DEFAULT_REPLAY_CACHE_QUOTA_GB: f64 = 2.0;

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
    fn load_from_value(value: Option<&Value>) -> Self {
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

    fn to_service_region(&self) -> CaptureRegion {
        CaptureRegion {
            display_id: self.display_id.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

fn default_enabled() -> bool {
    true
}

/// Tolerate an unknown `video_encoder` value — a hand-edit, or a downgrade
/// from a future build that adds an HEVC/AV1 option — by falling back to Auto
/// instead of failing the whole-file parse. Mirrors how `hotkey` is repaired
/// in `load_from`; reuses `VideoEncoder`'s own snake_case serde so the names
/// can't drift from the enum.
fn deserialize_video_encoder<'de, D>(deserializer: D) -> Result<VideoEncoder, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or(VideoEncoder::Auto))
}

fn default_volume() -> f64 {
    1.0
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

/// Guard against a pathological icon bloating settings.json. A 32x32 RGBA PNG
/// data URL is a few KB; this leaves generous headroom for larger icons.
const MAX_ICON_DATA_URL_LEN: usize = 256 * 1024;

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
    pub recording_mode: GameRecordingMode,
    /// Icon extracted from the game's executable, as a PNG `data:` URL. Shown
    /// in the custom-games list and on the game's clips.
    #[serde(default)]
    pub icon: Option<String>,
}

impl CustomGameSettings {
    fn normalize(&mut self) {
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

    fn has_match_identity(&self) -> bool {
        !self.exe_name.trim().is_empty()
            || self
                .process_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
            || !self.window_title.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameRecordingMode {
    FullSession,
    #[default]
    ReplaysOnly,
}

impl From<GameRecordingMode> for RecordingMode {
    fn from(value: GameRecordingMode) -> Self {
        match value {
            GameRecordingMode::FullSession => Self::FullSession,
            GameRecordingMode::ReplaysOnly => Self::ReplaysOnly,
        }
    }
}

fn default_game_recording_mode_full_session() -> GameRecordingMode {
    GameRecordingMode::FullSession
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GamePluginSettings {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_game_recording_mode_full_session")]
    pub recording_mode: GameRecordingMode,
}

impl Default for GamePluginSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            recording_mode: GameRecordingMode::FullSession,
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
    fn bitrate_mbps(self, resolution: OutputResolution) -> f64 {
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GameSettings {
    #[serde(default = "default_enabled")]
    pub auto_detect: bool,
    #[serde(default)]
    pub plugins: BTreeMap<String, GamePluginSettings>,
    #[serde(default)]
    pub custom_games: Vec<CustomGameSettings>,
}

#[derive(Deserialize)]
struct GameSettingsWire {
    #[serde(default = "default_enabled")]
    auto_detect: bool,
    #[serde(default)]
    plugins: BTreeMap<String, GamePluginSettings>,
    #[serde(default, rename = "recording_mode")]
    legacy_recording_mode: Option<GameRecordingMode>,
    #[serde(default)]
    custom_games: Vec<CustomGameSettings>,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            auto_detect: true,
            plugins: BTreeMap::new(),
            custom_games: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for GameSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut wire = GameSettingsWire::deserialize(deserializer)?;
        if let Some(mode) = wire.legacy_recording_mode {
            for game in &mut wire.custom_games {
                game.recording_mode = mode;
            }
        }
        Ok(Self {
            auto_detect: wire.auto_detect,
            plugins: wire.plugins,
            custom_games: wire.custom_games,
        })
    }
}

impl GameSettings {
    fn normalize(&mut self) {
        self.plugins = std::mem::take(&mut self.plugins)
            .into_iter()
            .map(|(id, settings)| (normalize_game_plugin_id(&id), settings))
            .filter(|(id, _)| !id.is_empty())
            .collect();
        for game in &mut self.custom_games {
            game.normalize();
        }
    }
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
    fn load_from_value(value: Option<&Value>) -> Self {
        let defaults = Self::default();
        let Some(object) = value.and_then(Value::as_object) else {
            return defaults;
        };

        Self {
            output_enabled: bool_field(object, "output_enabled").unwrap_or(defaults.output_enabled),
            output_device_id: optional_string_field(object, "output_device_id")
                .unwrap_or(defaults.output_device_id),
            output_volume: f64_field(object, "output_volume")
                .map(|value| value.clamp(MIN_AUDIO_VOLUME, MAX_AUDIO_VOLUME))
                .unwrap_or(defaults.output_volume),
            split_output_by_process: bool_field(object, "split_output_by_process")
                .unwrap_or(defaults.split_output_by_process),
            mic_enabled: bool_field(object, "mic_enabled").unwrap_or(defaults.mic_enabled),
            mic_device_id: optional_string_field(object, "mic_device_id")
                .unwrap_or(defaults.mic_device_id),
            mic_volume: f64_field(object, "mic_volume")
                .map(|value| value.clamp(MIN_AUDIO_VOLUME, MAX_AUDIO_VOLUME))
                .unwrap_or(defaults.mic_volume),
            mic_channels: deserialize_field(object, "mic_channels")
                .unwrap_or(defaults.mic_channels),
        }
    }

    fn to_service_options(&self) -> AudioOptions {
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
    fn to_service_options(&self) -> Result<ReplayStorageOptions, String> {
        match self.mode {
            ReplayStorageMode::Memory => Ok(ReplayStorageOptions::Memory),
            ReplayStorageMode::Disk => Ok(ReplayStorageOptions::Disk {
                dir: normalize_replay_cache_dir(&self.disk_dir)?,
                quota_bytes: replay_cache_quota_bytes_from_gb(self.disk_quota_gb)?,
            }),
        }
    }
}

fn default_cloud_visibility() -> String {
    "private".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudUploadRecord {
    pub local_clip_id: String,
    pub path: String,
    #[serde(default)]
    pub remote_clip_id: Option<String>,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default = "default_cloud_visibility")]
    pub visibility: String,
    #[serde(default = "default_upload_status")]
    pub upload_status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updated_at_unix: u64,
}

fn default_upload_status() -> String {
    "not_uploaded".to_string()
}

impl CloudUploadRecord {
    fn normalize(&mut self) {
        self.local_clip_id = self.local_clip_id.trim().to_string();
        self.path = self.path.trim().to_string();
        self.remote_clip_id = self
            .remote_clip_id
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.remote_url = self
            .remote_url
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.visibility = normalize_cloud_visibility(&self.visibility);
        self.upload_status = normalize_upload_status(&self.upload_status);
        self.error = self
            .error
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloudSettings {
    #[serde(default)]
    pub host_url: String,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub connected_user_id: Option<String>,
    #[serde(default)]
    pub connected_username: Option<String>,
    #[serde(default)]
    pub credential_target: Option<String>,
    #[serde(default = "default_cloud_visibility")]
    pub default_visibility: String,
    #[serde(default)]
    pub delete_local_after_upload: bool,
    #[serde(default)]
    pub auto_upload_rules: bool,
    #[serde(default)]
    pub uploads: BTreeMap<String, CloudUploadRecord>,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            host_url: String::new(),
            public_url: None,
            connected_user_id: None,
            connected_username: None,
            credential_target: None,
            default_visibility: default_cloud_visibility(),
            delete_local_after_upload: false,
            auto_upload_rules: false,
            uploads: BTreeMap::new(),
        }
    }
}

impl CloudSettings {
    pub fn connected(&self) -> bool {
        !self.host_url.trim().is_empty()
            && self.connected_user_id.is_some()
            && self.credential_target.is_some()
    }

    pub fn normalize(&mut self) {
        self.host_url = self.host_url.trim().trim_end_matches('/').to_string();
        self.public_url = self
            .public_url
            .take()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty());
        self.connected_user_id = self
            .connected_user_id
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.connected_username = self
            .connected_username
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.credential_target = self
            .credential_target
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.default_visibility = normalize_cloud_visibility(&self.default_visibility);
        self.uploads = std::mem::take(&mut self.uploads)
            .into_iter()
            .filter_map(|(key, mut record)| {
                record.normalize();
                (!record.local_clip_id.is_empty())
                    .then(|| (normalize_cloud_upload_key(&key, &record), record))
            })
            .collect();
    }

    fn validate(&self) -> Result<(), String> {
        validate_cloud_visibility(&self.default_visibility)?;
        for record in self.uploads.values() {
            validate_cloud_visibility(&record.visibility)?;
            validate_upload_status(&record.upload_status)?;
            if record.local_clip_id.trim().is_empty() {
                return Err("cloud upload record is missing local_clip_id".into());
            }
        }
        Ok(())
    }
}

fn normalize_cloud_upload_key(key: &str, record: &CloudUploadRecord) -> String {
    let key = key.trim();
    if key.is_empty() {
        record.local_clip_id.clone()
    } else {
        key.to_string()
    }
}

pub fn normalize_cloud_visibility(value: &str) -> String {
    match value {
        "public" => "public".to_string(),
        "unlisted" => "unlisted".to_string(),
        _ => "private".to_string(),
    }
}

fn validate_cloud_visibility(value: &str) -> Result<(), String> {
    match value {
        "private" | "public" | "unlisted" => Ok(()),
        _ => Err("cloud visibility must be private, public, or unlisted".into()),
    }
}

fn normalize_upload_status(value: &str) -> String {
    match value {
        "queued" | "uploading" | "processing" | "uploaded_private" | "uploaded_public"
        | "failed" | "retrying" => value.to_string(),
        _ => default_upload_status(),
    }
}

fn validate_upload_status(value: &str) -> Result<(), String> {
    match value {
        "not_uploaded" | "queued" | "uploading" | "processing" | "uploaded_private"
        | "uploaded_public" | "failed" | "retrying" => Ok(()),
        _ => Err("cloud upload status is invalid".into()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub capture_mode: CaptureMode,
    pub window_title: String,
    #[serde(default)]
    pub capture_region: CaptureRegionSettings,
    #[serde(default)]
    pub games: GameSettings,
    #[serde(default)]
    pub audio: AudioSettings,
    pub buffer_seconds: f64,
    pub replay_window_s: f64,
    #[serde(default)]
    pub video_quality: VideoQuality,
    pub bitrate_mbps: f64,
    pub fps: u32,
    #[serde(default, deserialize_with = "deserialize_video_encoder")]
    pub video_encoder: VideoEncoder,
    #[serde(default)]
    pub output_resolution: OutputResolution,
    pub disk_quota_gb: f64,
    #[serde(default = "default_media_dir")]
    pub media_dir: String,
    #[serde(default)]
    pub replay_storage: ReplayStorageSettings,
    pub hotkey: String,
    #[serde(default)]
    pub open_on_startup: bool,
    #[serde(default = "default_enabled")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub minimize_to_tray: bool,
    #[serde(default = "default_enabled")]
    pub capture_preview_enabled: bool,
    #[serde(default)]
    pub update_channel: UpdateChannel,
    #[serde(default)]
    pub cloud: CloudSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            capture_mode: CaptureMode::PrimaryMonitor,
            window_title: String::new(),
            capture_region: CaptureRegionSettings::default(),
            games: GameSettings::default(),
            audio: AudioSettings::default(),
            buffer_seconds: 60.0 + BUFFER_HEADROOM_S,
            replay_window_s: 60.0,
            video_quality: VideoQuality::Balanced,
            bitrate_mbps: 12.0,
            fps: 60,
            video_encoder: VideoEncoder::Auto,
            output_resolution: OutputResolution::Source,
            disk_quota_gb: 10.0,
            media_dir: default_media_dir(),
            replay_storage: ReplayStorageSettings::default(),
            hotkey: "Alt+F10".into(),
            open_on_startup: false,
            close_to_tray: true,
            minimize_to_tray: false,
            capture_preview_enabled: true,
            update_channel: UpdateChannel::Nightly,
            cloud: CloudSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.capture_mode, CaptureMode::WindowTitle)
            && self.window_title.trim().is_empty()
        {
            return Err("window title is required for window capture".into());
        }
        if matches!(self.capture_mode, CaptureMode::DisplayRegion) {
            if self.capture_region.width < MIN_CAPTURE_REGION_SIDE
                || self.capture_region.height < MIN_CAPTURE_REGION_SIDE
            {
                return Err("capture region must be at least 2x2 pixels".into());
            }
            if self.capture_region.width > MAX_CAPTURE_REGION_SIDE
                || self.capture_region.height > MAX_CAPTURE_REGION_SIDE
            {
                return Err("capture region is too large".into());
            }
        }
        self.validate_games()?;
        validate_range(
            "output volume",
            self.audio.output_volume,
            MIN_AUDIO_VOLUME,
            MAX_AUDIO_VOLUME,
        )?;
        validate_range(
            "microphone volume",
            self.audio.mic_volume,
            MIN_AUDIO_VOLUME,
            MAX_AUDIO_VOLUME,
        )?;
        validate_range(
            "buffer seconds",
            self.buffer_seconds,
            MIN_BUFFER_SECONDS,
            MAX_BUFFER_SECONDS,
        )?;
        validate_range(
            "replay seconds",
            self.replay_window_s,
            MIN_REPLAY_WINDOW_S,
            MAX_REPLAY_WINDOW_S,
        )?;
        if self.replay_window_s > self.buffer_seconds {
            return Err("replay seconds cannot be longer than buffer seconds".into());
        }
        validate_range(
            "bitrate Mbps",
            self.effective_bitrate_mbps(),
            MIN_BITRATE_MBPS,
            MAX_BITRATE_MBPS,
        )?;
        if !matches!(self.fps, 30 | 60 | 90 | 120) {
            return Err("fps must be 30, 60, 90, or 120".into());
        }
        if !self.update_channel.enabled() {
            return Err(format!(
                "{} update channel is not available yet",
                self.update_channel.label()
            ));
        }
        quota_bytes_from_gb(self.disk_quota_gb)?;
        self.media_dir_path()?;
        self.validate_replay_storage()?;
        normalize_hotkey(&self.hotkey)?;
        self.cloud.validate()?;
        Ok(())
    }

    pub fn media_dir_path(&self) -> Result<PathBuf, String> {
        normalize_media_dir(&self.media_dir)
    }

    pub fn to_service_options(&self, lol_url: Option<String>) -> Result<ServiceOptions, String> {
        self.validate()?;
        Ok(ServiceOptions {
            capture_source: match self.capture_mode {
                CaptureMode::PrimaryMonitor => CaptureSource::PrimaryMonitor,
                CaptureMode::WindowTitle => {
                    CaptureSource::WindowTitle(self.window_title.trim().to_string())
                }
                CaptureMode::DisplayRegion => {
                    CaptureSource::DisplayRegion(self.capture_region.to_service_region())
                }
            },
            active_game_plugin_id: None,
            active_game: None,
            media_dir: self.media_dir_path()?,
            lol_url,
            replay_window_s: self.replay_window_s,
            buffer_bytes: estimated_buffer_bytes(
                replay_buffer_seconds(self),
                self.effective_bitrate_mbps(),
            ),
            replay_storage: self.replay_storage.to_service_options()?,
            disk_quota_bytes: quota_bytes_from_gb(self.disk_quota_gb)?,
            recording_mode: RecordingMode::ReplaysOnly,
            fps: self.fps,
            bitrate_bps: (self.effective_bitrate_mbps() * 1_000_000.0).round() as u32,
            video_encoder: self.video_encoder,
            output_resolution: self.output_resolution,
            // Codecs the in-app player can decode are reported by the frontend
            // at spawn (see app.rs); H.264 is the always-safe default so Auto
            // never records an unplayable clip if that probe hasn't run.
            decodable_codecs: vec![clipline_capture::probe::Codec::H264],
            audio: self.audio.to_service_options(),
        })
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let object = value
            .as_object()
            .ok_or_else(|| "settings file must be a JSON object".to_string())?;
        let settings = Self::load_from_object(object);
        settings.validate()?;
        Ok(settings)
    }

    fn load_from_object(object: &Map<String, Value>) -> Self {
        let defaults = Self::default();
        let output_resolution =
            deserialize_field(object, "output_resolution").unwrap_or(defaults.output_resolution);
        let legacy_bitrate_mbps = f64_field(object, "bitrate_mbps")
            .map(|value| value.clamp(MIN_BITRATE_MBPS, MAX_BITRATE_MBPS))
            .unwrap_or(defaults.bitrate_mbps);
        let video_quality = deserialize_field(object, "video_quality").unwrap_or_else(|| {
            repair_video_quality_from_legacy_bitrate(legacy_bitrate_mbps, output_resolution)
        });
        let mut settings = Self {
            capture_mode: deserialize_field(object, "capture_mode")
                .unwrap_or_else(|| defaults.capture_mode.clone()),
            window_title: string_field(object, "window_title")
                .unwrap_or_else(|| defaults.window_title.clone()),
            capture_region: CaptureRegionSettings::load_from_value(object.get("capture_region")),
            games: deserialize_field(object, "games").unwrap_or_default(),
            audio: AudioSettings::load_from_value(object.get("audio")),
            buffer_seconds: defaults.buffer_seconds,
            replay_window_s: f64_field(object, "replay_window_s")
                .map(|value| value.clamp(MIN_REPLAY_WINDOW_S, MAX_REPLAY_WINDOW_S))
                .unwrap_or(defaults.replay_window_s),
            video_quality,
            bitrate_mbps: legacy_bitrate_mbps,
            fps: integer_field(object, "fps")
                .map(repair_fps)
                .unwrap_or(defaults.fps),
            video_encoder: deserialize_field(object, "video_encoder")
                .unwrap_or(defaults.video_encoder),
            output_resolution,
            disk_quota_gb: f64_field(object, "disk_quota_gb")
                .map(repair_disk_quota_gb)
                .unwrap_or(defaults.disk_quota_gb),
            media_dir: string_field(object, "media_dir")
                .and_then(|raw| normalize_media_dir(&raw).ok())
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| defaults.media_dir.clone()),
            // Added after this PR was written; load it the same repair-or-default
            // way (a malformed value falls back to the default instead of failing
            // the whole-file parse).
            replay_storage: deserialize_field(object, "replay_storage").unwrap_or_default(),
            hotkey: string_field(object, "hotkey")
                .and_then(|raw| normalize_hotkey(&raw).ok())
                .unwrap_or_else(|| defaults.hotkey.clone()),
            open_on_startup: bool_field(object, "open_on_startup")
                .unwrap_or(defaults.open_on_startup),
            close_to_tray: bool_field(object, "close_to_tray").unwrap_or(defaults.close_to_tray),
            minimize_to_tray: bool_field(object, "minimize_to_tray")
                .unwrap_or(defaults.minimize_to_tray),
            capture_preview_enabled: bool_field(object, "capture_preview_enabled")
                .unwrap_or(defaults.capture_preview_enabled),
            update_channel: deserialize_field(object, "update_channel")
                .map(normalize_channel)
                .unwrap_or(defaults.update_channel),
            cloud: deserialize_field(object, "cloud").unwrap_or_default(),
        };

        settings.games.normalize();
        settings.cloud.normalize();
        // Size the ring to the replay window (+ headroom), not whatever was
        // persisted. This migrates old fixed 120 s buffers down and keeps the
        // recording footprint proportional to what a save actually needs.
        settings.buffer_seconds = replay_buffer_seconds(&settings);
        settings.bitrate_mbps = settings.effective_bitrate_mbps();
        if matches!(settings.capture_mode, CaptureMode::WindowTitle)
            && settings.window_title.trim().is_empty()
        {
            settings.capture_mode = defaults.capture_mode;
        }
        settings
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut settings = self.clone();
        settings.hotkey = normalize_hotkey(&settings.hotkey)?;
        settings.games.normalize();
        settings.cloud.normalize();
        settings.media_dir = settings.media_dir_path()?.display().to_string();
        settings.bitrate_mbps = settings.effective_bitrate_mbps();
        if matches!(settings.replay_storage.mode, ReplayStorageMode::Disk) {
            settings.replay_storage.disk_dir =
                normalize_replay_cache_dir(&settings.replay_storage.disk_dir)?
                    .display()
                    .to_string();
        }
        settings.buffer_seconds = replay_buffer_seconds(&settings);
        settings.validate()?;
        let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn load_or_default() -> Self {
        Self::load_from(&settings_path()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings_path())
    }

    fn validate_replay_storage(&self) -> Result<(), String> {
        if matches!(self.replay_storage.mode, ReplayStorageMode::Memory) {
            return Ok(());
        }
        if !self.replay_storage.disk_acknowledged {
            return Err("disk replay buffer requires acknowledging SSD wear".into());
        }
        replay_cache_quota_bytes_from_gb(self.replay_storage.disk_quota_gb)?;
        let cache_dir = normalize_replay_cache_dir(&self.replay_storage.disk_dir)?;
        let media_dir = self.media_dir_path()?;
        if same_or_nested_path(&cache_dir, &media_dir)
            || same_or_nested_path(&media_dir, &cache_dir)
        {
            return Err("replay cache folder must be separate from the media folder".into());
        }
        Ok(())
    }

    fn validate_games(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        for game in &self.games.custom_games {
            let id = game.id.trim();
            if id.is_empty() {
                return Err("custom game id is required".into());
            }
            if !ids.insert(id.to_ascii_lowercase()) {
                return Err(format!("custom game id {id:?} is duplicated"));
            }
            if game.name.trim().is_empty() {
                return Err("custom game name is required".into());
            }
            if !game.has_match_identity() {
                return Err(format!(
                    "custom game {:?} needs a process or window identity",
                    game.name
                ));
            }
        }
        Ok(())
    }

    fn effective_bitrate_mbps(&self) -> f64 {
        self.video_quality.bitrate_mbps(self.output_resolution)
    }
}

pub fn parse_hotkey(raw: &str) -> Result<Shortcut, String> {
    normalize_hotkey(raw)?
        .parse::<Shortcut>()
        .map_err(|e| format!("hotkey: {e}"))
}

pub fn normalize_hotkey(raw: &str) -> Result<String, String> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = None::<u8>;

    for part in raw.split('+') {
        let token = part.trim();
        if token.is_empty() {
            return Err("hotkey has an empty part".into());
        }
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => set_once(&mut ctrl, "Ctrl")?,
            "alt" => set_once(&mut alt, "Alt")?,
            "shift" => set_once(&mut shift, "Shift")?,
            other if other.starts_with('f') => {
                if key.is_some() {
                    return Err("hotkey has more than one key".into());
                }
                let n = other[1..]
                    .parse::<u8>()
                    .map_err(|_| "hotkey key must be F1-F11 or F13-F24")?;
                if !(1..=24).contains(&n) {
                    return Err("hotkey key must be F1-F11 or F13-F24".into());
                }
                if n == 12 {
                    return Err("F12 is reserved by Windows for debuggers".into());
                }
                key = Some(n);
            }
            _ => {
                return Err(
                    "hotkey must use optional Ctrl, Alt, Shift, and F1-F11 or F13-F24".into(),
                )
            }
        }
    }

    let key = key.ok_or("hotkey needs an F-key")?;

    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    parts.push(format!("F{key}"));
    Ok(parts.join("+"))
}

pub fn quota_bytes_from_gb(gb: f64) -> Result<Option<u64>, String> {
    const GIB_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

    if !gb.is_finite() || gb < 0.0 {
        return Err("disk quota must be a non-negative finite number".into());
    }
    if gb == 0.0 {
        return Ok(None);
    }
    let bytes = gb * GIB_BYTES;
    if bytes > u64::MAX as f64 {
        return Err("disk quota is too large".into());
    }
    Ok(Some(bytes.round() as u64))
}

fn config_base() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join("AppData").join("Roaming"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("Clipline")
}

pub fn settings_path() -> PathBuf {
    config_base().join("settings.json")
}

/// Where extracted plugin icons are cached (the bundled-icon fallback).
pub fn icon_cache_dir() -> PathBuf {
    config_base().join("icons")
}

pub fn audio_preview_cache_dir() -> PathBuf {
    config_base().join("audio-previews")
}

pub fn default_media_dir() -> String {
    default_clips_dir().display().to_string()
}

pub fn normalize_media_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("media folder is required".into());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("media folder must be an absolute path".into());
    }
    Ok(path)
}

fn normalize_game_plugin_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn deserialize_field<T>(object: &Map<String, Value>, key: &str) -> Option<T>
where
    T: DeserializeOwned,
{
    object
        .get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn bool_field(object: &Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(Value::as_bool)
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_string_field(object: &Map<String, Value>, key: &str) -> Option<Option<String>> {
    match object.get(key)? {
        Value::Null => Some(None),
        Value::String(value) if value.trim().is_empty() => Some(None),
        Value::String(value) => Some(Some(value.clone())),
        _ => None,
    }
}

fn f64_field(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn integer_field(object: &Map<String, Value>, key: &str) -> Option<i64> {
    let value = object.get(key)?;
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    if let Some(value) = value.as_u64() {
        return Some(value.min(i64::MAX as u64) as i64);
    }
    value.as_f64().and_then(|value| {
        value
            .is_finite()
            .then(|| value.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64)
    })
}

fn i32_field(object: &Map<String, Value>, key: &str) -> Option<i32> {
    integer_field(object, key).map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

fn clamp_u32(value: i64, min: u32, max: u32) -> u32 {
    value.clamp(i64::from(min), i64::from(max)) as u32
}

fn repair_fps(value: i64) -> u32 {
    const FPS_STOPS: [u32; 4] = [30, 60, 90, 120];
    let value = clamp_u32(value, FPS_STOPS[0], *FPS_STOPS.last().unwrap());
    FPS_STOPS
        .into_iter()
        .min_by_key(|candidate| value.abs_diff(*candidate))
        .unwrap_or(AppSettings::default().fps)
}

fn repair_video_quality_from_legacy_bitrate(
    mbps: f64,
    resolution: OutputResolution,
) -> VideoQuality {
    [
        VideoQuality::Compact,
        VideoQuality::Balanced,
        VideoQuality::Sharp,
        VideoQuality::Maximum,
    ]
    .into_iter()
    .min_by(|left, right| {
        let left = left.bitrate_mbps(resolution);
        let right = right.bitrate_mbps(resolution);
        (mbps - left)
            .abs()
            .partial_cmp(&(mbps - right).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
    .unwrap_or_default()
}

fn repair_disk_quota_gb(value: f64) -> f64 {
    const GIB_BYTES: u64 = 1024 * 1024 * 1024;
    value.clamp(0.0, (u64::MAX / GIB_BYTES) as f64)
}

pub fn normalize_replay_cache_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("replay cache folder is required".into());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("replay cache folder must be an absolute path".into());
    }
    Ok(path)
}

pub fn replay_cache_quota_bytes_from_gb(gb: f64) -> Result<u64, String> {
    const GIB_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

    if !gb.is_finite() || gb < 0.25 {
        return Err("replay cache quota must be at least 0.25 GiB".into());
    }
    let bytes = gb * GIB_BYTES;
    if bytes > u64::MAX as f64 {
        return Err("replay cache quota is too large".into());
    }
    Ok(bytes.round() as u64)
}

fn default_replay_cache_quota_gb() -> f64 {
    DEFAULT_REPLAY_CACHE_QUOTA_GB
}

fn validate_range(name: &str, value: f64, min: f64, max: f64) -> Result<(), String> {
    if !value.is_finite() || value < min || value > max {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(())
}

fn set_once(slot: &mut bool, name: &str) -> Result<(), String> {
    if *slot {
        return Err(format!("hotkey repeats {name}"));
    }
    *slot = true;
    Ok(())
}

fn replay_buffer_seconds(settings: &AppSettings) -> f64 {
    settings.replay_window_s + BUFFER_HEADROOM_S
}

fn estimated_buffer_bytes(buffer_seconds: f64, bitrate_mbps: f64) -> usize {
    const MIN_BUFFER_BYTES: f64 = 64.0 * 1024.0 * 1024.0;
    const ENCODER_OVERSHOOT_HEADROOM: f64 = 2.0;

    let video_bytes = bitrate_mbps * 1_000_000.0 / 8.0 * buffer_seconds;
    (video_bytes * ENCODER_OVERSHOOT_HEADROOM).max(MIN_BUFFER_BYTES) as usize
}

fn same_or_nested_path(child: &Path, parent: &Path) -> bool {
    let child = normalize_components(child);
    let parent = normalize_components(parent);
    child == parent || child.starts_with(&parent)
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        out.push(component.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::DEFAULT_DISK_QUOTA_BYTES;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-settings-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_match_current_recorder_behavior() {
        let settings = AppSettings::default();

        assert_eq!(settings.capture_mode, CaptureMode::PrimaryMonitor);
        assert!(settings.games.auto_detect);
        assert!(settings.games.plugins.is_empty());
        assert!(settings.games.custom_games.is_empty());
        assert!(settings.audio.output_enabled);
        assert_eq!(settings.audio.output_device_id, None);
        assert_eq!(settings.audio.output_volume, 1.0);
        assert!(!settings.audio.split_output_by_process);
        let serialized = serde_json::to_value(&settings).unwrap();
        assert_eq!(serialized["audio"]["split_output_by_process"], false);
        assert!(!settings.audio.mic_enabled);
        assert_eq!(settings.audio.mic_device_id, None);
        assert_eq!(settings.audio.mic_volume, 1.0);
        assert_eq!(settings.audio.mic_channels, AudioChannelMode::Mono);
        assert_eq!(settings.replay_window_s, 60.0);
        assert_eq!(settings.buffer_seconds, 75.0);
        assert_eq!(settings.video_quality, VideoQuality::Balanced);
        assert_eq!(settings.bitrate_mbps, 12.0);
        assert_eq!(settings.fps, 60);
        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert_eq!(settings.output_resolution, OutputResolution::Source);
        assert_eq!(settings.disk_quota_gb, 10.0);
        assert_eq!(settings.media_dir, default_media_dir());
        assert_eq!(settings.replay_storage, ReplayStorageSettings::default());
        assert_eq!(settings.hotkey, "Alt+F10");
        assert!(!settings.open_on_startup);
        assert!(settings.close_to_tray);
        assert!(!settings.minimize_to_tray);
        assert!(settings.capture_preview_enabled);
        assert_eq!(settings.update_channel, UpdateChannel::Nightly);
    }

    #[test]
    fn validation_rejects_replay_longer_than_two_minutes() {
        let settings = AppSettings {
            replay_window_s: 121.0,
            buffer_seconds: 300.0,
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn validation_requires_window_title_for_window_capture() {
        let settings = AppSettings {
            capture_mode: CaptureMode::WindowTitle,
            window_title: " ".into(),
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn legacy_settings_default_capture_region() {
        let json = r#"{
            "capture_mode": "primary_monitor",
            "window_title": "",
            "buffer_seconds": 120.0,
            "replay_window_s": 60.0,
            "bitrate_mbps": 12.0,
            "fps": 60,
            "disk_quota_gb": 10.0,
            "hotkey": "Alt+F10"
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(settings.capture_region.width, 1920);
        assert_eq!(settings.capture_region.height, 1080);
        assert_eq!(settings.audio, AudioSettings::default());
        assert_eq!(settings.games, GameSettings::default());
        assert_eq!(settings.media_dir, default_media_dir());
        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert_eq!(settings.output_resolution, OutputResolution::Source);
        assert!(settings.close_to_tray);
        assert!(!settings.minimize_to_tray);
        assert!(settings.capture_preview_enabled);
        assert_eq!(settings.update_channel, UpdateChannel::Nightly);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn load_repairs_disabled_stable_update_channel() {
        let dir = TestDir::new("stable-update-channel");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "replay_window_s": 60.0,
                "bitrate_mbps": 12.0,
                "fps": 60,
                "disk_quota_gb": 10.0,
                "hotkey": "Alt+F10",
                "update_channel": "stable"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.update_channel, UpdateChannel::Nightly);
    }

    #[test]
    fn legacy_custom_games_default_to_replays_only() {
        let json = r#"{
            "capture_mode": "primary_monitor",
            "window_title": "",
            "buffer_seconds": 120.0,
            "replay_window_s": 60.0,
            "bitrate_mbps": 12.0,
            "fps": 60,
            "disk_quota_gb": 10.0,
            "hotkey": "Alt+F10",
            "games": {
                "auto_detect": true,
                "custom_games": [{
                    "id": "custom-test",
                    "name": "Test Game",
                    "enabled": true,
                    "exe_name": "game.exe",
                    "process_path": "C:\\Games\\Test\\game.exe",
                    "window_title": "Test Game"
                }]
            }
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(
            settings.games.custom_games[0].recording_mode,
            GameRecordingMode::ReplaysOnly
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn legacy_global_game_recording_mode_migrates_to_custom_games() {
        let json = r#"{
            "capture_mode": "primary_monitor",
            "window_title": "",
            "buffer_seconds": 120.0,
            "replay_window_s": 60.0,
            "bitrate_mbps": 12.0,
            "fps": 60,
            "disk_quota_gb": 10.0,
            "hotkey": "Alt+F10",
            "games": {
                "auto_detect": true,
                "recording_mode": "full_session",
                "custom_games": [{
                    "id": "custom-test",
                    "name": "Test Game",
                    "enabled": true,
                    "exe_name": "game.exe",
                    "process_path": "C:\\Games\\Test\\game.exe",
                    "window_title": "Test Game"
                }]
            }
        }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(
            settings.games.custom_games[0].recording_mode,
            GameRecordingMode::FullSession
        );
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved["games"].get("recording_mode").is_none());
    }

    #[test]
    fn validation_rejects_relative_media_folder() {
        let settings = AppSettings {
            media_dir: "clips".into(),
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn load_heals_invalid_media_folder_without_resetting_settings() {
        let dir = TestDir::new("heal-media-folder");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 120.0,
                "replay_window_s": 60.0,
                "bitrate_mbps": 24.0,
                "fps": 90,
                "media_dir": "relative/not/allowed",
                "disk_quota_gb": 6.0,
                "hotkey": "Alt+F9"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.media_dir, default_media_dir());
        assert_eq!(settings.bitrate_mbps, 24.0);
        assert_eq!(settings.fps, 90);
        assert_eq!(settings.disk_quota_gb, 6.0);
        assert_eq!(settings.hotkey, "Alt+F9");
    }

    #[test]
    fn legacy_bitrate_migration_uses_output_resolution() {
        let dir = TestDir::new("legacy-quality-resolution");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 120.0,
                "replay_window_s": 60.0,
                "output_resolution": "720p",
                "bitrate_mbps": 5.0,
                "fps": 60,
                "disk_quota_gb": 10.0,
                "hotkey": "Alt+F10"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.output_resolution, OutputResolution::P720);
        assert_eq!(settings.video_quality, VideoQuality::Balanced);
        assert_eq!(settings.bitrate_mbps, 5.0);
        assert_eq!(
            settings.to_service_options(None).unwrap().bitrate_bps,
            5_000_000
        );
    }

    #[test]
    fn validation_rejects_out_of_range_audio_volume() {
        let settings = AppSettings {
            audio: AudioSettings {
                output_volume: 2.1,
                ..AudioSettings::default()
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());

        let settings = AppSettings {
            audio: AudioSettings {
                mic_volume: -0.1,
                ..AudioSettings::default()
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn load_clamps_legacy_replay_window_to_two_minutes() {
        let dir = TestDir::new("clamp-replay-window");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 300.0,
                "replay_window_s": 300.0,
                "bitrate_mbps": 12.0,
                "fps": 60,
                "disk_quota_gb": 10.0,
                "hotkey": "Alt+F10"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.replay_window_s, 120.0);
        // Buffer is recomputed from the (clamped) window + headroom, not kept
        // at the legacy 300 s.
        assert_eq!(settings.buffer_seconds, 120.0 + 15.0);
    }

    #[test]
    fn load_migrates_invalid_legacy_hotkey_without_resetting_settings() {
        let dir = TestDir::new("migrate-hotkey");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "display_region",
                "window_title": "",
                "capture_region": {
                    "display_id": "\\\\.\\DISPLAY2",
                    "x": 1920,
                    "y": 0,
                    "width": 1280,
                    "height": 720
                },
                "buffer_seconds": 120.0,
                "replay_window_s": 60.0,
                "bitrate_mbps": 24.0,
                "fps": 90,
                "disk_quota_gb": 6.0,
                "hotkey": "F12"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.hotkey, "Alt+F10");
        assert_eq!(settings.capture_mode, CaptureMode::DisplayRegion);
        assert_eq!(settings.capture_region.width, 1280);
        assert_eq!(settings.bitrate_mbps, 24.0);
        assert_eq!(settings.fps, 90);
        assert_eq!(settings.disk_quota_gb, 6.0);
    }

    #[test]
    fn load_tolerates_unknown_video_encoder_without_resetting_settings() {
        let dir = TestDir::new("unknown-encoder");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 120.0,
                "replay_window_s": 60.0,
                "bitrate_mbps": 24.0,
                "fps": 90,
                "video_encoder": "hevc_av1_turbo",
                "disk_quota_gb": 6.0,
                "hotkey": "Alt+F9"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert_eq!(settings.bitrate_mbps, 24.0);
        assert_eq!(settings.fps, 90);
        assert_eq!(settings.disk_quota_gb, 6.0);
        assert_eq!(settings.hotkey, "Alt+F9");
    }

    #[test]
    fn load_repairs_invalid_fields_without_resetting_valid_neighbors() {
        let dir = TestDir::new("repair-invalid-fields");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{
                "capture_mode": "future_capture",
                "window_title": "Keep this window title",
                "capture_region": {
                    "display_id": "\\\\.\\DISPLAY2",
                    "x": 100,
                    "y": 200,
                    "width": 1,
                    "height": 50000
                },
                "audio": {
                    "output_enabled": false,
                    "output_device_id": "speaker-id",
                    "output_volume": 2.5,
                    "mic_enabled": true,
                    "mic_device_id": "   ",
                    "mic_volume": -0.25,
                    "mic_channels": "surround"
                },
                "buffer_seconds": 1.0,
                "replay_window_s": 999.0,
                "bitrate_mbps": 250.0,
                "fps": 999,
                "video_encoder": "av1_future",
                "disk_quota_gb": -1.0,
                "media_dir": "",
                "hotkey": "F12"
            }"#,
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.capture_mode, CaptureMode::PrimaryMonitor);
        assert_eq!(settings.window_title, "Keep this window title");
        assert_eq!(
            settings.capture_region.display_id.as_deref(),
            Some(r"\\.\DISPLAY2")
        );
        assert_eq!(settings.capture_region.x, 100);
        assert_eq!(settings.capture_region.y, 200);
        assert_eq!(settings.capture_region.width, 2);
        assert_eq!(settings.capture_region.height, 16_384);
        assert!(!settings.audio.output_enabled);
        assert_eq!(
            settings.audio.output_device_id.as_deref(),
            Some("speaker-id")
        );
        assert_eq!(settings.audio.output_volume, 2.0);
        assert!(settings.audio.mic_enabled);
        assert_eq!(settings.audio.mic_device_id, None);
        assert_eq!(settings.audio.mic_volume, 0.0);
        assert_eq!(settings.audio.mic_channels, AudioChannelMode::Mono);
        assert_eq!(settings.replay_window_s, 120.0);
        assert_eq!(settings.buffer_seconds, 120.0 + 15.0);
        assert_eq!(settings.video_quality, VideoQuality::Maximum);
        assert_eq!(settings.bitrate_mbps, 40.0);
        assert_eq!(settings.fps, 120);
        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert_eq!(settings.disk_quota_gb, 0.0);
        assert_eq!(settings.media_dir, default_media_dir());
        assert_eq!(settings.hotkey, "Alt+F10");
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn display_region_settings_round_trip_json() {
        let dir = TestDir::new("region-round-trip");
        let path = dir.path().join("settings.json");
        let settings = AppSettings {
            capture_mode: CaptureMode::DisplayRegion,
            capture_region: CaptureRegionSettings {
                display_id: Some(r"\\.\DISPLAY2".into()),
                x: 1920,
                y: 120,
                width: 1280,
                height: 720,
            },
            ..AppSettings::default()
        };

        settings.save_to(&path).unwrap();
        let loaded = AppSettings::load_from(&path).unwrap();

        assert_eq!(loaded, settings);
    }

    #[test]
    fn validation_rejects_too_small_display_region() {
        let settings = AppSettings {
            capture_mode: CaptureMode::DisplayRegion,
            capture_region: CaptureRegionSettings {
                width: 1,
                height: 1080,
                ..CaptureRegionSettings::default()
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn service_options_include_estimated_buffer_bytes() {
        let settings = AppSettings::default();
        let opts = settings
            .to_service_options(Some("http://mock".into()))
            .unwrap();

        assert_eq!(opts.replay_window_s, 60.0);
        assert_eq!(opts.fps, 60);
        assert_eq!(opts.bitrate_bps, 12_000_000);
        assert_eq!(opts.video_encoder, VideoEncoder::Auto);
        assert_eq!(opts.output_resolution, OutputResolution::Source);
        assert_eq!(opts.disk_quota_bytes, Some(DEFAULT_DISK_QUOTA_BYTES));
        assert_eq!(opts.media_dir, PathBuf::from(default_media_dir()));
        assert_eq!(opts.lol_url.as_deref(), Some("http://mock"));
        assert_eq!(opts.audio, AudioOptions::default());
        assert_eq!(opts.replay_storage, ReplayStorageOptions::Memory);
        // Ring tracks the 60 s window + 15 s headroom with enough byte
        // slack for hardware encoders that overshoot their target bitrate.
        assert!(opts.buffer_bytes >= 200 * 1024 * 1024);
        assert!(opts.buffer_bytes < 240 * 1024 * 1024);
    }

    #[test]
    fn service_options_include_audio_settings() {
        let settings = AppSettings {
            audio: AudioSettings {
                output_enabled: true,
                output_device_id: Some("output-id".into()),
                output_volume: 0.75,
                split_output_by_process: false,
                mic_enabled: true,
                mic_device_id: Some("mic-id".into()),
                mic_volume: 1.5,
                mic_channels: AudioChannelMode::Stereo,
            },
            ..AppSettings::default()
        };

        let opts = settings.to_service_options(None).unwrap();

        assert!(opts.audio.output_enabled);
        assert_eq!(opts.audio.output_device_id.as_deref(), Some("output-id"));
        assert_eq!(opts.audio.output_volume, 0.75);
        assert!(!opts.audio.split_output_by_process);
        assert!(opts.audio.mic_enabled);
        assert_eq!(opts.audio.mic_device_id.as_deref(), Some("mic-id"));
        assert_eq!(opts.audio.mic_volume, 1.5);
        assert_eq!(opts.audio.mic_channels, AudioChannelMode::Stereo);
    }

    #[test]
    fn load_audio_split_toggle_from_json() {
        let json = r#"{
            "audio": {
                "split_output_by_process": false
            }
        }"#;
        let settings = AppSettings::load_from_object(
            serde_json::from_str::<Value>(json)
                .unwrap()
                .as_object()
                .unwrap(),
        );

        assert!(!settings.audio.split_output_by_process);
    }

    #[test]
    fn service_options_include_video_encoder_choice() {
        let settings = AppSettings {
            video_encoder: VideoEncoder::AmfH264,
            ..AppSettings::default()
        };

        let opts = settings.to_service_options(None).unwrap();

        assert_eq!(opts.video_encoder, VideoEncoder::AmfH264);
    }

    #[test]
    fn service_options_include_output_resolution_choice() {
        let settings = AppSettings {
            output_resolution: OutputResolution::P720,
            video_quality: VideoQuality::Sharp,
            ..AppSettings::default()
        };

        let opts = settings.to_service_options(None).unwrap();

        assert_eq!(opts.output_resolution, OutputResolution::P720);
        assert_eq!(opts.bitrate_bps, 8_000_000);
    }

    #[test]
    fn service_options_include_display_region_source() {
        let settings = AppSettings {
            capture_mode: CaptureMode::DisplayRegion,
            capture_region: CaptureRegionSettings {
                display_id: Some(r"\\.\DISPLAY1".into()),
                x: 100,
                y: 50,
                width: 800,
                height: 450,
            },
            ..AppSettings::default()
        };

        let opts = settings.to_service_options(None).unwrap();

        assert_eq!(
            opts.capture_source,
            crate::service::CaptureSource::DisplayRegion(crate::service::CaptureRegion {
                display_id: Some(r"\\.\DISPLAY1".into()),
                x: 100,
                y: 50,
                width: 800,
                height: 450,
            })
        );
    }

    #[test]
    fn settings_round_trip_json() {
        let dir = TestDir::new("round-trip");
        let path = dir.path().join("settings.json");
        let settings = AppSettings {
            video_quality: VideoQuality::Sharp,
            bitrate_mbps: 16.0,
            output_resolution: OutputResolution::P1080,
            hotkey: "Ctrl+Alt+F9".into(),
            close_to_tray: false,
            minimize_to_tray: true,
            capture_preview_enabled: false,
            update_channel: UpdateChannel::Nightly,
            games: GameSettings {
                auto_detect: true,
                plugins: BTreeMap::from([(
                    "league_of_legends".into(),
                    GamePluginSettings {
                        enabled: true,
                        recording_mode: GameRecordingMode::FullSession,
                    },
                )]),
                custom_games: vec![CustomGameSettings {
                    id: "custom-notepad".into(),
                    name: "Notepad".into(),
                    enabled: true,
                    exe_name: "notepad.exe".into(),
                    process_path: Some(r"C:\Windows\System32\notepad.exe".into()),
                    window_title: "Untitled - Notepad".into(),
                    recording_mode: GameRecordingMode::FullSession,
                    icon: None,
                }],
            },
            ..AppSettings::default()
        };

        settings.save_to(&path).unwrap();
        let loaded = AppSettings::load_from(&path).unwrap();

        assert_eq!(loaded, settings);
    }

    #[test]
    fn validation_rejects_custom_game_without_match_identity() {
        let settings = AppSettings {
            games: GameSettings {
                auto_detect: true,
                plugins: BTreeMap::new(),
                custom_games: vec![CustomGameSettings {
                    id: "custom-empty".into(),
                    name: "Mystery".into(),
                    enabled: true,
                    exe_name: " ".into(),
                    process_path: None,
                    window_title: " ".into(),
                    recording_mode: GameRecordingMode::ReplaysOnly,
                    icon: None,
                }],
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }

    #[test]
    fn parses_alt_f10_hotkey() {
        assert_eq!(normalize_hotkey("alt+f10").unwrap(), "Alt+F10");
        assert!(parse_hotkey("Alt+F10").is_ok());
    }

    #[test]
    fn parses_multi_modifier_hotkey() {
        assert_eq!(normalize_hotkey("ctrl+shift+f9").unwrap(), "Ctrl+Shift+F9");
        assert!(parse_hotkey("Ctrl+Shift+F9").is_ok());
    }

    #[test]
    fn rejects_non_function_key_hotkeys() {
        assert!(parse_hotkey("Alt+S").is_err());
        assert!(parse_hotkey("F12").is_err());
    }

    #[test]
    fn parses_plain_function_key_hotkey() {
        assert_eq!(normalize_hotkey("f10").unwrap(), "F10");
        assert!(parse_hotkey("F10").is_ok());
    }

    #[test]
    fn quota_zero_disables_gc() {
        assert_eq!(quota_bytes_from_gb(0.0).unwrap(), None);
        assert_eq!(quota_bytes_from_gb(0.5).unwrap(), Some(512 * 1024 * 1024));
    }

    #[test]
    fn buffer_estimate_scales_with_duration_and_bitrate() {
        let small = estimated_buffer_bytes(60.0, 8.0);
        let large = estimated_buffer_bytes(120.0, 16.0);

        assert!(small >= 64 * 1024 * 1024);
        assert!(large > small * 3);
    }

    #[test]
    fn thirty_second_replay_has_buffer_slack_for_encoder_overshoot() {
        let settings = AppSettings {
            replay_window_s: 30.0,
            buffer_seconds: 45.0,
            bitrate_mbps: 5.0,
            ..AppSettings::default()
        };
        let opts = settings.to_service_options(None).unwrap();

        assert!(opts.buffer_bytes >= 64 * 1024 * 1024);
    }

    #[test]
    fn load_normalizes_buffer_seconds_to_replay_plus_headroom() {
        let dir = TestDir::new("buffer-headroom");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "capture_mode": "primary_monitor",
                "window_title": "",
                "buffer_seconds": 120.0,
                "replay_window_s": 30.0,
                "bitrate_mbps": 12.0,
                "fps": 60,
                "video_encoder": "auto",
                "disk_quota_gb": 10.0,
                "media_dir": default_media_dir(),
                "hotkey": "Alt+F10"
            })
            .to_string(),
        )
        .unwrap();

        let settings = AppSettings::load_from(&path).unwrap();

        assert_eq!(settings.buffer_seconds, 45.0);
        assert_eq!(settings.replay_storage, ReplayStorageSettings::default());
    }

    #[test]
    fn disk_replay_requires_acknowledgement_and_folder() {
        let mut settings = AppSettings {
            replay_storage: ReplayStorageSettings {
                mode: ReplayStorageMode::Disk,
                disk_dir: String::new(),
                disk_quota_gb: 2.0,
                disk_acknowledged: false,
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
        settings.replay_storage.disk_acknowledged = true;
        assert!(settings.validate().is_err());
        settings.replay_storage.disk_dir = std::env::temp_dir()
            .join("clipline-cache")
            .display()
            .to_string();
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn disk_replay_rejects_media_folder_overlap() {
        let media = std::env::temp_dir().join("clipline-media");
        let settings = AppSettings {
            media_dir: media.display().to_string(),
            replay_storage: ReplayStorageSettings {
                mode: ReplayStorageMode::Disk,
                disk_dir: media.join("cache").display().to_string(),
                disk_quota_gb: 2.0,
                disk_acknowledged: true,
            },
            ..AppSettings::default()
        };

        assert!(settings.validate().is_err());
    }
}
