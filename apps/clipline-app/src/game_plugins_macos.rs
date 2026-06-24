use crate::settings::{GamePluginSettings, GameRecordingMode, GameSettings};

pub const LEAGUE_OF_LEGENDS_ID: &str = "league_of_legends";

pub struct GamePlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub icon: Option<&'static str>,
    pub event_markers: bool,
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

impl GamePlugin {
    pub fn default_settings(&self) -> GamePluginSettings {
        GamePluginSettings {
            enabled: self.default_enabled,
            recording_mode: self.default_recording_mode,
        }
    }

    pub fn settings(&self, settings: &GameSettings) -> GamePluginSettings {
        settings
            .plugins
            .get(self.id)
            .cloned()
            .unwrap_or_else(|| self.default_settings())
    }

    pub fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: self.id.into(),
            name: self.name.into(),
            summary: self.summary.into(),
            default_enabled: self.default_enabled,
            default_recording_mode: self.default_recording_mode,
            event_markers: self.event_markers,
            icon: self.icon.map(str::to_string),
        }
    }
}

pub fn all() -> &'static [GamePlugin] {
    &GAME_PLUGINS
}

pub fn contains(id: &str) -> bool {
    all().iter().any(|plugin| plugin.id == id)
}

#[allow(dead_code)]
pub fn ensure_plugin_icon_cached(_plugin_id: &str, _exe_path: &str) {}

static GAME_PLUGINS: [GamePlugin; 1] = [GamePlugin {
    id: LEAGUE_OF_LEGENDS_ID,
    name: "League of Legends",
    summary: "Auto-records full matches when the in-game window is active.",
    default_enabled: true,
    default_recording_mode: GameRecordingMode::FullSession,
    icon: Some("assets/games/league-of-legends.png"),
    event_markers: true,
}];
