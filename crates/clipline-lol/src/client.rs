use std::time::Duration;

use serde::Deserialize;

use crate::raw::EventData;

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

    /// Riot returns the active player's name as a bare JSON string.
    pub async fn active_player_name(&self) -> Result<String, Error> {
        Ok(self.get_json("/liveclientdata/activeplayername").await?)
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
