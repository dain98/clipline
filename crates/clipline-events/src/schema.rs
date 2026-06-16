use serde::{Deserialize, Serialize};

/// Which game produced an event (ddoc §5 normalized schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameId {
    LeagueOfLegends,
    Valorant,
    Cs2,
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
    Multikill,
    Ace,
    // Community-observed, not in Riot's official sample (ddoc §5a) —
    // parsed defensively, never relied upon.
    FirstBlood,
    GameEnd,
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

/// Product-level timeline policy: keep only markers that are useful while
/// reviewing a match, not every event the game API reports.
pub fn is_timeline_marker(event: &GameEvent) -> bool {
    match event.kind {
        EventKind::ChampionKill => event.involves_local_player,
        EventKind::TurretKilled | EventKind::DragonKill | EventKind::BaronKill => true,
        _ => false,
    }
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
    fn timeline_policy_keeps_only_review_worthy_markers() {
        assert!(is_timeline_marker(&ev(EventKind::ChampionKill, true)));
        assert!(!is_timeline_marker(&ev(EventKind::ChampionKill, false)));
        assert!(is_timeline_marker(&ev(EventKind::TurretKilled, false)));
        assert!(is_timeline_marker(&ev(EventKind::DragonKill, false)));
        assert!(is_timeline_marker(&ev(EventKind::BaronKill, false)));

        for kind in [
            EventKind::GameStart,
            EventKind::MinionsSpawning,
            EventKind::FirstBrick,
            EventKind::InhibKilled,
            EventKind::HeraldKill,
            EventKind::Multikill,
            EventKind::Ace,
            EventKind::FirstBlood,
            EventKind::GameEnd,
            EventKind::Other,
        ] {
            assert!(
                !is_timeline_marker(&ev(kind, true)),
                "{kind:?} should not be shown on the review timeline"
            );
        }
    }
}
