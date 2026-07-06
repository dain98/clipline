use serde::{Deserialize, Serialize};

/// Which game produced an event (ddoc §5 normalized schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameId {
    LeagueOfLegends,
    Valorant,
    Cs2,
    Osu,
}

/// Normalized event kind across all game adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    GameStart,
    MinionsSpawning,
    FirstBrick,
    TurretKilled,
    InhibKilled,
    DragonKill,
    HeraldKill,
    BaronKill,
    ChampionKill,
    ChampionAssist,
    ChampionDeath,
    Multikill,
    Ace,
    // Community-observed, not in Riot's official sample (ddoc §5a) —
    // parsed defensively, never relied upon.
    FirstBlood,
    GameEnd,
    // CS2 Game State Integration kinds. Own-play GSI carries no victim
    // names or roster, so these are actor-light by design.
    PlayerKill,
    PlayerDeath,
    PlayerAssist,
    BombPlanted,
    BombDefused,
    BombExploded,
    RoundWon,
    RoundLost,
    Mvp,
    Other,
}

/// One normalized timeline event (ddoc §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameEvent {
    pub game_id: GameId,
    pub kind: EventKind,
    /// Killer / acer / local actor. Empty string when the source has none.
    pub actor: String,
    pub victim: Option<String>,
    pub assisters: Vec<String>,
    /// DragonType, kill-streak count, turret id, acing team, game result, …
    pub subtype: Option<String>,
    /// Seconds of game clock, as reported by the source.
    pub game_time_s: f64,
    /// Position on the recording timeline; None until anchored (ddoc §5).
    pub recording_offset_s: Option<f64>,
    /// 0–10, drives auto-clip thresholds later.
    pub importance: u8,
    pub involves_local_player: bool,
}

/// Storage/review policy: keep events that a review surface may show after
/// user filtering. This is intentionally broader than the default timeline.
pub fn is_review_event(event: &GameEvent) -> bool {
    matches!(
        event.kind,
        EventKind::ChampionKill
            | EventKind::ChampionAssist
            | EventKind::ChampionDeath
            | EventKind::TurretKilled
            | EventKind::InhibKilled
            | EventKind::DragonKill
            | EventKind::HeraldKill
            | EventKind::BaronKill
            | EventKind::PlayerKill
            | EventKind::PlayerDeath
            | EventKind::PlayerAssist
            | EventKind::BombPlanted
            | EventKind::BombDefused
            | EventKind::BombExploded
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: EventKind, involves_local_player: bool) -> GameEvent {
        GameEvent {
            game_id: GameId::LeagueOfLegends,
            kind,
            actor: "Killer".into(),
            victim: Some("Victim".into()),
            assisters: vec!["Helper".into()],
            subtype: None,
            game_time_s: 312.5,
            recording_offset_s: Some(95.25),
            importance: 7,
            involves_local_player,
        }
    }

    #[test]
    fn game_event_serde_roundtrip() {
        let ev = GameEvent {
            game_id: GameId::LeagueOfLegends,
            kind: EventKind::ChampionKill,
            actor: "Killer".into(),
            victim: Some("Victim".into()),
            assisters: vec!["Helper".into()],
            subtype: None,
            game_time_s: 312.5,
            recording_offset_s: Some(95.25),
            importance: 7,
            involves_local_player: true,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: GameEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn review_event_policy_keeps_match_event_sources_without_timeline_noise() {
        assert!(is_review_event(&ev(EventKind::ChampionKill, true)));
        assert!(is_review_event(&ev(EventKind::ChampionKill, false)));
        assert!(is_review_event(&ev(EventKind::ChampionAssist, true)));
        assert!(is_review_event(&ev(EventKind::ChampionDeath, false)));
        assert!(is_review_event(&ev(EventKind::TurretKilled, false)));
        assert!(is_review_event(&ev(EventKind::InhibKilled, false)));
        assert!(is_review_event(&ev(EventKind::DragonKill, false)));
        assert!(is_review_event(&ev(EventKind::HeraldKill, false)));
        assert!(is_review_event(&ev(EventKind::BaronKill, false)));
        assert!(is_review_event(&ev(EventKind::PlayerKill, true)));
        assert!(is_review_event(&ev(EventKind::PlayerDeath, true)));
        assert!(is_review_event(&ev(EventKind::PlayerAssist, true)));
        assert!(is_review_event(&ev(EventKind::BombPlanted, false)));
        assert!(is_review_event(&ev(EventKind::BombDefused, false)));
        assert!(is_review_event(&ev(EventKind::BombExploded, false)));

        for kind in [
            EventKind::GameStart,
            EventKind::MinionsSpawning,
            EventKind::FirstBrick,
            EventKind::Multikill,
            EventKind::Ace,
            EventKind::FirstBlood,
            EventKind::GameEnd,
            EventKind::RoundWon,
            EventKind::RoundLost,
            EventKind::Mvp,
            EventKind::Other,
        ] {
            assert!(
                !is_review_event(&ev(kind, true)),
                "{kind:?} should not be stored as a review event"
            );
        }
    }
}
