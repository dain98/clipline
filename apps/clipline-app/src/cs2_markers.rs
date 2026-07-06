//! CS2 event source: a thread running the GSI HTTP listener, forwarding
//! normalized events to the recorder service. Unlike League's poller (we
//! poll Riot's local API), GSI pushes — CS2 POSTs to our loopback endpoint,
//! so match boundaries come from the payload stream, not connectivity.
//!
//! The listener holds a TCP port, so this thread must exit promptly when the
//! service goes away: a heartbeat send on every idle tick detects the
//! dropped receiver and releases the port for the next recording session.

use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use clipline_cs2::{GsiTracker, GsiUpdate, DEFAULT_GSI_ADDR, DEFAULT_GSI_TOKEN};

use crate::markers::PollerMsg;

const IDLE_TICK: Duration = Duration::from_millis(500);

/// Spawn the GSI event source. `addr` overrides the loopback endpoint
/// (tests/mock feeds); `recording_t0` anchors event arrival times onto the
/// recording timeline. GSI carries no game clock in own play, so arrival
/// time (~0.4 s behind the action, within marker tolerance) is the anchor.
pub fn spawn(addr: Option<String>, recording_t0: Instant) -> Receiver<PollerMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("clipline-cs2-events".into())
        .spawn(move || {
            let addr = addr.as_deref().unwrap_or(DEFAULT_GSI_ADDR).to_string();
            let source = match clipline_cs2::bind(&addr, Some(DEFAULT_GSI_TOKEN.into())) {
                Ok((source, _)) => source,
                Err(e) => {
                    eprintln!("cs2 gsi: bind {addr}: {e}");
                    return;
                }
            };
            let mut tracker = GsiTracker::new();
            loop {
                let payload = match source.recv_timeout(IDLE_TICK) {
                    Ok(payload) => payload,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Menus can go silent for many minutes (observed):
                        // the heartbeat exists to notice the service is gone,
                        // not to signal liveness.
                        if tx.send(PollerMsg::Heartbeat).is_err() {
                            return; // service gone — drop `source`, free the port
                        }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                };
                let offset = payload_offset_s(recording_t0);
                for update in tracker.ingest(&payload) {
                    let msg = match update {
                        GsiUpdate::MatchStarted => PollerMsg::MatchStarted,
                        GsiUpdate::MatchEnded => PollerMsg::MatchEnded,
                        GsiUpdate::Summary(summary) => PollerMsg::PlayerSummary(summary),
                        GsiUpdate::Event(mut event) => {
                            event.recording_offset_s = Some(offset);
                            PollerMsg::Event(event)
                        }
                    };
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }
        })
        .expect("spawn cs2 gsi thread");
    rx
}

fn payload_offset_s(recording_t0: Instant) -> f64 {
    Instant::now()
        .saturating_duration_since(recording_t0)
        .as_secs_f64()
}
