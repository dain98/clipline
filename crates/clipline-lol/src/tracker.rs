use crate::raw::RawEvent;

/// Watermark over Riot's monotonic `EventID` (ddoc §5a): each poll returns
/// the full event list; only events above the watermark are surfaced.
#[derive(Debug, Default)]
pub struct EventTracker {
    last_seen: Option<u64>,
}

impl EventTracker {
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
}
