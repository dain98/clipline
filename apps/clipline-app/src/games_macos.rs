use crate::game_plugins::GamePluginInfo;
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
    Vec::new()
}

pub fn detect_active_game(_settings: &GameSettings) -> Option<DetectedGame> {
    None
}

pub fn built_in_game_still_configured(_settings: &GameSettings, _id: &str) -> bool {
    false
}
