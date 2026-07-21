use crate::raw::RawEvent;

/// Watermark over Riot's monotonic `EventID` (ddoc §5a): each poll returns
/// the full event list; only events above the watermark are surfaced.
#[derive(Debug, Default)]
pub struct EventTracker {
    last_seen: Option<u64>,
    last_game_time_s: Option<f64>,
}

impl EventTracker {
    const CLOCK_REGRESSION_GRACE_S: f64 = 5.0;

    /// Observe the identity signals for a successful poll before calling
    /// [`Self::fresh`]. Returns true and clears the event watermark when the
    /// game clock or the largest cumulative event ID moves backwards far
    /// enough to identify a new match.
    pub fn prepare_poll(&mut self, game_time_s: f64, events: &[RawEvent]) -> bool {
        let clock_regressed = self.last_game_time_s.is_some_and(|previous| {
            game_time_s.is_finite()
                && previous.is_finite()
                && game_time_s + Self::CLOCK_REGRESSION_GRACE_S < previous
        });
        let event_ids_regressed = self.last_seen.is_some_and(|seen| {
            events
                .iter()
                .map(|event| event.event_id)
                .max()
                .is_some_and(|maximum| maximum < seen)
        });
        let new_match = clock_regressed || event_ids_regressed;
        if new_match {
            self.last_seen = None;
        }
        if game_time_s.is_finite() && game_time_s >= 0.0 {
            self.last_game_time_s = Some(game_time_s);
        }
        new_match
    }

    /// Returns the not-yet-seen events in ascending `EventID` order and
    /// advances the watermark.
    pub fn fresh<'a>(&mut self, events: &'a [RawEvent]) -> Vec<&'a RawEvent> {
        let mut out: Vec<&RawEvent> = events
            .iter()
            .filter(|e| self.last_seen.is_none_or(|seen| e.event_id > seen))
            .collect();
        out.sort_by_key(|e| e.event_id);
        if let Some(last) = out.last() {
            self.last_seen = Some(last.event_id);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawEvent;

    fn ev(id: u64) -> RawEvent {
        serde_json::from_str(&format!(
            r#"{{ "EventID": {id}, "EventName": "ChampionKill", "EventTime": 1.0 }}"#
        ))
        .unwrap()
    }

    #[test]
    fn first_poll_returns_everything_and_sets_watermark() {
        let mut t = EventTracker::default();
        let all = vec![ev(0), ev(1), ev(2)];
        assert_eq!(t.fresh(&all).len(), 3);
        assert_eq!(t.fresh(&all).len(), 0, "same payload again yields nothing");
    }

    #[test]
    fn later_polls_only_return_new_events() {
        let mut t = EventTracker::default();
        t.fresh(&[ev(0), ev(1)]);
        let payload = [ev(0), ev(1), ev(2), ev(3)];
        let fresh = t.fresh(&payload);
        let ids: Vec<u64> = fresh.iter().map(|e| e.event_id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn out_of_order_payload_is_sorted() {
        let mut t = EventTracker::default();
        let payload = [ev(2), ev(0), ev(1)];
        let fresh = t.fresh(&payload);
        let ids: Vec<u64> = fresh.iter().map(|e| e.event_id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn game_clock_regression_resets_the_event_watermark() {
        let mut t = EventTracker::default();
        let old_match = [ev(40), ev(41)];
        assert!(!t.prepare_poll(1_800.0, &old_match));
        t.fresh(&old_match);

        let new_match = [ev(0), ev(1)];
        assert!(t.prepare_poll(12.0, &new_match));
        let ids: Vec<u64> = t
            .fresh(&new_match)
            .into_iter()
            .map(|event| event.event_id)
            .collect();
        assert_eq!(ids, vec![0, 1]);
    }

    #[test]
    fn event_id_regression_resets_even_when_clock_does_not() {
        let mut t = EventTracker::default();
        let old_payload = [ev(90), ev(91)];
        assert!(!t.prepare_poll(100.0, &old_payload));
        t.fresh(&old_payload);

        let reset_payload = [ev(0), ev(1)];
        assert!(t.prepare_poll(101.0, &reset_payload));
        assert_eq!(t.fresh(&reset_payload).len(), 2);
    }

    #[test]
    fn small_clock_correction_does_not_reset_the_watermark() {
        let mut t = EventTracker::default();
        let payload = [ev(0), ev(1)];
        assert!(!t.prepare_poll(100.0, &payload));
        t.fresh(&payload);

        assert!(!t.prepare_poll(98.0, &payload));
        assert!(t.fresh(&payload).is_empty());
    }
}
