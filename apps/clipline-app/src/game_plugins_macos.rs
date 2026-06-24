use crate::settings::GameRecordingMode;

pub const LEAGUE_OF_LEGENDS_ID: &str = "league_of_legends";

pub struct GamePlugin {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GamePluginInfo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub event_markers: bool,
    pub icon: Option<String>,
}

pub fn all() -> &'static [GamePlugin] {
    &[]
}

pub fn contains(_id: &str) -> bool {
    false
}

#[allow(dead_code)]
pub fn ensure_plugin_icon_cached(_plugin_id: &str, _exe_path: &str) {}
