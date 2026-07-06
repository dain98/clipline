//! Serde types for CS2 Game State Integration POST bodies.
//!
//! Everything is optional: GSI omits whole sections depending on state (no
//! `map`/`round` in menus, no `player` before the first spawn), and own-play
//! payloads never include the spectator-only sections (`allplayers_*`,
//! top-level `bomb`, `phase_countdowns`) — verified against a live capture,
//! not just the docs. Unknown fields are ignored so Valve can add data
//! without breaking us.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiPayload {
    #[serde(default)]
    pub auth: Option<GsiAuth>,
    #[serde(default)]
    pub provider: Option<GsiProvider>,
    #[serde(default)]
    pub map: Option<GsiMap>,
    #[serde(default)]
    pub round: Option<GsiRound>,
    #[serde(default)]
    pub player: Option<GsiPlayer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiAuth {
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiProvider {
    /// SteamID64 of the account running the game — the identity anchor.
    #[serde(default)]
    pub steamid: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiMap {
    #[serde(default)]
    pub name: String,
    /// "warmup" | "live" | "intermission" | "gameover"
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub round: Option<u32>,
    #[serde(default)]
    pub team_ct: Option<GsiTeamState>,
    #[serde(default)]
    pub team_t: Option<GsiTeamState>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiTeamState {
    #[serde(default)]
    pub score: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiRound {
    /// "freezetime" | "live" | "over"
    #[serde(default)]
    pub phase: String,
    /// "planted" | "defused" | "exploded" — absent when no bomb is down.
    #[serde(default)]
    pub bomb: Option<String>,
    /// "CT" | "T" — present only once the round is decided.
    #[serde(default)]
    pub win_team: Option<String>,
}

/// The player *on screen*: the local player while alive, the spectated
/// teammate while dead or joining mid-round. Never trust its stats without
/// checking `steamid` against [`GsiProvider::steamid`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiPlayer {
    #[serde(default)]
    pub steamid: String,
    #[serde(default)]
    pub name: String,
    /// "CT" | "T"
    #[serde(default)]
    pub team: Option<String>,
    /// "playing" | "menu" | "textinput"
    #[serde(default)]
    pub activity: Option<String>,
    #[serde(default)]
    pub state: Option<GsiPlayerState>,
    #[serde(default)]
    pub match_stats: Option<GsiMatchStats>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiPlayerState {
    #[serde(default)]
    pub health: Option<i32>,
    #[serde(default)]
    pub round_kills: Option<u32>,
    #[serde(default, rename = "round_killhs")]
    pub round_kill_headshots: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GsiMatchStats {
    #[serde(default)]
    pub kills: u32,
    #[serde(default)]
    pub assists: u32,
    #[serde(default)]
    pub deaths: u32,
    #[serde(default)]
    pub mvps: u32,
    #[serde(default)]
    pub score: u32,
}

impl GsiPayload {
    pub fn from_json(json: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(json)
    }

    /// The `player` node, but only when it describes the local account.
    /// Guards against the spectated-player trap: while dead or joining
    /// mid-round, `player` is whoever the camera follows.
    pub fn local_player(&self) -> Option<&GsiPlayer> {
        let local = self.provider.as_ref()?.steamid.trim();
        let player = self.player.as_ref()?;
        (!local.is_empty() && player.steamid.trim() == local).then_some(player)
    }

    pub fn auth_token(&self) -> &str {
        self.auth.as_ref().map(|a| a.token.as_str()).unwrap_or("")
    }
}
