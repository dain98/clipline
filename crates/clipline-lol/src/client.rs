use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;

use clipline_events::{PlayerItem, PlayerParticipant, PlayerSummary, PlayerSummonerSpell};

use crate::normalize::player_name_key;
use crate::raw::{EventData, PlayerItemEntry, PlayerListEntry, PlayerSummonerSpellEntry};

/// Riot's local Live Client Data endpoint (ddoc §5a).
const DEFAULT_BASE: &str = "https://127.0.0.1:2999";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("live client request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("refusing to disable certificate validation for non-loopback URL: {0}")]
    NotLoopback(String),
}

/// Client for the League Live Client Data API. The real endpoint serves a
/// self-signed Riot cert over loopback, so certificate validation is
/// disabled — the client is only ever pointed at 127.0.0.1 (or a test mock).
pub struct LiveClient {
    base: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for LiveClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct GameStats {
    #[serde(rename = "gameTime")]
    game_time: f64,
}

impl LiveClient {
    pub fn new(base: impl Into<String>) -> Result<Self, Error> {
        let base = base.into();
        if !is_loopback_url(&base).unwrap_or(false) {
            return Err(Error::NotLoopback(base));
        }
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()?;
        Ok(Self { base, http })
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
        let game_time_s = self.game_time_s().await.ok();
        Ok(player_summary_from_list_with_game_time(
            &players,
            local_player,
            game_time_s,
        ))
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

/// Returns `Some(true)` for loopback URLs (cert validation can safely be
/// skipped for Riot's self-signed local cert), `Some(false)` for other
/// well-formed URLs, and `None` for unparseable inputs.
fn is_loopback_url(base: &str) -> Option<bool> {
    let url = reqwest::Url::parse(base).ok()?;
    let host = url.host_str()?;
    if host == "localhost" {
        return Some(true);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip.is_loopback());
    }
    // Strip brackets from IPv6 literals like `[::1]`.
    let trimmed = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));
    if let Some(inner) = trimmed {
        if let Ok(ip) = inner.parse::<IpAddr>() {
            return Some(ip.is_loopback());
        }
    }
    Some(false)
}

fn normalized_game_time_s(game_time_s: Option<f64>) -> Option<u32> {
    let seconds = game_time_s?;
    if !seconds.is_finite() || seconds < 0.0 || seconds > u32::MAX as f64 {
        return None;
    }
    Some(seconds.floor() as u32)
}

fn summoner_spell_asset_key(spell: &PlayerSummonerSpellEntry) -> String {
    let raw = spell.raw_display_name.trim();
    if let Some((_, after_prefix)) = raw.split_once("SummonerSpell_") {
        let key = after_prefix.split('_').next().unwrap_or_default();
        if key.starts_with("Summoner")
            && key.len() > "Summoner".len()
            && key.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            return key.to_string();
        }
    }

    match spell
        .display_name
        .trim()
        .to_ascii_lowercase()
        .replace(|ch: char| !ch.is_ascii_alphanumeric(), "")
        .as_str()
    {
        "barrier" => "SummonerBarrier",
        "clarity" => "SummonerMana",
        "cleanse" => "SummonerBoost",
        "dash" => "SummonerSnowball",
        "exhaust" => "SummonerExhaust",
        "flash" => "SummonerFlash",
        "ghost" => "SummonerHaste",
        "heal" => "SummonerHeal",
        "ignite" => "SummonerDot",
        "mark" => "SummonerSnowball",
        "smite" => "SummonerSmite",
        "teleport" => "SummonerTeleport",
        _ => "",
    }
    .to_string()
}

fn summoner_spell_summary(spell: &PlayerSummonerSpellEntry) -> Option<PlayerSummonerSpell> {
    let name = spell.display_name.trim();
    if name.is_empty() {
        return None;
    }
    Some(PlayerSummonerSpell {
        name: name.to_string(),
        asset_key: summoner_spell_asset_key(spell),
    })
}

fn item_summary(item: &PlayerItemEntry) -> Option<PlayerItem> {
    if item.item_id == 0 {
        return None;
    }
    let name = item.display_name.trim();
    Some(PlayerItem {
        id: item.item_id,
        name: if name.is_empty() {
            item.item_id.to_string()
        } else {
            name.to_string()
        },
        slot: item.slot,
    })
}

pub fn player_summary_from_list_with_game_time(
    players: &[PlayerListEntry],
    local_player: &str,
    game_time_s: Option<f64>,
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
    let participants = players
        .iter()
        .filter_map(|player| {
            let player_name = player.summoner_name.trim();
            let champion_name = player.champion_name.trim();
            if player_name.is_empty() || champion_name.is_empty() {
                return None;
            }
            Some(PlayerParticipant {
                player_name: player_name.to_string(),
                champion_name: champion_name.to_string(),
                team: player.team.trim().to_string(),
            })
        })
        .collect();
    let summoner_spells = [
        player.summoner_spells.one.as_ref(),
        player.summoner_spells.two.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(summoner_spell_summary)
    .collect();
    let mut items: Vec<_> = player.items.iter().filter_map(item_summary).collect();
    items.sort_by_key(|item| item.slot.unwrap_or(u32::MAX));

    Some(PlayerSummary {
        champion_name: champion_name.to_string(),
        kills: player.scores.kills,
        deaths: player.scores.deaths,
        assists: player.scores.assists,
        creep_score: player.scores.creep_score,
        game_time_s: normalized_game_time_s(game_time_s),
        player_name: player.summoner_name.trim().to_string(),
        team: player.team.trim().to_string(),
        participants,
        summoner_spells,
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{PlayerListEntry, PlayerScores, PlayerSummonerSpells};

    #[test]
    fn is_loopback_url_accepts_loopback_variants() {
        assert_eq!(is_loopback_url("https://127.0.0.1:2999"), Some(true));
        assert_eq!(is_loopback_url("https://127.0.0.1"), Some(true));
        assert_eq!(is_loopback_url("http://localhost:1234"), Some(true));
        assert_eq!(is_loopback_url("https://[::1]:2999"), Some(true));
    }

    #[test]
    fn is_loopback_url_rejects_remote_hosts() {
        assert_eq!(is_loopback_url("https://example.com"), Some(false));
        assert_eq!(is_loopback_url("https://192.168.1.1:2999"), Some(false));
        assert_eq!(is_loopback_url("https://10.0.0.1"), Some(false));
    }

    #[test]
    fn is_loopback_url_returns_none_for_garbage() {
        assert_eq!(is_loopback_url("not a url"), None);
    }

    #[test]
    fn new_rejects_non_loopback_url() {
        let err = LiveClient::new("https://example.com:2999").unwrap_err();
        assert!(matches!(err, Error::NotLoopback(_)));
    }

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
            team: String::new(),
            items: Vec::new(),
            summoner_spells: PlayerSummonerSpells::default(),
            scores: PlayerScores {
                kills,
                deaths,
                assists,
                creep_score: None,
            },
        }
    }

    #[test]
    fn player_summary_matches_summoner_name_or_riot_id() {
        let players = [
            player("Someone", Some("Someone#NA1"), "Ahri", 1, 2, 3),
            player("dain", Some("Dain#NA1"), "Nautilus", 3, 4, 23),
        ];

        let by_riot_id =
            player_summary_from_list_with_game_time(&players, "dain#NA1", None).unwrap();
        assert_eq!(by_riot_id.champion_name, "Nautilus");
        assert_eq!(
            (by_riot_id.kills, by_riot_id.deaths, by_riot_id.assists),
            (3, 4, 23)
        );

        let by_summoner =
            player_summary_from_list_with_game_time(&players, " DAIN ", None).unwrap();
        assert_eq!(by_summoner.champion_name, "Nautilus");
    }

    #[test]
    fn player_summary_carries_participants_and_team() {
        let players: Vec<PlayerListEntry> = serde_json::from_str(
            r#"[
              {
                "summonerName": "dain",
                "riotId": "Dain#NA1",
                "championName": "Nautilus",
                "team": "ORDER",
                "scores": { "kills": 3, "deaths": 4, "assists": 23, "creepScore": 187 }
              },
              {
                "summonerName": "Soupmaster",
                "riotId": "Soup#NA1",
                "championName": "Ahri",
                "team": "CHAOS",
                "scores": { "kills": 7, "deaths": 2, "assists": 4, "creepScore": 120 }
              }
            ]"#,
        )
        .unwrap();

        let summary =
            player_summary_from_list_with_game_time(&players, "dain#NA1", Some(1800.4)).unwrap();

        assert_eq!(summary.player_name, "dain");
        assert_eq!(summary.creep_score, Some(187));
        assert_eq!(summary.game_time_s, Some(1800));
        assert_eq!(summary.team, "ORDER");
        assert_eq!(summary.participants.len(), 2);
        assert_eq!(summary.participants[0].player_name, "dain");
        assert_eq!(summary.participants[0].champion_name, "Nautilus");
        assert_eq!(summary.participants[0].team, "ORDER");
        assert_eq!(summary.participants[1].player_name, "Soupmaster");
        assert_eq!(summary.participants[1].champion_name, "Ahri");
        assert_eq!(summary.participants[1].team, "CHAOS");
    }

    #[test]
    fn player_summary_carries_summoner_spells_and_item_build() {
        let players: Vec<PlayerListEntry> = serde_json::from_str(
            r#"[
              {
                "summonerName": "dain",
                "riotId": "Dain#NA1",
                "championName": "Vel'Koz",
                "team": "ORDER",
                "summonerSpells": {
                  "summonerSpellOne": {
                    "displayName": "Ignite",
                    "rawDisplayName": "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"
                  },
                  "summonerSpellTwo": {
                    "displayName": "Flash",
                    "rawDisplayName": "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName"
                  }
                },
                "items": [
                  { "itemID": 1056, "displayName": "Doran's Ring", "slot": 0 },
                  { "itemID": 3020, "displayName": "Sorcerer's Shoes", "slot": 1 },
                  { "itemID": 6655, "displayName": "Luden's Companion", "slot": 2 }
                ],
                "scores": { "kills": 11, "deaths": 19, "assists": 34, "creepScore": 204 }
              }
            ]"#,
        )
        .unwrap();

        let summary = player_summary_from_list_with_game_time(&players, "dain#NA1", None).unwrap();
        let value = serde_json::to_value(summary).unwrap();

        assert_eq!(value["summoner_spells"][0]["name"], "Ignite");
        assert_eq!(value["summoner_spells"][0]["asset_key"], "SummonerDot");
        assert_eq!(value["summoner_spells"][1]["name"], "Flash");
        assert_eq!(value["summoner_spells"][1]["asset_key"], "SummonerFlash");
        assert_eq!(value["items"][0]["id"], 1056);
        assert_eq!(value["items"][0]["name"], "Doran's Ring");
        assert_eq!(value["items"][0]["slot"], 0);
        assert_eq!(value["items"][2]["id"], 6655);
    }

    #[test]
    fn player_summary_ignores_missing_local_player_or_champion() {
        let players = [player("dain", Some("Dain#NA1"), "", 3, 4, 23)];

        assert!(player_summary_from_list_with_game_time(&players, "other", None).is_none());
        assert!(player_summary_from_list_with_game_time(&players, "dain", None).is_none());
    }
}
