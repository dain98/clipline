use crate::raw::RawEvent;
use clipline_events::{EventKind, GameEvent, GameId};

/// Importance added when the local player is the event actor.
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

    let local_player_identity = PlayerNameIdentity::new(local_player);
    let actor_is_local = local_player_identity.matches(&actor);
    let local_assister = raw
        .assisters
        .iter()
        .find(|assister| local_player_identity.matches(assister))
        .cloned();
    let local_player_assisted = local_assister.is_some();
    let local_player_was_victim = raw
        .victim_name
        .as_deref()
        .is_some_and(|victim| local_player_identity.matches(victim));

    // The Live Client API only emits ChampionKill for any champion death. We
    // split local involvement so the review timeline can distinguish the local
    // player's kills, assists, and deaths.
    let kind = match kind {
        EventKind::ChampionKill if local_player_was_victim => EventKind::ChampionDeath,
        EventKind::ChampionKill if local_player_assisted && !actor_is_local => {
            EventKind::ChampionAssist
        }
        k => k,
    };

    let involves_local_player = match kind {
        EventKind::ChampionAssist | EventKind::ChampionDeath => true,
        EventKind::ChampionKill => actor_is_local,
        _ => actor_is_local || local_player_assisted || local_player_was_victim,
    };

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
    let actor = match kind {
        EventKind::ChampionAssist => local_assister.unwrap_or_else(|| local_player.trim().into()),
        _ => actor,
    };

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayerNameIdentity {
    game_name: String,
    full_riot_id: Option<String>,
}

impl PlayerNameIdentity {
    pub(crate) fn new(name: &str) -> Self {
        let trimmed = name.trim();
        let (game_name, tagline) = trimmed
            .split_once('#')
            .map_or((trimmed, None), |(game_name, tagline)| {
                (game_name.trim(), Some(tagline.trim()))
            });
        let game_name = game_name.to_lowercase();
        let full_riot_id = tagline
            .filter(|tagline| {
                !game_name.is_empty() && !tagline.is_empty() && !tagline.contains('#')
            })
            .map(|tagline| format!("{game_name}#{}", tagline.to_lowercase()));
        Self {
            game_name,
            full_riot_id,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.game_name.is_empty()
    }

    pub(crate) fn has_tagline(&self) -> bool {
        self.full_riot_id.is_some()
    }

    pub(crate) fn exact_match(&self, candidate: &str) -> bool {
        let candidate = Self::new(candidate);
        matches!(
            (&self.full_riot_id, &candidate.full_riot_id),
            (Some(local), Some(candidate)) if local == candidate
        )
    }

    pub(crate) fn matches(&self, candidate: &str) -> bool {
        if self.is_empty() {
            return false;
        }
        let candidate = Self::new(candidate);
        if candidate.is_empty() {
            return false;
        }
        match (&self.full_riot_id, &candidate.full_riot_id) {
            (Some(local), Some(candidate)) => local == candidate,
            _ => self.game_name == candidate.game_name,
        }
    }
}

fn base_importance(kind: EventKind) -> u8 {
    match kind {
        EventKind::Ace => 8,
        EventKind::Multikill | EventKind::BaronKill => 7,
        EventKind::DragonKill | EventKind::FirstBlood => 6,
        EventKind::ChampionKill
        | EventKind::ChampionAssist
        | EventKind::ChampionDeath
        | EventKind::InhibKilled
        | EventKind::HeraldKill => 5,
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
    fn local_player_as_victim_becomes_champion_death() {
        let r = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": "A", "VictimName": "Me", "Assisters": ["B"] }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(
            ev.kind,
            EventKind::ChampionDeath,
            "the local player's own death is classified distinctly from a kill"
        );
        assert!(
            ev.involves_local_player,
            "deaths are timeline-worthy for the local player"
        );
        assert_eq!(ev.importance, 7); // base 5 + local boost 2
        assert_eq!(ev.actor, "A");
        assert_eq!(ev.victim.as_deref(), Some("Me"));
    }

    #[test]
    fn local_player_as_assister_becomes_champion_assist() {
        let r = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": "A", "VictimName": "Them", "Assisters": ["B"] }"#,
        );
        let ev = normalize(&r, "B");
        assert_eq!(ev.kind, EventKind::ChampionAssist);
        assert_eq!(ev.actor, "B");
        assert!(
            ev.involves_local_player,
            "local assists should get timeline-worthy assist markers"
        );
        assert_eq!(ev.importance, 7);
    }

    #[test]
    fn local_player_matching_ignores_case_whitespace_and_riot_tagline() {
        let r = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": " dain ", "VictimName": "Them", "Assisters": [] }"#,
        );
        assert!(normalize(&r, "Dain#NA1").involves_local_player);
    }

    #[test]
    fn local_player_matching_distinguishes_present_riot_taglines() {
        let foreign = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": " dain # EUW ", "VictimName": "Them", "Assisters": [] }"#,
        );
        assert!(
            !normalize(&foreign, "Dain#NA1").involves_local_player,
            "two present taglines must not collapse to the same game-name key"
        );

        let legacy = raw(
            r#"{ "EventID": 3, "EventName": "ChampionKill", "EventTime": 11.0,
                 "KillerName": " DAIN ", "VictimName": "Them", "Assisters": [] }"#,
        );
        assert!(
            normalize(&legacy, " dain # na1 ").involves_local_player,
            "an untagged event payload must retain the compatibility fallback"
        );
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
