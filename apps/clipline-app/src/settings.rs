//! Persisted application settings and mapping to recorder service options.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

use crate::service::{
    AudioChannelMode, AudioOptions, CaptureRegion, CaptureSource, ServiceOptions, VideoEncoder,
};

const MAX_REPLAY_WINDOW_S: f64 = 120.0;

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
            mic_enabled: false,
            mic_device_id: None,
            mic_volume: 1.0,
            mic_channels: AudioChannelMode::Mono,
        }
    }
}

impl AudioSettings {
    fn to_service_options(&self) -> AudioOptions {
        AudioOptions {
            output_enabled: self.output_enabled,
            output_device_id: self
                .output_device_id
                .clone()
                .filter(|id| !id.trim().is_empty()),
            output_volume: self.output_volume,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub capture_mode: CaptureMode,
    pub window_title: String,
    #[serde(default)]
    pub capture_region: CaptureRegionSettings,
    #[serde(default)]
    pub audio: AudioSettings,
    pub buffer_seconds: f64,
    pub replay_window_s: f64,
    pub bitrate_mbps: f64,
    pub fps: u32,
    #[serde(default, deserialize_with = "deserialize_video_encoder")]
    pub video_encoder: VideoEncoder,
    pub disk_quota_gb: f64,
    pub hotkey: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            capture_mode: CaptureMode::PrimaryMonitor,
            window_title: String::new(),
            capture_region: CaptureRegionSettings::default(),
            audio: AudioSettings::default(),
            buffer_seconds: 120.0,
            replay_window_s: 60.0,
            bitrate_mbps: 12.0,
            fps: 60,
            video_encoder: VideoEncoder::Auto,
            disk_quota_gb: 10.0,
            hotkey: "Alt+F10".into(),
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
            if self.capture_region.width < 2 || self.capture_region.height < 2 {
                return Err("capture region must be at least 2x2 pixels".into());
            }
            if self.capture_region.width > 16_384 || self.capture_region.height > 16_384 {
                return Err("capture region is too large".into());
            }
        }
        validate_range("output volume", self.audio.output_volume, 0.0, 2.0)?;
        validate_range("microphone volume", self.audio.mic_volume, 0.0, 2.0)?;
        validate_range("buffer seconds", self.buffer_seconds, 10.0, 20.0 * 60.0)?;
        validate_range(
            "replay seconds",
            self.replay_window_s,
            5.0,
            MAX_REPLAY_WINDOW_S,
        )?;
        if self.replay_window_s > self.buffer_seconds {
            return Err("replay seconds cannot be longer than buffer seconds".into());
        }
        validate_range("bitrate Mbps", self.bitrate_mbps, 1.0, 100.0)?;
        if !matches!(self.fps, 30 | 60 | 90 | 120) {
            return Err("fps must be 30, 60, 90, or 120".into());
        }
        quota_bytes_from_gb(self.disk_quota_gb)?;
        normalize_hotkey(&self.hotkey)?;
        Ok(())
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
            lol_url,
            replay_window_s: self.replay_window_s,
            buffer_bytes: estimated_buffer_bytes(self.buffer_seconds, self.bitrate_mbps),
            disk_quota_bytes: quota_bytes_from_gb(self.disk_quota_gb)?,
            fps: self.fps,
            bitrate_bps: (self.bitrate_mbps * 1_000_000.0).round() as u32,
            video_encoder: self.video_encoder,
            audio: self.audio.to_service_options(),
        })
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut settings: Self = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        settings.hotkey =
            normalize_hotkey(&settings.hotkey).unwrap_or_else(|_| AppSettings::default().hotkey);
        settings.audio.output_device_id = settings
            .audio
            .output_device_id
            .filter(|id| !id.trim().is_empty());
        settings.audio.mic_device_id = settings
            .audio
            .mic_device_id
            .filter(|id| !id.trim().is_empty());
        settings.replay_window_s = settings.replay_window_s.min(MAX_REPLAY_WINDOW_S);
        settings.buffer_seconds = settings.buffer_seconds.max(settings.replay_window_s);
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut settings = self.clone();
        settings.hotkey = normalize_hotkey(&settings.hotkey)?;
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

pub fn settings_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join("AppData").join("Roaming"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("Clipline").join("settings.json")
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

fn estimated_buffer_bytes(buffer_seconds: f64, bitrate_mbps: f64) -> usize {
    const MIN_BUFFER_BYTES: f64 = 64.0 * 1024.0 * 1024.0;
    const AUDIO_AND_MOTION_HEADROOM: f64 = 1.30;

    let video_bytes = bitrate_mbps * 1_000_000.0 / 8.0 * buffer_seconds;
    (video_bytes * AUDIO_AND_MOTION_HEADROOM).max(MIN_BUFFER_BYTES) as usize
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
        assert!(settings.audio.output_enabled);
        assert_eq!(settings.audio.output_device_id, None);
        assert_eq!(settings.audio.output_volume, 1.0);
        assert!(!settings.audio.mic_enabled);
        assert_eq!(settings.audio.mic_device_id, None);
        assert_eq!(settings.audio.mic_volume, 1.0);
        assert_eq!(settings.audio.mic_channels, AudioChannelMode::Mono);
        assert_eq!(settings.buffer_seconds, 120.0);
        assert_eq!(settings.replay_window_s, 60.0);
        assert_eq!(settings.bitrate_mbps, 12.0);
        assert_eq!(settings.fps, 60);
        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert_eq!(settings.disk_quota_gb, 10.0);
        assert_eq!(settings.hotkey, "Alt+F10");
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
        assert_eq!(settings.video_encoder, VideoEncoder::Auto);
        assert!(settings.validate().is_ok());
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
        assert_eq!(settings.buffer_seconds, 300.0);
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
        assert_eq!(opts.disk_quota_bytes, Some(DEFAULT_DISK_QUOTA_BYTES));
        assert_eq!(opts.lol_url.as_deref(), Some("http://mock"));
        assert_eq!(opts.audio, AudioOptions::default());
        assert!(opts.buffer_bytes >= 220 * 1024 * 1024);
    }

    #[test]
    fn service_options_include_audio_settings() {
        let settings = AppSettings {
            audio: AudioSettings {
                output_enabled: true,
                output_device_id: Some("output-id".into()),
                output_volume: 0.75,
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
        assert!(opts.audio.mic_enabled);
        assert_eq!(opts.audio.mic_device_id.as_deref(), Some("mic-id"));
        assert_eq!(opts.audio.mic_volume, 1.5);
        assert_eq!(opts.audio.mic_channels, AudioChannelMode::Stereo);
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
            bitrate_mbps: 18.0,
            hotkey: "Ctrl+Alt+F9".into(),
            ..AppSettings::default()
        };

        settings.save_to(&path).unwrap();
        let loaded = AppSettings::load_from(&path).unwrap();

        assert_eq!(loaded, settings);
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
}
