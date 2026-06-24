use super::*;
use crate::service::{
    AudioChannelMode, AudioOptions, CaptureRegion, CaptureSource, ReplayStorageOptions,
    VideoEncoder, DEFAULT_DISK_QUOTA_BYTES,
};
use crate::settings::persistence::sibling_tmp_path;
use crate::settings::types::ReplayStorageMode;
use std::collections::BTreeMap;
use std::path::PathBuf;

use clipline_test_utils::TestDir;
use serde_json::Value;

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
    assert_eq!(settings.update_channel, UpdateChannel::Nightly);
    assert!(settings.validate().is_ok());
}

#[test]
fn load_repairs_disabled_stable_update_channel() {
    let dir = TestDir::new("clipline-settings", "stable-update-channel");
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
    let dir = TestDir::new("clipline-settings", "heal-media-folder");
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
    let dir = TestDir::new("clipline-settings", "legacy-quality-resolution");
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
    let dir = TestDir::new("clipline-settings", "clamp-replay-window");
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
    let dir = TestDir::new("clipline-settings", "migrate-hotkey");
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
    let dir = TestDir::new("clipline-settings", "unknown-encoder");
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
    let dir = TestDir::new("clipline-settings", "repair-invalid-fields");
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{
                "capture_mode": "future_capture",
                "capture_backend": "smart_future",
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
    assert_eq!(settings.capture_backend, CaptureBackend::Auto);
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
    let dir = TestDir::new("clipline-settings", "region-round-trip");
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
fn service_options_include_capture_backend_choice() {
    let settings = AppSettings {
        capture_backend: CaptureBackend::DesktopDuplication,
        ..AppSettings::default()
    };

    let opts = settings.to_service_options(None).unwrap();

    assert_eq!(opts.capture_backend, CaptureBackend::DesktopDuplication);
}

#[test]
fn capture_backend_defaults_to_auto() {
    assert_eq!(AppSettings::default().capture_backend, CaptureBackend::Auto);
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
        CaptureSource::DisplayRegion(CaptureRegion {
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
    let dir = TestDir::new("clipline-settings", "round-trip");
    let path = dir.path().join("settings.json");
    let settings = AppSettings {
        video_quality: VideoQuality::Sharp,
        bitrate_mbps: 16.0,
        output_resolution: OutputResolution::P1080,
        hotkey: "Ctrl+Alt+F9".into(),
        close_to_tray: false,
        minimize_to_tray: true,
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
fn parses_mouse_button_hotkeys() {
    assert_eq!(normalize_hotkey("mouse5").unwrap(), "Mouse5");
    assert_eq!(normalize_hotkey("ctrl+mouse4").unwrap(), "Ctrl+Mouse4");
    assert_eq!(normalize_hotkey("alt+forward").unwrap(), "Alt+Mouse5");
    assert_eq!(normalize_hotkey("shift+back").unwrap(), "Shift+Mouse4");
    assert_eq!(normalize_hotkey("mbutton").unwrap(), "Middle");
}

#[test]
fn rejects_non_function_key_hotkeys() {
    assert!(parse_hotkey("Alt+S").is_err());
    assert!(parse_hotkey("F12").is_err());
}

#[test]
fn rejects_unsafe_mouse_hotkeys() {
    assert!(normalize_hotkey("Mouse1").is_err());
    assert!(normalize_hotkey("RightMouse").is_err());
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
    let dir = TestDir::new("clipline-settings", "buffer-headroom");
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
fn save_to_replaces_settings_via_temp_file() {
    let dir = TestDir::new("clipline-settings", "atomic-save");
    let path = dir.path().join("settings.json");
    let tmp = dir.path().join("settings.json.tmp");
    std::fs::write(&path, "{}").unwrap();
    std::fs::write(&tmp, "stale").unwrap();

    AppSettings::default().save_to(&path).unwrap();

    assert!(!tmp.exists(), "save_to must not leave stale temp files");
    assert_eq!(
        AppSettings::load_from(&path).unwrap(),
        AppSettings::default()
    );
}

#[test]
fn temporary_settings_paths_are_unique_per_save_attempt() {
    let dir = TestDir::new("clipline-settings", "atomic-save-unique-temp");
    let path = dir.path().join("settings.json");

    let first = sibling_tmp_path(&path).unwrap();
    let second = sibling_tmp_path(&path).unwrap();

    assert_ne!(first, second);
    for tmp in [first, second] {
        let name = tmp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("settings.json."));
        assert!(name.ends_with(".tmp"));
        assert_eq!(tmp.parent(), path.parent());
    }
}

#[test]
fn uploaded_processing_status_survives_cloud_settings_normalization() {
    let mut cloud = CloudSettings::default();
    cloud.uploads.insert(
        "clip".into(),
        CloudUploadRecord {
            local_clip_id: "clip".into(),
            path: "D:\\Videos\\clip.mp4".into(),
            remote_clip_id: Some("remote".into()),
            remote_url: Some("https://clips.example.com/clip/remote".into()),
            visibility: "unlisted".into(),
            upload_status: "uploaded_processing".into(),
            error: Some("processing is still pending".into()),
            updated_at_unix: 2,
        },
    );

    cloud.normalize();

    assert_eq!(
        cloud.uploads.get("clip").unwrap().upload_status,
        "uploaded_processing"
    );
    assert!(cloud.validate().is_ok());
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

#[test]
fn disk_replay_rejects_case_variant_media_folder_overlap() {
    let media = std::env::temp_dir().join("clipline-media-case");
    let media_dir = media.display().to_string().to_ascii_uppercase();
    let cache_dir = media
        .join("cache")
        .display()
        .to_string()
        .to_ascii_lowercase();
    let settings = AppSettings {
        media_dir,
        replay_storage: ReplayStorageSettings {
            mode: ReplayStorageMode::Disk,
            disk_dir: cache_dir,
            disk_quota_gb: 2.0,
            disk_acknowledged: true,
        },
        ..AppSettings::default()
    };

    assert!(settings.validate().is_err());
}

#[test]
fn disk_replay_rejects_verbatim_media_folder_overlap() {
    let media = std::env::temp_dir().join("clipline-media-verbatim");
    let media_dir = media.display().to_string();
    let cache_dir = format!(r"\\?\{}", media.join("cache").display());
    let settings = AppSettings {
        media_dir,
        replay_storage: ReplayStorageSettings {
            mode: ReplayStorageMode::Disk,
            disk_dir: cache_dir,
            disk_quota_gb: 2.0,
            disk_acknowledged: true,
        },
        ..AppSettings::default()
    };

    assert!(settings.validate().is_err());
}
