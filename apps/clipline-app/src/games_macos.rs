use crate::game_plugins::GamePluginInfo;
use crate::platform;
use crate::settings::{GameRecordingMode, GameSettings};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GameWindowInfo {
    pub title: String,
    pub process_id: u32,
    pub exe_name: String,
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetectedGame {
    pub id: String,
    pub name: String,
    pub hwnd: isize,
    pub window_title: String,
    pub process_id: u32,
    pub exe_name: String,
    pub recording_mode: GameRecordingMode,
}

pub fn game_plugin_catalog() -> Vec<GamePluginInfo> {
    Vec::new()
}

pub fn list_game_windows() -> Vec<GameWindowInfo> {
    platform::enumerate_capturable_windows()
        .into_iter()
        .map(|window| GameWindowInfo {
            title: window.title,
            process_id: window.process_id,
            exe_name: window.exe_name,
            exe_path: window.exe_path,
        })
        .collect()
}

pub fn detect_active_game(_settings: &GameSettings) -> Option<DetectedGame> {
    None
}

pub fn built_in_game_still_configured(_settings: &GameSettings, _id: &str) -> bool {
    false
}
