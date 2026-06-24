use crate::settings::GameRecordingMode;

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

pub fn contains(_id: &str) -> bool {
    false
}
