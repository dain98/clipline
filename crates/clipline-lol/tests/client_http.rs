use clipline_lol::LiveClient;
use httpmock::prelude::*;
use serde_json::json;

#[tokio::test]
async fn fetches_and_parses_all_three_endpoints() {
    let server = MockServer::start();

    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/eventdata");
        then.status(200).json_body(json!({
            "Events": [
                { "EventID": 0, "EventName": "GameStart", "EventTime": 0.05 }
            ]
        }));
    });
    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/activeplayername");
        then.status(200).json_body(json!("Me#NA1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/gamestats");
        then.status(200).json_body(json!({
            "gameMode": "CLASSIC", "gameTime": 123.5, "mapName": "Map11"
        }));
    });
    server.mock(|when, then| {
        when.method(GET).path("/liveclientdata/playerlist");
        then.status(200).json_body(json!([
            {
                "summonerName": "Me",
                "riotId": "Me#NA1",
                "championName": "Nautilus",
                "scores": { "kills": 3, "deaths": 4, "assists": 23, "creepScore": 187 }
            }
        ]));
    });

    let client = LiveClient::new(server.base_url()).unwrap();
    let data = client.event_data().await.unwrap();
    assert_eq!(data.events.len(), 1);
    assert_eq!(client.active_player_name().await.unwrap(), "Me#NA1");
    assert!((client.game_time_s().await.unwrap() - 123.5).abs() < 1e-9);
    let summary = client.player_summary("Me#NA1").await.unwrap().unwrap();
    assert_eq!(summary.champion_name, "Nautilus");
    assert_eq!((summary.kills, summary.deaths, summary.assists), (3, 4, 23));
    assert_eq!(summary.creep_score, Some(187));
    assert_eq!(summary.game_time_s, Some(123));
}

#[tokio::test]
async fn connection_refused_is_an_error_not_a_panic() {
    // Nothing listens on this port.
    let client = LiveClient::new("http://127.0.0.1:9").unwrap();
    assert!(client.event_data().await.is_err());
}
