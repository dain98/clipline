//! Milestone 6 chain proof: poll the (mock) Live Client API → anchored
//! events → MarkerLog → clip window → sidecar JSON. What the app's poller
//! thread + recorder service do, with the device layer removed.

use std::time::{Duration, Instant};

use clipline_events::{ClipMarkers, EventKind, MarkerLog};
use clipline_lol::{poll_once, EventTracker, LiveClient};
use httpmock::prelude::*;
use serde_json::json;

fn mount(server: &MockServer, game_time: f64, events: serde_json::Value) -> Vec<httpmock::Mock<'_>> {
    vec![
        server.mock(|when, then| {
            when.method(GET).path("/liveclientdata/gamestats");
            then.status(200).json_body(json!({ "gameTime": game_time }));
        }),
        server.mock(|when, then| {
            when.method(GET).path("/liveclientdata/eventdata");
            then.status(200).json_body(json!({ "Events": events }));
        }),
    ]
}

#[tokio::test]
async fn polled_events_land_in_the_saved_clip_sidecar() {
    let server = MockServer::start();
    // The recording started 100 s of wall time ago (mock polls run in
    // microseconds, so the anchor's wall-clock term comes from backdating
    // t0); the game clock reads 100 s at the first poll.
    let t0 = Instant::now() - Duration::from_secs(100);
    let mut mocks = mount(
        &server,
        100.0,
        json!([
            // 60 s before the anchor → recording offset ≈ 40 s: OUTSIDE a
            // clip window starting at 90 s.
            { "EventID": 1, "EventName": "ChampionKill", "EventTime": 40.0,
              "KillerName": "Me", "VictimName": "Early", "Assisters": [] }
        ]),
    );

    let client = LiveClient::new(server.base_url()).unwrap();
    let mut tracker = EventTracker::default();
    let mut log = MarkerLog::new();

    for ev in poll_once(&client, &mut tracker, "Me", t0, 0.0).await.unwrap() {
        log.push(ev);
    }

    // Later poll: game clock 200 s, a dragon at 195 s → offset ≈ 95 s.
    for mut m in mocks.drain(..) {
        m.delete();
    }
    mount(
        &server,
        200.0,
        json!([
            { "EventID": 1, "EventName": "ChampionKill", "EventTime": 40.0,
              "KillerName": "Me", "VictimName": "Early", "Assisters": [] },
            { "EventID": 2, "EventName": "DragonKill", "EventTime": 195.0,
              "KillerName": "Me", "DragonType": "Infernal", "Assisters": [] }
        ]),
    );
    for ev in poll_once(&client, &mut tracker, "Me", t0, 0.0).await.unwrap() {
        log.push(ev);
    }
    assert_eq!(log.len(), 2, "both events anchored and logged");

    // Save Replay over recording window [90 s, 150 s).
    let clip = log.clip_markers(90.0, 150.0);
    assert_eq!(clip.markers.len(), 1, "early kill excluded, dragon included");
    let dragon = &clip.markers[0];
    assert_eq!(dragon.event.kind, EventKind::DragonKill);
    assert_eq!(dragon.event.subtype.as_deref(), Some("Infernal"));
    // offset ≈ 95 s (mock polls run in microseconds) → clip time ≈ 5 s.
    assert!((4.5..=5.5).contains(&dragon.t_s), "clip time {} not near 5.0", dragon.t_s);

    // The sidecar document round-trips with the data intact.
    let json = serde_json::to_string_pretty(&clip).unwrap();
    let back: ClipMarkers = serde_json::from_str(&json).unwrap();
    assert_eq!(back.markers.len(), 1);
    assert!(back.markers[0].event.involves_local_player);
}
