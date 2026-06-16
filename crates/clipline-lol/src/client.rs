use std::time::Duration;

use serde::Deserialize;

use clipline_events::PlayerSummary;

use crate::normalize::player_name_key;
use crate::raw::{EventData, PlayerListEntry};

/// Riot's local Live Client Data endpoint (ddoc §5a).
pub const DEFAULT_BASE: &str = "https://127.0.0.1:2999";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("live client request failed: {0}")]
    Http(#[from] reqwest::Error),
}

/// Client for the League Live Client Data API. The real endpoint serves a
/// self-signed Riot cert over loopback, so certificate validation is
/// disabled — the client is only ever pointed at 127.0.0.1 (or a test mock).
pub struct LiveClient {
    base: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct GameStats {
    #[serde(rename = "gameTime")]
    game_time: f64,
}

impl LiveClient {
    pub fn new(base: impl Into<String>) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()?;
        Ok(Self {
            base: base.into(),
            http,
        })
    }

    /// Client against the real local game endpoint.
    pub fn default_local() -> Result<Self, Error> {
        Self::new(DEFAULT_BASE)
    }

    pub async fn event_data(&self) -> Result<EventData, Error> {
        Ok(self.get_json("/liveclientdata/eventdata").await?)
    }

    pub async fn player_list(&self) -> Result<Vec<PlayerListEntry>, Error> {
        Ok(self.get_json("/liveclientdata/playerlist").await?)
    }

    /// Riot returns the active player's name as a bare JSON string.
    pub async fn active_player_name(&self) -> Result<String, Error> {
        Ok(self.get_json("/liveclientdata/activeplayername").await?)
    }

    pub async fn player_summary(&self, local_player: &str) -> Result<Option<PlayerSummary>, Error> {
        let players = self.player_list().await?;
        Ok(player_summary_from_list(&players, local_player))
    }

    /// Current game clock in seconds, from `gamestats.gameTime` (ddoc §5).
    pub async fn game_time_s(&self) -> Result<f64, Error> {
        let stats: GameStats = self.get_json("/liveclientdata/gamestats").await?;
        Ok(stats.game_time)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, reqwest::Error> {
        self.http
            .get(format!("{}{}", self.base, path))
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
    }
}

pub fn player_summary_from_list(
    players: &[PlayerListEntry],
    local_player: &str,
) -> Option<PlayerSummary> {
    let local_key = player_name_key(local_player);
    if local_key.is_empty() {
        return None;
    }
    let player = players.iter().find(|player| {
        player_name_key(&player.summoner_name) == local_key
            || player
                .riot_id
                .as_deref()
                .is_some_and(|riot_id| player_name_key(riot_id) == local_key)
    })?;
    let champion_name = player.champion_name.trim();
    if champion_name.is_empty() {
        return None;
    }
    Some(PlayerSummary {
        champion_name: champion_name.to_string(),
        kills: player.scores.kills,
        deaths: player.scores.deaths,
        assists: player.scores.assists,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{PlayerListEntry, PlayerScores};

    fn player(
        summoner_name: &str,
        riot_id: Option<&str>,
        champion_name: &str,
        kills: u32,
        deaths: u32,
        assists: u32,
    ) -> PlayerListEntry {
        PlayerListEntry {
            summoner_name: summoner_name.into(),
            riot_id: riot_id.map(str::to_string),
            champion_name: champion_name.into(),
            scores: PlayerScores {
                kills,
                deaths,
                assists,
            },
        }
    }

    #[test]
    fn player_summary_matches_summoner_name_or_riot_id() {
        let players = [
            player("Someone", Some("Someone#NA1"), "Ahri", 1, 2, 3),
            player("dain", Some("Dain#NA1"), "Nautilus", 3, 4, 23),
        ];

        let by_riot_id = player_summary_from_list(&players, "dain#NA1").unwrap();
        assert_eq!(by_riot_id.champion_name, "Nautilus");
        assert_eq!(
            (by_riot_id.kills, by_riot_id.deaths, by_riot_id.assists),
            (3, 4, 23)
        );

        let by_summoner = player_summary_from_list(&players, " DAIN ").unwrap();
        assert_eq!(by_summoner.champion_name, "Nautilus");
    }

    #[test]
    fn player_summary_ignores_missing_local_player_or_champion() {
        let players = [player("dain", Some("Dain#NA1"), "", 3, 4, 23)];

        assert!(player_summary_from_list(&players, "other").is_none());
        assert!(player_summary_from_list(&players, "dain").is_none());
    }
}
