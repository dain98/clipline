use std::time::Instant;

use clipline_lol::{poll_once, EventTracker, LiveClient};
use httpmock::prelude::*;
use serde_json::json;

fn mount_gamestats(server: &MockServer, game_time: f64) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/gamestats");
        then.status(200).json_body(json!({ "gameTime": game_time }));
    })
}

fn mount_events(server: &MockServer, events: serde_json::Value) -> httpmock::Mock<'_> {
    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/eventdata");
        then.status(200).json_body(json!({ "Events": events }));
    })
}

#[tokio::test]
async fn polls_dedupe_and_stamp_recording_offsets() {
    let server = MockServer::start();
    let mut stats = mount_gamestats(&server, 100.0);
    let mut events = mount_events(
        &server,
        json!([
            { "EventID": 0, "EventName": "GameStart", "EventTime": 0.05 },
            { "EventID": 1, "EventName": "ChampionKill", "EventTime": 95.0,
              "KillerName": "Me", "VictimName": "Them", "Assisters": [] }
        ]),
    );

    let client = LiveClient::new(server.base_url()).unwrap();
    let mut tracker = EventTracker::default();
    let t0 = Instant::now();

    let batch = poll_once(&client, &mut tracker, "Me", t0, 0.0).await.unwrap();
    assert_eq!(batch.len(), 2);
    let kill = &batch[1];
    assert!(kill.involves_local_player);
    // Game clock read 100.0 moments after t0; EventTime 95.0 → offset ≈ -5s
    // relative to the anchor (event happened ~5s of game time ago).
    let off = kill.recording_offset_s.unwrap();
    assert!((-5.5..=-4.5).contains(&off), "offset {off} not near -5.0");

    // Second poll: one new event appended; only it is returned.
    stats.delete();
    events.delete();
    mount_gamestats(&server, 110.0);
    mount_events(
        &server,
        json!([
            { "EventID": 0, "EventName": "GameStart", "EventTime": 0.05 },
            { "EventID": 1, "EventName": "ChampionKill", "EventTime": 95.0,
              "KillerName": "Me", "VictimName": "Them", "Assisters": [] },
            { "EventID": 2, "EventName": "Multikill", "EventTime": 109.0,
              "KillerName": "Me", "KillStreak": 2 }
        ]),
    );

    let batch2 = poll_once(&client, &mut tracker, "Me", t0, 0.0).await.unwrap();
    assert_eq!(batch2.len(), 1);
    assert_eq!(batch2[0].subtype.as_deref(), Some("2"));
}
