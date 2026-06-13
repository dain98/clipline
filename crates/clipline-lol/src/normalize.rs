use crate::raw::RawEvent;
use clipline_events::{EventKind, GameEvent, GameId};

/// Importance added when the local player is killer, victim, or assister.
const LOCAL_PLAYER_BOOST: u8 = 2;

/// Normalize one raw Live Client event (ddoc §5a). `recording_offset_s` is
/// left as None; the poller anchors it.
pub fn normalize(raw: &RawEvent, local_player: &str) -> GameEvent {
    let kind = match raw.event_name.as_str() {
        "GameStart" => EventKind::GameStart,
        "MinionsSpawning" => EventKind::MinionsSpawning,
        "FirstBrick" => EventKind::FirstBrick,
        "TurretKilled" => EventKind::TurretKilled,
        "InhibKilled" => EventKind::InhibKilled,
        "DragonKill" => EventKind::DragonKill,
        "HeraldKill" => EventKind::HeraldKill,
        "BaronKill" => EventKind::BaronKill,
        "ChampionKill" => EventKind::ChampionKill,
        "Multikill" => EventKind::Multikill,
        "Ace" => EventKind::Ace,
        "FirstBlood" => EventKind::FirstBlood,
        "GameEnd" => EventKind::GameEnd,
        _ => EventKind::Other,
    };

    let actor = raw
        .killer_name
        .clone()
        .or_else(|| raw.acer.clone())
        .or_else(|| raw.recipient.clone())
        .unwrap_or_default();

    let involves_local_player = !local_player.is_empty()
        && (actor == local_player
            || raw.victim_name.as_deref() == Some(local_player)
            || raw.assisters.iter().any(|a| a == local_player));

    let subtype = match kind {
        EventKind::DragonKill => raw.dragon_type.clone(),
        EventKind::Multikill => raw.kill_streak.map(|k| k.to_string()),
        EventKind::TurretKilled => raw.turret_killed.clone(),
        EventKind::InhibKilled => raw.inhib_killed.clone(),
        EventKind::Ace => raw.acing_team.clone(),
        EventKind::GameEnd => raw.result.clone(),
        _ => None,
    };

    let importance = (base_importance(kind)
        + if involves_local_player {
            LOCAL_PLAYER_BOOST
        } else {
            0
        })
    .min(10);

    GameEvent {
        game_id: GameId::LeagueOfLegends,
        kind,
        actor,
        victim: raw.victim_name.clone(),
        assisters: raw.assisters.clone(),
        subtype,
        game_time_s: raw.event_time,
        recording_offset_s: None,
        importance,
        involves_local_player,
    }
}

fn base_importance(kind: EventKind) -> u8 {
    match kind {
        EventKind::Ace => 8,
        EventKind::Multikill | EventKind::BaronKill => 7,
        EventKind::DragonKill | EventKind::FirstBlood => 6,
        EventKind::ChampionKill | EventKind::InhibKilled | EventKind::HeraldKill => 5,
        EventKind::TurretKilled | EventKind::FirstBrick => 4,
        EventKind::GameEnd => 3,
        EventKind::GameStart | EventKind::MinionsSpawning | EventKind::Other => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawEvent;
    use clipline_events::EventKind;

    fn raw(json: &str) -> RawEvent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn champion_kill_by_local_player_is_boosted() {
        let r = raw(
            r#"{ "EventID": 1, "EventName": "ChampionKill", "EventTime": 300.0,
                 "KillerName": "Me", "VictimName": "Them", "Assisters": [] }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::ChampionKill);
        assert!(ev.involves_local_player);
        assert_eq!(ev.actor, "Me");
        assert_eq!(ev.victim.as_deref(), Some("Them"));
        assert_eq!(ev.importance, 7); // base 5 + local boost 2

        let bystander = normalize(&r, "SomeoneElse");
        assert!(!bystander.involves_local_player);
        assert_eq!(bystander.importance, 5);
    }

    #[test]
    fn local_player_as_victim_or_assister_counts_as_involved() {
        let r = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": "A", "VictimName": "Me", "Assisters": ["B"] }"#,
        );
        assert!(normalize(&r, "Me").involves_local_player);
        assert!(normalize(&r, "B").involves_local_player);
    }

    #[test]
    fn dragon_kill_carries_type_subtype() {
        let r = raw(
            r#"{ "EventID": 3, "EventName": "DragonKill", "EventTime": 1000.0,
                 "DragonType": "Hextech", "Stolen": "False", "KillerName": "A",
                 "Assisters": [] }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::DragonKill);
        assert_eq!(ev.subtype.as_deref(), Some("Hextech"));
    }

    #[test]
    fn multikill_carries_streak_subtype() {
        let r = raw(
            r#"{ "EventID": 4, "EventName": "Multikill", "EventTime": 305.0,
                 "KillerName": "Me", "KillStreak": 3 }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::Multikill);
        assert_eq!(ev.subtype.as_deref(), Some("3"));
    }

    #[test]
    fn unknown_event_maps_to_other_not_an_error() {
        let r = raw(r#"{ "EventID": 5, "EventName": "Brand New", "EventTime": 1.0 }"#);
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::Other);
        assert_eq!(ev.game_time_s, 1.0);
    }
}
