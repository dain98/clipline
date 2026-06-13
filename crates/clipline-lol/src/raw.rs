use serde::Deserialize;

/// Response body of `GET /liveclientdata/eventdata`.
#[derive(Debug, Clone, Deserialize)]
pub struct EventData {
    #[serde(rename = "Events")]
    pub events: Vec<RawEvent>,
}

/// One raw Live Client Data event. Field set is the union of all event
/// types in Riot's sample (ddoc §5a) — absent fields deserialize to None.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "EventID")]
    pub event_id: u64,
    #[serde(rename = "EventName")]
    pub event_name: String,
    /// Seconds of game time.
    #[serde(rename = "EventTime")]
    pub event_time: f64,
    #[serde(rename = "KillerName")]
    pub killer_name: Option<String>,
    #[serde(rename = "VictimName")]
    pub victim_name: Option<String>,
    #[serde(rename = "Assisters", default)]
    pub assisters: Vec<String>,
    #[serde(rename = "KillStreak")]
    pub kill_streak: Option<u32>,
    #[serde(rename = "DragonType")]
    pub dragon_type: Option<String>,
    /// String "True"/"False" in Riot's sample, not a bool.
    #[serde(rename = "Stolen")]
    pub stolen: Option<String>,
    #[serde(rename = "TurretKilled")]
    pub turret_killed: Option<String>,
    #[serde(rename = "InhibKilled")]
    pub inhib_killed: Option<String>,
    #[serde(rename = "Acer")]
    pub acer: Option<String>,
    #[serde(rename = "AcingTeam")]
    pub acing_team: Option<String>,
    // Community-observed fields (FirstBlood / GameEnd) — defensive only.
    #[serde(rename = "Recipient")]
    pub recipient: Option<String>,
    #[serde(rename = "Result")]
    pub result: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like Riot's official liveclientdata_events.json sample.
    const FIXTURE: &str = r#"{
      "Events": [
        { "EventID": 0, "EventName": "GameStart", "EventTime": 0.05 },
        { "EventID": 3, "EventName": "ChampionKill", "EventTime": 300.2,
          "VictimName": "Shaco", "KillerName": "Teemo", "Assisters": ["Lux"] },
        { "EventID": 4, "EventName": "Multikill", "EventTime": 305.0,
          "KillerName": "Teemo", "KillStreak": 2 },
        { "EventID": 5, "EventName": "DragonKill", "EventTime": 1000.4,
          "DragonType": "Earth", "Stolen": "False",
          "KillerName": "Teemo", "Assisters": [] }
      ]
    }"#;

    #[test]
    fn parses_riot_sample_shaped_payload() {
        let data: EventData = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(data.events.len(), 4);

        let kill = &data.events[1];
        assert_eq!(kill.event_id, 3);
        assert_eq!(kill.event_name, "ChampionKill");
        assert_eq!(kill.killer_name.as_deref(), Some("Teemo"));
        assert_eq!(kill.victim_name.as_deref(), Some("Shaco"));
        assert_eq!(kill.assisters, vec!["Lux".to_string()]);

        let multi = &data.events[2];
        assert_eq!(multi.kill_streak, Some(2));
        assert!(
            multi.assisters.is_empty(),
            "missing Assisters defaults to empty"
        );

        let dragon = &data.events[3];
        assert_eq!(dragon.dragon_type.as_deref(), Some("Earth"));
        assert_eq!(dragon.stolen.as_deref(), Some("False"));
    }

    #[test]
    fn unknown_event_names_still_parse() {
        let json = r#"{ "Events": [
          { "EventID": 9, "EventName": "SomeFutureThing", "EventTime": 12.0 }
        ] }"#;
        let data: EventData = serde_json::from_str(json).unwrap();
        assert_eq!(data.events[0].event_name, "SomeFutureThing");
    }
}
