//! Built-in game plugin registry.
//!
//! A game plugin describes how Clipline should recognize a game's real
//! in-game window from normal Win32 window/process metadata. Plugins must stay
//! anti-cheat-safe: no injection, no memory reads, no game-process hooks.

use std::sync::mpsc::Receiver;
use std::time::Instant;

use clipline_capture::windows::CapturableWindow;

use crate::markers::PollerMsg;
use crate::settings::{GamePluginSettings, GameRecordingMode, GameSettings};

pub const LEAGUE_OF_LEGENDS_ID: &str = "league_of_legends";

pub type WindowMatcher = for<'a> fn(&'a [CapturableWindow]) -> Option<&'a CapturableWindow>;
pub type EventSourceSpawner = fn(GameEventSourceContext) -> Receiver<PollerMsg>;

#[derive(Clone, Debug)]
pub struct GameEventSourceContext {
    pub lol_url: Option<String>,
    pub recording_t0: Instant,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GamePluginInfo {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub event_markers: bool,
}

pub struct GamePlugin {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub default_enabled: bool,
    pub default_recording_mode: GameRecordingMode,
    pub match_window: WindowMatcher,
    pub event_source: Option<EventSourceSpawner>,
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
            event_markers: self.event_source.is_some(),
        }
    }
}

pub fn all() -> &'static [GamePlugin] {
    &GAME_PLUGINS
}

pub fn contains(id: &str) -> bool {
    all().iter().any(|plugin| plugin.id == id)
}

pub fn has_event_source(plugin_id: Option<&str>) -> bool {
    let Some(id) = plugin_id else {
        return false;
    };
    all()
        .iter()
        .any(|plugin| plugin.id == id && plugin.event_source.is_some())
}

pub fn spawn_event_source(
    plugin_id: Option<&str>,
    context: GameEventSourceContext,
) -> Option<Receiver<PollerMsg>> {
    let id = plugin_id?;
    all()
        .iter()
        .find(|plugin| plugin.id == id)
        .and_then(|plugin| plugin.event_source.map(|spawn| spawn(context)))
}

static GAME_PLUGINS: [GamePlugin; 1] = [GamePlugin {
    id: LEAGUE_OF_LEGENDS_ID,
    name: "League of Legends",
    summary: "Auto-records full matches when the in-game window is active.",
    default_enabled: true,
    default_recording_mode: GameRecordingMode::FullSession,
    match_window: league_of_legends::match_window,
    event_source: Some(league_of_legends::spawn_event_source),
}];

mod league_of_legends {
    use std::sync::mpsc::Receiver;

    use clipline_capture::windows::CapturableWindow;

    use super::GameEventSourceContext;
    use crate::markers::PollerMsg;

    const IN_GAME_EXE: &str = "League of Legends.exe";

    pub fn match_window(windows: &[CapturableWindow]) -> Option<&CapturableWindow> {
        windows
            .iter()
            .filter(|window| window.exe_name.eq_ignore_ascii_case(IN_GAME_EXE))
            .max_by_key(|window| window.title.len())
    }

    pub fn spawn_event_source(context: GameEventSourceContext) -> Receiver<PollerMsg> {
        crate::markers::spawn(context.lol_url, context.recording_t0)
    }
}
