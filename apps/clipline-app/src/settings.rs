//! Persisted application settings and mapping to recorder service options.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

use crate::service::ServiceOptions;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    PrimaryMonitor,
    WindowTitle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub capture_mode: CaptureMode,
    pub window_title: String,
    pub buffer_seconds: f64,
    pub replay_window_s: f64,
    pub bitrate_mbps: f64,
    pub fps: u32,
    pub disk_quota_gb: f64,
    pub hotkey: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            capture_mode: CaptureMode::PrimaryMonitor,
            window_title: String::new(),
            buffer_seconds: 120.0,
            replay_window_s: 60.0,
            bitrate_mbps: 12.0,
            fps: 60,
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
        validate_range("buffer seconds", self.buffer_seconds, 10.0, 20.0 * 60.0)?;
        validate_range(
            "replay seconds",
            self.replay_window_s,
            5.0,
            self.buffer_seconds,
        )?;
        validate_range("bitrate Mbps", self.bitrate_mbps, 1.0, 100.0)?;
        if !matches!(self.fps, 30 | 60 | 120) {
            return Err("fps must be 30, 60, or 120".into());
        }
        quota_bytes_from_gb(self.disk_quota_gb)?;
        normalize_hotkey(&self.hotkey)?;
        Ok(())
    }

    pub fn to_service_options(&self, lol_url: Option<String>) -> Result<ServiceOptions, String> {
        self.validate()?;
        Ok(ServiceOptions {
            window_title: match self.capture_mode {
                CaptureMode::PrimaryMonitor => None,
                CaptureMode::WindowTitle => Some(self.window_title.trim().to_string()),
            },
            lol_url,
            replay_window_s: self.replay_window_s,
            buffer_bytes: estimated_buffer_bytes(self.buffer_seconds, self.bitrate_mbps),
            disk_quota_bytes: quota_bytes_from_gb(self.disk_quota_gb)?,
            fps: self.fps,
            bitrate_bps: (self.bitrate_mbps * 1_000_000.0).round() as u32,
        })
    }

    pub fn load_from(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut settings: Self = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        settings.hotkey = normalize_hotkey(&settings.hotkey)?;
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
                    .map_err(|_| "hotkey key must be F1 through F24")?;
                if !(1..=24).contains(&n) {
                    return Err("hotkey key must be F1 through F24".into());
                }
                key = Some(n);
            }
            _ => return Err("hotkey must use Ctrl, Alt, Shift, and F1 through F24".into()),
        }
    }

    let key = key.ok_or("hotkey needs an F-key")?;
    if !ctrl && !alt && !shift {
        return Err("hotkey needs at least one modifier".into());
    }

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
        assert_eq!(settings.buffer_seconds, 120.0);
        assert_eq!(settings.replay_window_s, 60.0);
        assert_eq!(settings.bitrate_mbps, 12.0);
        assert_eq!(settings.fps, 60);
        assert_eq!(settings.disk_quota_gb, 10.0);
        assert_eq!(settings.hotkey, "Alt+F10");
    }

    #[test]
    fn validation_rejects_replay_longer_than_buffer() {
        let settings = AppSettings {
            replay_window_s: 121.0,
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
    fn service_options_include_estimated_buffer_bytes() {
        let settings = AppSettings::default();
        let opts = settings
            .to_service_options(Some("http://mock".into()))
            .unwrap();

        assert_eq!(opts.replay_window_s, 60.0);
        assert_eq!(opts.fps, 60);
        assert_eq!(opts.bitrate_bps, 12_000_000);
        assert_eq!(opts.disk_quota_bytes, Some(DEFAULT_DISK_QUOTA_BYTES));
        assert_eq!(opts.lol_url.as_deref(), Some("http://mock"));
        assert!(opts.buffer_bytes >= 220 * 1024 * 1024);
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
        assert!(parse_hotkey("F10").is_err());
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
