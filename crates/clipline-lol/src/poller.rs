use std::time::Instant;

use clipline_events::{recording_offset_s, ClockAnchor, GameEvent};

use crate::client::{Error, LiveClient};
use crate::normalize::normalize;
use crate::tracker::EventTracker;

/// One poll of the Live Client Data API (ddoc §5a, poll cadence ~1–2 Hz):
/// samples a fresh clock anchor, fetches events, dedupes by EventID,
/// normalizes, and stamps each new event's recording offset.
pub async fn poll_once(
    client: &LiveClient,
    tracker: &mut EventTracker,
    local_player: &str,
    recording_t0: Instant,
    emit_latency_s: f64,
) -> Result<Vec<GameEvent>, Error> {
    // Anchor first, paired with the wall clock at the moment of sampling.
    // Re-sampling every poll lets game-clock pauses self-correct (ddoc §5).
    let game_time_s = client.game_time_s().await?;
    let anchor = ClockAnchor { game_time_s, sampled_at: Instant::now() };

    let data = client.event_data().await?;
    let events = tracker
        .fresh(&data.events)
        .into_iter()
        .map(|raw| {
            let mut ev = normalize(raw, local_player);
            ev.recording_offset_s = Some(recording_offset_s(
                raw.event_time,
                anchor,
                recording_t0,
                emit_latency_s,
            ));
            ev
        })
        .collect();
    Ok(events)
}
