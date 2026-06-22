//! League event source (ddoc §5a): a thread with a current-thread tokio
//! runtime polling the Live Client Data API at ~1 Hz, forwarding anchored
//! events to the recorder service. Quietly waits while no game runs; the
//! API only exists in-game. The League game plugin owns when this source is
//! attached to a recorder session.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use clipline_events::{GameEvent, PlayerSummary};
use clipline_lol::{poll_once, EventTracker, LiveClient};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// What the poller tells the service: anchored events, plus the match
/// boundaries it observes (the Live Client API only exists in-game, so
/// connecting/losing it *is* the boundary signal).
pub enum PollerMsg {
    Event(GameEvent),
    PlayerSummary(PlayerSummary),
    MatchStarted,
    MatchEnded,
}

/// Spawn the poller. `base_url` overrides the local Live Client endpoint
/// (mock servers in tests/demos); `recording_t0` is the wall-clock twin of
/// the capture clock origin — sample them together.
pub fn spawn(base_url: Option<String>, recording_t0: Instant) -> Receiver<PollerMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipline-lol-poller".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("lol poller: failed to build tokio runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let client = match base_url {
                    Some(url) => LiveClient::new(url),
                    None => LiveClient::default_local(),
                };
                let client = match client {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("lol poller: failed to create live client: {e}");
                        return;
                    }
                };
                loop {
                    // Wait for a game: the endpoint 404s/refuses otherwise.
                    let local_player = loop {
                        match client.active_player_name().await {
                            Ok(name) => break name,
                            Err(_) => tokio::time::sleep(RETRY_INTERVAL).await,
                        }
                    };
                    if tx.send(PollerMsg::MatchStarted).is_err() {
                        return; // service gone
                    }
                    if let Ok(Some(summary)) = client.player_summary(&local_player).await {
                        if tx.send(PollerMsg::PlayerSummary(summary)).is_err() {
                            return;
                        }
                    }
                    let mut tracker = EventTracker::default();
                    loop {
                        match poll_once(&client, &mut tracker, &local_player, recording_t0, 0.0)
                            .await
                        {
                            Ok(events) => {
                                for ev in events {
                                    if tx.send(PollerMsg::Event(ev)).is_err() {
                                        return; // service gone
                                    }
                                }
                            }
                            Err(_) => break, // game ended — back to waiting
                        }
                        if let Ok(Some(summary)) = client.player_summary(&local_player).await {
                            if tx.send(PollerMsg::PlayerSummary(summary)).is_err() {
                                return;
                            }
                        }
                        tokio::time::sleep(POLL_INTERVAL).await;
                    }
                    if tx.send(PollerMsg::MatchEnded).is_err() {
                        return;
                    }
                }
            });
        })
        .expect("spawn lol poller thread");
    rx
}
