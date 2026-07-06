//! Replays an anonymized live GSI capture (one round of casual on
//! de_inferno, 2026-07-05) through the tracker and asserts the whole story.
//! The capture includes the two hazards that shaped the design: joining
//! mid-round while the `player` node shows a spectated teammate, and the
//! final kill/death arriving in the same post that flips the map to
//! gameover.

use clipline_cs2::{GsiPayload, GsiTracker, GsiUpdate};
use clipline_events::EventKind;

fn fixture_updates() -> Vec<GsiUpdate> {
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/casual-round.jsonl"),
    )
    .expect("read fixture");
    let mut tracker = GsiTracker::new();
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .flat_map(|line| {
            let payload = GsiPayload::from_json(line.as_bytes()).expect("fixture line parses");
            tracker.ingest(&payload)
        })
        .collect()
}

fn kinds(updates: &[GsiUpdate]) -> Vec<(EventKind, Option<&str>)> {
    updates
        .iter()
        .filter_map(|u| match u {
            GsiUpdate::Event(e) => Some((e.kind, e.subtype.as_deref())),
            _ => None,
        })
        .collect()
}

#[test]
fn casual_round_capture_tells_the_expected_story() {
    let updates = fixture_updates();
    let events = kinds(&updates);

    // One session in the capture: menus, then the casual game joined
    // mid-match, then back to menus. Opens and closes exactly once — the
    // lingering gameover/menu posts must not produce extra boundaries.
    let starts = updates.iter().filter(|u| **u == GsiUpdate::MatchStarted).count();
    let ends = updates.iter().filter(|u| **u == GsiUpdate::MatchEnded).count();
    assert_eq!((starts, ends), (1, 1), "{updates:#?}");

    // The local player's real round: three kills, two of them headshots,
    // reaching a triple-kill, then dying as the round ends.
    let kills: Vec<_> = events.iter().filter(|(k, _)| *k == EventKind::PlayerKill).collect();
    assert_eq!(kills.len(), 3, "exactly the local player's kills: {events:?}");
    assert_eq!(
        kills.iter().filter(|(_, s)| *s == Some("headshot")).count(),
        2
    );
    assert_eq!(
        events.iter().filter(|(k, _)| *k == EventKind::Multikill).count(),
        1
    );
    assert!(events.contains(&(EventKind::Multikill, Some("3"))));
    assert_eq!(
        events.iter().filter(|(k, _)| *k == EventKind::PlayerDeath).count(),
        1,
        "the spectated teammate's stats must not produce deaths: {events:?}"
    );
    assert_eq!(
        events.iter().filter(|(k, _)| *k == EventKind::PlayerAssist).count(),
        1
    );

    // Bomb plants: one seen while spectating (round 9), one in the real
    // round (round 10). Both are timeline-worthy.
    assert_eq!(
        events.iter().filter(|(k, _)| *k == EventKind::BombPlanted).count(),
        2
    );

    // Round 9's win arrived before any self post (team unknown -> skipped).
    // Round 10: the local player is CT, T wins on the bomb — a loss, despite
    // the triple kill. win_team arrives only on the next freezetime post
    // (the "over" phase was skipped between posts), so this also proves the
    // value-edge trigger.
    assert_eq!(
        events.iter().filter(|(k, _)| *k == EventKind::RoundLost).count(),
        1
    );
    assert!(events.contains(&(EventKind::RoundLost, Some("T"))));
    assert!(!events.iter().any(|(k, _)| *k == EventKind::RoundWon));

    // The death and round result precede the final MatchEnded.
    let last = updates.last().expect("updates end with the menu post");
    assert_eq!(*last, GsiUpdate::MatchEnded);

    // Summaries only ever describe the local player.
    for u in &updates {
        if let GsiUpdate::Summary(s) = u {
            assert_eq!(s.player_name, "LocalPlayer", "spectated teammate leaked into summary");
        }
    }
    let final_summary = updates
        .iter()
        .rev()
        .find_map(|u| match u {
            GsiUpdate::Summary(s) => Some(s),
            _ => None,
        })
        .expect("at least one summary");
    assert_eq!(
        (final_summary.kills, final_summary.deaths, final_summary.assists),
        (3, 1, 1)
    );
    assert_eq!(final_summary.team, "CT");
}

#[test]
fn no_local_events_before_the_first_self_post() {
    // The mid-round join: everything up to the first self-identified player
    // post must produce zero player-stat events (the spectated teammate's
    // kills, deaths, and health swings all happen in that window).
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/casual-round.jsonl"),
    )
    .expect("read fixture");
    let mut tracker = GsiTracker::new();
    let mut seen_self = false;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let payload = GsiPayload::from_json(line.as_bytes()).expect("fixture line parses");
        let updates = tracker.ingest(&payload);
        if !seen_self {
            for u in &updates {
                if let GsiUpdate::Event(e) = u {
                    assert!(
                        !matches!(
                            e.kind,
                            EventKind::PlayerKill
                                | EventKind::PlayerDeath
                                | EventKind::PlayerAssist
                                | EventKind::Mvp
                                | EventKind::Multikill
                        ),
                        "player event {e:?} before any self post"
                    );
                }
            }
        }
        seen_self = seen_self || payload.local_player().is_some();
    }
    assert!(seen_self, "fixture should contain self posts");
}
