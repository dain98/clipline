//! Turns the GSI payload stream into normalized events and match boundaries.
//!
//! CS2 GSI is a state feed, not an event feed: kills, deaths, and MVPs are
//! inferred from stat deltas between consecutive snapshots of the *local*
//! player. The central hazard (observed live, 2026-07-05 capture) is that the
//! `player` node follows the camera — while dead or joining mid-round it
//! describes a spectated teammate. Deltas are therefore only computed between
//! consecutive self-posts, and a self→other identity switch mid-round is
//! itself the death signal (the camera only leaves you when you die).
//!
//! The tracker is clock-free: emitted events carry `recording_offset_s: None`
//! and the wiring layer stamps arrival time. GSI posts arrive ~0.4 s after
//! the fact (throttle/buffer), which is within marker tolerance.

use clipline_events::{EventKind, GameEvent, GameId, PlayerSummary};

use crate::payload::GsiPayload;

/// What one ingested payload produced, in emission order.
#[derive(Debug, Clone, PartialEq)]
pub enum GsiUpdate {
    MatchStarted,
    MatchEnded,
    Event(GameEvent),
    Summary(PlayerSummary),
}

#[derive(Debug, Clone, Default)]
struct SelfSnapshot {
    kills: u32,
    assists: u32,
    deaths: u32,
    mvps: u32,
    health: Option<i32>,
    round_kills: u32,
    round_kill_headshots: u32,
}

#[derive(Debug, Default)]
pub struct GsiTracker {
    in_match: bool,
    last_self: Option<SelfSnapshot>,
    prev_post_was_self: bool,
    local_name: String,
    local_team: Option<String>,
    last_round_phase: Option<String>,
    last_bomb: Option<String>,
    last_win_team: Option<String>,
    death_emitted_this_round: bool,
    last_summary: Option<PlayerSummary>,
}

impl GsiTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&mut self, payload: &GsiPayload) -> Vec<GsiUpdate> {
        let mut out = Vec::new();

        let map_phase = payload.map.as_ref().map(|m| m.phase.as_str());
        let match_running = matches!(map_phase, Some("warmup" | "live" | "intermission"));

        if match_running && !self.in_match {
            self.start_match();
            out.push(GsiUpdate::MatchStarted);
        }

        if self.in_match {
            self.track_round(payload, &mut out);
            self.track_local_player(payload, map_phase, &mut out);
        }

        // Boundary after events: the post that flips map.phase to "gameover"
        // can also carry the final kill or death (observed in capture).
        if self.in_match && !match_running {
            self.in_match = false;
            out.push(GsiUpdate::MatchEnded);
        }

        out
    }

    fn start_match(&mut self) {
        self.in_match = true;
        self.last_self = None;
        self.prev_post_was_self = false;
        self.local_team = None;
        self.last_round_phase = None;
        self.last_bomb = None;
        self.last_win_team = None;
        self.last_summary = None;
        self.reset_round();
    }

    fn reset_round(&mut self) {
        self.death_emitted_this_round = false;
    }

    fn track_round(&mut self, payload: &GsiPayload, out: &mut Vec<GsiUpdate>) {
        let Some(round) = payload.round.as_ref() else {
            return;
        };

        let phase = round.phase.as_str();
        if phase == "freezetime" && self.last_round_phase.as_deref() != Some("freezetime") {
            self.reset_round();
        }

        // Bomb and round-result are edge-triggered on the *value*, not the
        // round phase: GSI can skip the "over" phase entirely between posts,
        // delivering win_team only on the next freezetime post (observed in
        // capture), and stale values linger until the state moves on.
        let bomb = round.bomb.as_deref();
        if bomb != self.last_bomb.as_deref() {
            let kind = match bomb {
                Some("planted") => Some((EventKind::BombPlanted, 5)),
                Some("defused") => Some((EventKind::BombDefused, 7)),
                Some("exploded") => Some((EventKind::BombExploded, 6)),
                _ => None,
            };
            if let Some((kind, importance)) = kind {
                out.push(GsiUpdate::Event(self.event(kind, importance, None, false)));
            }
            self.last_bomb = bomb.map(str::to_string);
        }

        let winner = round.win_team.as_deref();
        if winner != self.last_win_team.as_deref() {
            if let Some(winner) = winner {
                // A result seen before the local team is known (mid-round
                // join) can't be attributed; it is consumed, not deferred.
                if let Some(team) = self.local_team.as_deref() {
                    let (kind, importance) = if team.eq_ignore_ascii_case(winner) {
                        (EventKind::RoundWon, 4)
                    } else {
                        (EventKind::RoundLost, 3)
                    };
                    out.push(GsiUpdate::Event(self.event(
                        kind,
                        importance,
                        Some(winner.to_string()),
                        true,
                    )));
                }
            }
            self.last_win_team = winner.map(str::to_string);
        }

        self.last_round_phase = Some(phase.to_string());
    }

    fn track_local_player(
        &mut self,
        payload: &GsiPayload,
        map_phase: Option<&str>,
        out: &mut Vec<GsiUpdate>,
    ) {
        // Kills in warmup are real stat changes but not review-worthy;
        // snapshots still update below so warmup stats never replay as
        // deltas once the match goes live.
        let emit_allowed = map_phase != Some("warmup");

        let Some(local) = payload.local_player() else {
            // Camera switched off the local player mid-round: the only way
            // that happens is death. Only infer it from a *player* post —
            // a missing player section proves nothing.
            if self.prev_post_was_self
                && payload.player.is_some()
                && emit_allowed
                && !self.death_emitted_this_round
                && self.last_self.as_ref().is_some_and(|s| s.health != Some(0))
            {
                self.death_emitted_this_round = true;
                out.push(GsiUpdate::Event(self.event(EventKind::PlayerDeath, 5, None, true)));
            }
            if payload.player.is_some() {
                self.prev_post_was_self = false;
            }
            return;
        };

        self.local_name = local.name.clone();
        if let Some(team) = local.team.as_deref() {
            if !team.is_empty() {
                self.local_team = Some(team.to_string());
            }
        }

        let state = local.state.as_ref();
        let stats = local.match_stats.as_ref();
        let current = SelfSnapshot {
            kills: stats.map_or(0, |s| s.kills),
            assists: stats.map_or(0, |s| s.assists),
            deaths: stats.map_or(0, |s| s.deaths),
            mvps: stats.map_or(0, |s| s.mvps),
            health: state.and_then(|s| s.health),
            round_kills: state.and_then(|s| s.round_kills).unwrap_or(0),
            round_kill_headshots: state.and_then(|s| s.round_kill_headshots).unwrap_or(0),
        };

        if let Some(prev) = self.last_self.clone() {
            if emit_allowed && stats.is_some() {
                let kills = current.kills.saturating_sub(prev.kills);
                // Headshot deltas only mean something within a round;
                // saturating_sub zeroes them across round resets.
                let headshots = current
                    .round_kill_headshots
                    .saturating_sub(prev.round_kill_headshots);
                for i in 0..kills {
                    let subtype = (i < headshots).then(|| "headshot".to_string());
                    out.push(GsiUpdate::Event(self.event(EventKind::PlayerKill, 6, subtype, true)));
                }

                if !self.death_emitted_this_round
                    && (current.deaths > prev.deaths
                        || (current.health == Some(0) && prev.health.is_some_and(|h| h > 0)))
                {
                    self.death_emitted_this_round = true;
                    out.push(GsiUpdate::Event(self.event(EventKind::PlayerDeath, 5, None, true)));
                }

                for _ in 0..current.assists.saturating_sub(prev.assists) {
                    out.push(GsiUpdate::Event(self.event(EventKind::PlayerAssist, 4, None, true)));
                }

                for _ in 0..current.mvps.saturating_sub(prev.mvps) {
                    out.push(GsiUpdate::Event(self.event(EventKind::Mvp, 7, None, true)));
                }

                // Edge-triggered on the value: round_kills lingers at its
                // final count into the next freezetime post, so a phase-based
                // flag would double-report (observed in capture at gameover).
                if current.round_kills >= 3 && prev.round_kills < 3 {
                    out.push(GsiUpdate::Event(self.event(
                        EventKind::Multikill,
                        7,
                        Some(current.round_kills.to_string()),
                        true,
                    )));
                }
            }
        }

        self.last_self = Some(current);
        self.prev_post_was_self = true;

        if let Some(stats) = stats {
            let summary = PlayerSummary {
                champion_name: String::new(),
                kills: stats.kills,
                deaths: stats.deaths,
                assists: stats.assists,
                creep_score: None,
                game_time_s: None,
                player_name: local.name.clone(),
                team: self.local_team.clone().unwrap_or_default(),
                participants: Vec::new(),
                summoner_spells: Vec::new(),
                items: Vec::new(),
            };
            if self.last_summary.as_ref() != Some(&summary) {
                self.last_summary = Some(summary.clone());
                out.push(GsiUpdate::Summary(summary));
            }
        }
    }

    fn event(
        &self,
        kind: EventKind,
        importance: u8,
        subtype: Option<String>,
        involves_local_player: bool,
    ) -> GameEvent {
        GameEvent {
            game_id: GameId::Cs2,
            kind,
            actor: if involves_local_player {
                self.local_name.clone()
            } else {
                String::new()
            },
            victim: None,
            assisters: Vec::new(),
            subtype,
            // GSI exposes no game clock in own play; events are anchored by
            // arrival time in the wiring layer instead.
            game_time_s: 0.0,
            recording_offset_s: None,
            importance,
            involves_local_player,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: &str = "76561190000000001";
    const OTHER: &str = "76561190000000002";

    fn post(json: &str) -> GsiPayload {
        GsiPayload::from_json(json.as_bytes()).expect("test payload parses")
    }

    fn self_post(kills: u32, deaths: u32, assists: u32, health: i32, round_kills: u32, hs: u32) -> GsiPayload {
        post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 5 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T", "activity": "playing",
                "state": {{ "health": {health}, "round_kills": {round_kills}, "round_killhs": {hs} }},
                "match_stats": {{ "kills": {kills}, "assists": {assists}, "deaths": {deaths}, "mvps": 0, "score": 0 }}
              }}
            }}"#
        ))
    }

    fn events(updates: &[GsiUpdate]) -> Vec<(EventKind, Option<String>)> {
        updates
            .iter()
            .filter_map(|u| match u {
                GsiUpdate::Event(e) => Some((e.kind, e.subtype.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn map_appearing_starts_match_and_gameover_ends_it_after_events() {
        let mut t = GsiTracker::new();

        let updates = t.ingest(&self_post(0, 0, 0, 100, 0, 0));
        assert_eq!(updates.first(), Some(&GsiUpdate::MatchStarted));

        // Final post: the killing blow lands in the same payload that flips
        // the map to gameover — the death must precede MatchEnded.
        let last = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "gameover", "round": 6 }},
              "round": {{ "phase": "over" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 0, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 0, "assists": 0, "deaths": 1, "mvps": 0, "score": 0 }}
              }}
            }}"#
        ));
        let updates = t.ingest(&last);
        let kinds: Vec<EventKind> = events(&updates).into_iter().map(|(k, _)| k).collect();
        assert!(kinds.contains(&EventKind::PlayerDeath));
        assert_eq!(updates.last(), Some(&GsiUpdate::MatchEnded));

        // Lingering gameover posts do not re-end or restart the match.
        assert!(t.ingest(&last).is_empty());
    }

    #[test]
    fn kill_deltas_emit_events_with_headshot_subtypes() {
        let mut t = GsiTracker::new();
        t.ingest(&self_post(0, 0, 0, 100, 0, 0));

        assert_eq!(
            events(&t.ingest(&self_post(1, 0, 0, 100, 1, 0))),
            vec![(EventKind::PlayerKill, None)]
        );
        assert_eq!(
            events(&t.ingest(&self_post(2, 0, 0, 100, 2, 1))),
            vec![(EventKind::PlayerKill, Some("headshot".into()))]
        );
        // Two kills in one post, one of them a headshot.
        assert_eq!(
            events(&t.ingest(&self_post(4, 0, 0, 100, 4, 2))),
            vec![
                (EventKind::PlayerKill, Some("headshot".into())),
                (EventKind::PlayerKill, None),
                (EventKind::Multikill, Some("4".into())),
            ]
        );
    }

    #[test]
    fn spectated_player_stats_never_become_local_events() {
        // Regression for the live capture: joining mid-round shows a
        // teammate in the player node, with their kills/deaths/health.
        let mut t = GsiTracker::new();

        let spectated = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 9 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{OTHER}", "name": "Teammate", "team": "T",
                "state": {{ "health": 97, "round_kills": 1, "round_killhs": 0 }},
                "match_stats": {{ "kills": 5, "assists": 2, "deaths": 1, "mvps": 1, "score": 12 }}
              }}
            }}"#
        ));
        let updates = t.ingest(&spectated);
        assert_eq!(updates, vec![GsiUpdate::MatchStarted], "no events, no summary");

        // Teammate's stats moving must stay silent too.
        let spectated_kill = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 9 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{OTHER}", "name": "Teammate", "team": "T",
                "state": {{ "health": 40, "round_kills": 2, "round_killhs": 1 }},
                "match_stats": {{ "kills": 6, "assists": 2, "deaths": 1, "mvps": 1, "score": 14 }}
              }}
            }}"#
        ));
        assert!(t.ingest(&spectated_kill).is_empty());

        // First self post sets the baseline silently — the teammate's totals
        // never contaminate the local delta.
        let updates = t.ingest(&self_post(0, 0, 0, 100, 0, 0));
        assert!(events(&updates).is_empty());
        assert!(matches!(updates.first(), Some(GsiUpdate::Summary(s)) if s.kills == 0));
    }

    #[test]
    fn camera_leaving_local_player_mid_round_is_a_death() {
        let mut t = GsiTracker::new();
        t.ingest(&self_post(3, 0, 0, 100, 0, 0));

        let now_spectating = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 5 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{OTHER}", "name": "Teammate", "team": "T",
                "state": {{ "health": 80, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 2, "assists": 0, "deaths": 0, "mvps": 0, "score": 4 }}
              }}
            }}"#
        ));
        assert_eq!(
            events(&t.ingest(&now_spectating)),
            vec![(EventKind::PlayerDeath, None)]
        );

        // The late deaths-delta on self-resume must not double-report it...
        let resumed = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 5 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 0, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 3, "assists": 0, "deaths": 1, "mvps": 0, "score": 6 }}
              }}
            }}"#
        ));
        assert!(events(&t.ingest(&resumed)).is_empty());

        // ...but a death in a later round reports normally again.
        let freezetime = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 6 }},
              "round": {{ "phase": "freezetime" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 100, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 3, "assists": 0, "deaths": 1, "mvps": 0, "score": 6 }}
              }}
            }}"#
        ));
        t.ingest(&freezetime);
        let died_again = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 6 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 0, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 3, "assists": 0, "deaths": 2, "mvps": 0, "score": 6 }}
              }}
            }}"#
        ));
        assert_eq!(
            events(&t.ingest(&died_again)),
            vec![(EventKind::PlayerDeath, None)]
        );
    }

    #[test]
    fn bomb_and_round_result_events_come_from_round_state() {
        let mut t = GsiTracker::new();
        t.ingest(&self_post(0, 0, 0, 100, 0, 0));

        let planted = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 5 }},
              "round": {{ "phase": "live", "bomb": "planted" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 100, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 0, "assists": 0, "deaths": 0, "mvps": 0, "score": 0 }}
              }}
            }}"#
        ));
        assert_eq!(events(&t.ingest(&planted)), vec![(EventKind::BombPlanted, None)]);
        // Unchanged bomb state does not re-fire.
        assert!(events(&t.ingest(&planted)).is_empty());

        let round_over = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_inferno", "phase": "live", "round": 6 }},
              "round": {{ "phase": "over", "bomb": "exploded", "win_team": "T" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "T",
                "state": {{ "health": 100, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 0, "assists": 0, "deaths": 0, "mvps": 0, "score": 0 }}
              }}
            }}"#
        ));
        let got = events(&t.ingest(&round_over));
        assert_eq!(
            got,
            vec![
                (EventKind::BombExploded, None),
                (EventKind::RoundWon, Some("T".into())),
            ]
        );
    }

    #[test]
    fn warmup_stats_update_baseline_without_emitting() {
        let mut t = GsiTracker::new();
        let warmup = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_dust2", "phase": "warmup", "round": 0 }},
              "round": {{ "phase": "live" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "CT",
                "state": {{ "health": 100, "round_kills": 4, "round_killhs": 2 }},
                "match_stats": {{ "kills": 4, "assists": 0, "deaths": 2, "mvps": 0, "score": 8 }}
              }}
            }}"#
        ));
        let updates = t.ingest(&warmup);
        assert!(events(&updates).is_empty(), "warmup kills are not markers");

        // Going live must not replay warmup totals as fresh deltas.
        let live = post(&format!(
            r#"{{
              "provider": {{ "steamid": "{LOCAL}" }},
              "map": {{ "name": "de_dust2", "phase": "live", "round": 1 }},
              "round": {{ "phase": "freezetime" }},
              "player": {{
                "steamid": "{LOCAL}", "name": "LocalPlayer", "team": "CT",
                "state": {{ "health": 100, "round_kills": 0, "round_killhs": 0 }},
                "match_stats": {{ "kills": 4, "assists": 0, "deaths": 2, "mvps": 0, "score": 8 }}
              }}
            }}"#
        ));
        assert!(events(&t.ingest(&live)).is_empty());
    }

    #[test]
    fn summary_tracks_match_stats_and_dedupes() {
        let mut t = GsiTracker::new();
        let first = t.ingest(&self_post(0, 0, 0, 100, 0, 0));
        assert!(first.iter().any(|u| matches!(u, GsiUpdate::Summary(s) if s.player_name == "LocalPlayer" && s.team == "T")));

        // Same stats → no repeat summary.
        assert!(!t
            .ingest(&self_post(0, 0, 0, 90, 0, 0))
            .iter()
            .any(|u| matches!(u, GsiUpdate::Summary(_))));

        let after_kill = t.ingest(&self_post(1, 0, 0, 90, 1, 0));
        assert!(after_kill
            .iter()
            .any(|u| matches!(u, GsiUpdate::Summary(s) if s.kills == 1)));
    }
}
