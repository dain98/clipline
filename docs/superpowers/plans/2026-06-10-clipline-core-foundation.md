# Clipline Core Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the platform-neutral core of Clipline (per `ddoc.md`): the normalized game-event schema, timeline synchronization math, the League of Legends Live Client Data adapter, and the encoded-segment replay ring buffer — fully tested on Linux.

**Architecture:** A Cargo workspace with three library crates. `clipline-events` defines the normalized event schema (ddoc §5) and the game-clock→recording-timeline anchor math (ddoc §5, "Timeline synchronization"). `clipline-lol` implements the League adapter (ddoc §5a): Riot payload parsing, monotonic-EventID dedupe, normalization with local-player tagging, and an HTTP client + poller. `clipline-buffer` implements the replay buffer core (ddoc §6): GOP-aligned encoded segments in a byte-budgeted ring, keyframe-aligned save-window extraction with the "don't re-clip overlapping footage" smart mode, and the RAM estimator. Windows-specific capture/encode/audio, the Hybrid MP4 muxer, and the Tauri shell are separate follow-up plans (see end).

**Tech Stack:** Rust (stable, edition 2021), serde/serde_json, thiserror, reqwest (rustls, `danger_accept_invalid_certs` for Riot's self-signed localhost cert), tokio, httpmock (dev-only).

**Environment notes:**
- Dev machine is Linux; everything in this plan is platform-neutral and must compile and pass tests on Linux.
- Rust is installed via rustup to `~/.cargo/bin`. If `cargo` is not on PATH in a step, use `source "$HOME/.cargo/env"` first or invoke `"$HOME/.cargo/bin/cargo"`.
- All commits end with the trailer: `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `.gitignore`
- Create: `crates/clipline-events/Cargo.toml`, `crates/clipline-events/src/lib.rs`
- Create: `crates/clipline-lol/Cargo.toml`, `crates/clipline-lol/src/lib.rs`
- Create: `crates/clipline-buffer/Cargo.toml`, `crates/clipline-buffer/src/lib.rs`

- [ ] **Step 1: Initialize git** (the directory is not yet a repo)

```bash
cd /home/dain/clipline && git init -b main
```

- [ ] **Step 2: Write `.gitignore`**

```gitignore
/target
**/*.rs.bk
```

- [ ] **Step 3: Write workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
    "crates/clipline-events",
    "crates/clipline-lol",
    "crates/clipline-buffer",
]

[workspace.package]
edition = "2021"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
httpmock = "0.7"
```

- [ ] **Step 4: Write crate manifests and empty libs**

`crates/clipline-events/Cargo.toml`:
```toml
[package]
name = "clipline-events"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
serde = { workspace = true }

[dev-dependencies]
serde_json = { workspace = true }
```

`crates/clipline-lol/Cargo.toml`:
```toml
[package]
name = "clipline-lol"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
clipline-events = { path = "../clipline-events" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
reqwest = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
httpmock = { workspace = true }
```

`crates/clipline-buffer/Cargo.toml`:
```toml
[package]
name = "clipline-buffer"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
```

Each `src/lib.rs` starts as an empty file (modules added by later tasks).

- [ ] **Step 5: Verify the workspace builds**

Run: `cargo check --workspace`
Expected: `Finished` with no errors.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
chore: scaffold cargo workspace (events, lol, buffer crates)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Normalized event schema (`clipline-events`)

Implements the internal event schema from ddoc §5.

**Files:**
- Create: `crates/clipline-events/src/schema.rs`
- Modify: `crates/clipline-events/src/lib.rs`
- Test: inline `#[cfg(test)]` in `schema.rs`

- [ ] **Step 1: Write the failing test** (in `schema.rs`, plus module wiring in `lib.rs`)

`crates/clipline-events/src/lib.rs`:
```rust
pub mod schema;

pub use schema::{EventKind, GameEvent, GameId};
```

`crates/clipline-events/src/schema.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_event_serde_roundtrip() {
        let ev = GameEvent {
            game_id: GameId::LeagueOfLegends,
            kind: EventKind::ChampionKill,
            actor: "Killer".into(),
            victim: Some("Victim".into()),
            assisters: vec!["Helper".into()],
            subtype: None,
            game_time_s: 312.5,
            recording_offset_s: Some(95.25),
            importance: 7,
            involves_local_player: true,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: GameEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-events`
Expected: COMPILE ERROR (`GameEvent` not defined).

- [ ] **Step 3: Write the implementation** (top of `schema.rs`, above the tests)

```rust
use serde::{Deserialize, Serialize};

/// Which game produced an event (ddoc §5 normalized schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameId {
    LeagueOfLegends,
    Valorant,
    Cs2,
}

/// Normalized event kind across all game adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    GameStart,
    MinionsSpawning,
    FirstBrick,
    TurretKilled,
    InhibKilled,
    DragonKill,
    HeraldKill,
    BaronKill,
    ChampionKill,
    Multikill,
    Ace,
    // Community-observed, not in Riot's official sample (ddoc §5a) —
    // parsed defensively, never relied upon.
    FirstBlood,
    GameEnd,
    Other,
}

/// One normalized timeline event (ddoc §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameEvent {
    pub game_id: GameId,
    pub kind: EventKind,
    /// Killer / acer / local actor. Empty string when the source has none.
    pub actor: String,
    pub victim: Option<String>,
    pub assisters: Vec<String>,
    /// DragonType, kill-streak count, turret id, acing team, game result, …
    pub subtype: Option<String>,
    /// Seconds of game clock, as reported by the source.
    pub game_time_s: f64,
    /// Position on the recording timeline; None until anchored (ddoc §5).
    pub recording_offset_s: Option<f64>,
    /// 0–10, drives auto-clip thresholds later.
    pub importance: u8,
    pub involves_local_player: bool,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-events`
Expected: `test schema::tests::game_event_serde_roundtrip ... ok`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(events): normalized game-event schema

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Timeline synchronization math (`clipline-events`)

Implements ddoc §5 "Timeline synchronization": anchor a (game-clock, wall-clock) sample pair each poll; map `EventTime` onto the recording timeline; re-sampling each poll makes game-clock pauses self-correct.

**Files:**
- Create: `crates/clipline-events/src/sync.rs`
- Modify: `crates/clipline-events/src/lib.rs`
- Test: inline `#[cfg(test)]` in `sync.rs`

- [ ] **Step 1: Write the failing tests**

Add to `crates/clipline-events/src/lib.rs`:
```rust
pub mod sync;

pub use sync::{recording_offset_s, ClockAnchor};
```

`crates/clipline-events/src/sync.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn maps_event_time_onto_recording_timeline() {
        // Recording starts at t0. 100s later we poll: game clock reads 95s
        // (game started 5s after the recording). An event at EventTime=90
        // therefore happened at recording offset 95s.
        let t0 = Instant::now();
        let anchor = ClockAnchor {
            game_time_s: 95.0,
            sampled_at: t0 + Duration::from_secs(100),
        };
        let off = recording_offset_s(90.0, anchor, t0, 0.0);
        assert!((off - 95.0).abs() < 1e-9);
    }

    #[test]
    fn resampled_anchor_self_corrects_after_game_clock_pause() {
        // Game pauses for 60s at game_time=200. After the pause, wall time
        // has advanced 60s more than game time. A fresh anchor sampled at
        // wall t0+360 reads game_time=300; an event at EventTime=310 lands
        // at recording offset 370 — pause absorbed with no special-casing.
        let t0 = Instant::now();
        let post_pause_anchor = ClockAnchor {
            game_time_s: 300.0,
            sampled_at: t0 + Duration::from_secs(360),
        };
        let off = recording_offset_s(310.0, post_pause_anchor, t0, 0.0);
        assert!((off - 370.0).abs() < 1e-9);
    }

    #[test]
    fn emit_latency_nudges_marker_earlier() {
        let t0 = Instant::now();
        let anchor = ClockAnchor {
            game_time_s: 10.0,
            sampled_at: t0 + Duration::from_secs(10),
        };
        let off = recording_offset_s(10.0, anchor, t0, 0.5);
        assert!((off - 9.5).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-events`
Expected: COMPILE ERROR (`ClockAnchor` not defined).

- [ ] **Step 3: Write the implementation** (top of `sync.rs`)

```rust
use std::time::Instant;

/// A paired sample of the source game clock and the wall clock.
///
/// Re-sample one of these on every poll (ddoc §5): pauses and drift in the
/// game clock then self-correct, and already-placed markers are never
/// re-mapped. `sampled_at` must not be earlier than the recording's `t0`.
#[derive(Debug, Clone, Copy)]
pub struct ClockAnchor {
    /// Game clock (seconds since GameStart) at the moment of sampling.
    pub game_time_s: f64,
    /// Wall clock at the moment of sampling.
    pub sampled_at: Instant,
}

/// Map a source event time onto the recording timeline (ddoc §5):
/// `offset = (EventTime − anchor.gameTime) + (anchor.wall − t0) − latency`.
///
/// `emit_latency_s` is the small fixed kill-feed/event-emit delay that
/// nudges markers onto the visual moment. The result can be negative for
/// events that predate the recording; callers clamp as appropriate.
pub fn recording_offset_s(
    event_time_s: f64,
    anchor: ClockAnchor,
    recording_t0: Instant,
    emit_latency_s: f64,
) -> f64 {
    (event_time_s - anchor.game_time_s)
        + anchor.sampled_at.duration_since(recording_t0).as_secs_f64()
        - emit_latency_s
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-events`
Expected: all 4 tests pass (3 sync + 1 schema).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(events): game-clock to recording-timeline anchor math

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Riot Live Client payload parsing (`clipline-lol`)

Parses `GET /liveclientdata/eventdata` payloads. Field names per Riot's `liveclientdata_events.json` (ddoc §5a table). Note `Stolen` is the *string* `"False"`/`"True"` in Riot's sample, not a bool.

**Files:**
- Create: `crates/clipline-lol/src/raw.rs`
- Modify: `crates/clipline-lol/src/lib.rs`
- Test: inline `#[cfg(test)]` in `raw.rs`

- [ ] **Step 1: Write the failing test**

`crates/clipline-lol/src/lib.rs`:
```rust
pub mod raw;

pub use raw::{EventData, RawEvent};
```

`crates/clipline-lol/src/raw.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Shaped like Riot's official liveclientdata_events.json sample.
    const FIXTURE: &str = r#"{
      "Events": [
        { "EventID": 0, "EventName": "GameStart", "EventTime": 0.05 },
        { "EventID": 3, "EventName": "ChampionKill", "EventTime": 300.2,
          "VictimName": "Shaco", "KillerName": "Teemo", "Assisters": ["Lux"] },
        { "EventID": 4, "EventName": "Multikill", "EventTime": 305.0,
          "KillerName": "Teemo", "KillStreak": 2 },
        { "EventID": 5, "EventName": "DragonKill", "EventTime": 1000.4,
          "DragonType": "Earth", "Stolen": "False",
          "KillerName": "Teemo", "Assisters": [] }
      ]
    }"#;

    #[test]
    fn parses_riot_sample_shaped_payload() {
        let data: EventData = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(data.events.len(), 4);

        let kill = &data.events[1];
        assert_eq!(kill.event_id, 3);
        assert_eq!(kill.event_name, "ChampionKill");
        assert_eq!(kill.killer_name.as_deref(), Some("Teemo"));
        assert_eq!(kill.victim_name.as_deref(), Some("Shaco"));
        assert_eq!(kill.assisters, vec!["Lux".to_string()]);

        let multi = &data.events[2];
        assert_eq!(multi.kill_streak, Some(2));
        assert!(multi.assisters.is_empty(), "missing Assisters defaults to empty");

        let dragon = &data.events[3];
        assert_eq!(dragon.dragon_type.as_deref(), Some("Earth"));
        assert_eq!(dragon.stolen.as_deref(), Some("False"));
    }

    #[test]
    fn unknown_event_names_still_parse() {
        let json = r#"{ "Events": [
          { "EventID": 9, "EventName": "SomeFutureThing", "EventTime": 12.0 }
        ] }"#;
        let data: EventData = serde_json::from_str(json).unwrap();
        assert_eq!(data.events[0].event_name, "SomeFutureThing");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-lol`
Expected: COMPILE ERROR (`EventData` not defined).

- [ ] **Step 3: Write the implementation** (top of `raw.rs`)

```rust
use serde::Deserialize;

/// Response body of `GET /liveclientdata/eventdata`.
#[derive(Debug, Clone, Deserialize)]
pub struct EventData {
    #[serde(rename = "Events")]
    pub events: Vec<RawEvent>,
}

/// One raw Live Client Data event. Field set is the union of all event
/// types in Riot's sample (ddoc §5a) — absent fields deserialize to None.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEvent {
    #[serde(rename = "EventID")]
    pub event_id: u64,
    #[serde(rename = "EventName")]
    pub event_name: String,
    /// Seconds of game time.
    #[serde(rename = "EventTime")]
    pub event_time: f64,
    #[serde(rename = "KillerName")]
    pub killer_name: Option<String>,
    #[serde(rename = "VictimName")]
    pub victim_name: Option<String>,
    #[serde(rename = "Assisters", default)]
    pub assisters: Vec<String>,
    #[serde(rename = "KillStreak")]
    pub kill_streak: Option<u32>,
    #[serde(rename = "DragonType")]
    pub dragon_type: Option<String>,
    /// String "True"/"False" in Riot's sample, not a bool.
    #[serde(rename = "Stolen")]
    pub stolen: Option<String>,
    #[serde(rename = "TurretKilled")]
    pub turret_killed: Option<String>,
    #[serde(rename = "InhibKilled")]
    pub inhib_killed: Option<String>,
    #[serde(rename = "Acer")]
    pub acer: Option<String>,
    #[serde(rename = "AcingTeam")]
    pub acing_team: Option<String>,
    // Community-observed fields (FirstBlood / GameEnd) — defensive only.
    #[serde(rename = "Recipient")]
    pub recipient: Option<String>,
    #[serde(rename = "Result")]
    pub result: Option<String>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-lol`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(lol): parse Live Client Data eventdata payloads

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Monotonic EventID dedupe (`clipline-lol`)

`eventdata` returns the full event list each poll; `EventID` is monotonic, so we keep a watermark and only surface new events (ddoc §5a).

**Files:**
- Create: `crates/clipline-lol/src/tracker.rs`
- Modify: `crates/clipline-lol/src/lib.rs`
- Test: inline `#[cfg(test)]` in `tracker.rs`

- [ ] **Step 1: Write the failing tests**

Add to `crates/clipline-lol/src/lib.rs`:
```rust
pub mod tracker;

pub use tracker::EventTracker;
```

`crates/clipline-lol/src/tracker.rs`:
```rust
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
        let fresh = t.fresh(&[ev(0), ev(1), ev(2), ev(3)]);
        let ids: Vec<u64> = fresh.iter().map(|e| e.event_id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn out_of_order_payload_is_sorted() {
        let mut t = EventTracker::default();
        let fresh = t.fresh(&[ev(2), ev(0), ev(1)]);
        let ids: Vec<u64> = fresh.iter().map(|e| e.event_id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-lol`
Expected: COMPILE ERROR (`EventTracker` not defined).

- [ ] **Step 3: Write the implementation** (top of `tracker.rs`)

```rust
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
```

(If the installed Rust is older than 1.82 and `Option::is_none_or` is unavailable, use `.map_or(true, |seen| e.event_id > seen)` instead.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-lol`
Expected: 5 tests pass (2 raw + 3 tracker).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(lol): monotonic EventID dedupe tracker

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Normalization with local-player tagging (`clipline-lol`)

Maps `RawEvent` → `GameEvent` (ddoc §5/§5a): kind mapping, subtype extraction, importance heuristic, and marking the local player's kills/deaths/assists distinctly.

**Files:**
- Create: `crates/clipline-lol/src/normalize.rs`
- Modify: `crates/clipline-lol/src/lib.rs`
- Test: inline `#[cfg(test)]` in `normalize.rs`

- [ ] **Step 1: Write the failing tests**

Add to `crates/clipline-lol/src/lib.rs`:
```rust
pub mod normalize;

pub use normalize::normalize;
```

`crates/clipline-lol/src/normalize.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawEvent;
    use clipline_events::EventKind;

    fn raw(json: &str) -> RawEvent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn champion_kill_by_local_player_is_boosted() {
        let r = raw(
            r#"{ "EventID": 1, "EventName": "ChampionKill", "EventTime": 300.0,
                 "KillerName": "Me", "VictimName": "Them", "Assisters": [] }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::ChampionKill);
        assert!(ev.involves_local_player);
        assert_eq!(ev.actor, "Me");
        assert_eq!(ev.victim.as_deref(), Some("Them"));
        assert_eq!(ev.importance, 7); // base 5 + local boost 2

        let bystander = normalize(&r, "SomeoneElse");
        assert!(!bystander.involves_local_player);
        assert_eq!(bystander.importance, 5);
    }

    #[test]
    fn local_player_as_victim_or_assister_counts_as_involved() {
        let r = raw(
            r#"{ "EventID": 2, "EventName": "ChampionKill", "EventTime": 10.0,
                 "KillerName": "A", "VictimName": "Me", "Assisters": ["B"] }"#,
        );
        assert!(normalize(&r, "Me").involves_local_player);
        assert!(normalize(&r, "B").involves_local_player);
    }

    #[test]
    fn dragon_kill_carries_type_subtype() {
        let r = raw(
            r#"{ "EventID": 3, "EventName": "DragonKill", "EventTime": 1000.0,
                 "DragonType": "Hextech", "Stolen": "False", "KillerName": "A",
                 "Assisters": [] }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::DragonKill);
        assert_eq!(ev.subtype.as_deref(), Some("Hextech"));
    }

    #[test]
    fn multikill_carries_streak_subtype() {
        let r = raw(
            r#"{ "EventID": 4, "EventName": "Multikill", "EventTime": 305.0,
                 "KillerName": "Me", "KillStreak": 3 }"#,
        );
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::Multikill);
        assert_eq!(ev.subtype.as_deref(), Some("3"));
    }

    #[test]
    fn unknown_event_maps_to_other_not_an_error() {
        let r = raw(r#"{ "EventID": 5, "EventName": "Brand New", "EventTime": 1.0 }"#);
        let ev = normalize(&r, "Me");
        assert_eq!(ev.kind, EventKind::Other);
        assert_eq!(ev.game_time_s, 1.0);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-lol`
Expected: COMPILE ERROR (`normalize` not defined).

- [ ] **Step 3: Write the implementation** (top of `normalize.rs`)

```rust
use crate::raw::RawEvent;
use clipline_events::{EventKind, GameEvent, GameId};

/// Importance added when the local player is killer, victim, or assister.
const LOCAL_PLAYER_BOOST: u8 = 2;

/// Normalize one raw Live Client event (ddoc §5a). `recording_offset_s` is
/// left as None; the poller anchors it (Task 8).
pub fn normalize(raw: &RawEvent, local_player: &str) -> GameEvent {
    let kind = match raw.event_name.as_str() {
        "GameStart" => EventKind::GameStart,
        "MinionsSpawning" => EventKind::MinionsSpawning,
        "FirstBrick" => EventKind::FirstBrick,
        "TurretKilled" => EventKind::TurretKilled,
        "InhibKilled" => EventKind::InhibKilled,
        "DragonKill" => EventKind::DragonKill,
        "HeraldKill" => EventKind::HeraldKill,
        "BaronKill" => EventKind::BaronKill,
        "ChampionKill" => EventKind::ChampionKill,
        "Multikill" => EventKind::Multikill,
        "Ace" => EventKind::Ace,
        "FirstBlood" => EventKind::FirstBlood,
        "GameEnd" => EventKind::GameEnd,
        _ => EventKind::Other,
    };

    let actor = raw
        .killer_name
        .clone()
        .or_else(|| raw.acer.clone())
        .or_else(|| raw.recipient.clone())
        .unwrap_or_default();

    let involves_local_player = !local_player.is_empty()
        && (actor == local_player
            || raw.victim_name.as_deref() == Some(local_player)
            || raw.assisters.iter().any(|a| a == local_player));

    let subtype = match kind {
        EventKind::DragonKill => raw.dragon_type.clone(),
        EventKind::Multikill => raw.kill_streak.map(|k| k.to_string()),
        EventKind::TurretKilled => raw.turret_killed.clone(),
        EventKind::InhibKilled => raw.inhib_killed.clone(),
        EventKind::Ace => raw.acing_team.clone(),
        EventKind::GameEnd => raw.result.clone(),
        _ => None,
    };

    let importance = (base_importance(kind)
        + if involves_local_player { LOCAL_PLAYER_BOOST } else { 0 })
    .min(10);

    GameEvent {
        game_id: GameId::LeagueOfLegends,
        kind,
        actor,
        victim: raw.victim_name.clone(),
        assisters: raw.assisters.clone(),
        subtype,
        game_time_s: raw.event_time,
        recording_offset_s: None,
        importance,
        involves_local_player,
    }
}

fn base_importance(kind: EventKind) -> u8 {
    match kind {
        EventKind::Ace => 8,
        EventKind::Multikill | EventKind::BaronKill => 7,
        EventKind::DragonKill | EventKind::FirstBlood => 6,
        EventKind::ChampionKill | EventKind::InhibKilled | EventKind::HeraldKill => 5,
        EventKind::TurretKilled | EventKind::FirstBrick => 4,
        EventKind::GameEnd => 3,
        EventKind::GameStart | EventKind::MinionsSpawning | EventKind::Other => 1,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-lol`
Expected: 10 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(lol): normalize raw events with local-player tagging

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Live Client HTTP client (`clipline-lol`)

HTTP client for `https://127.0.0.1:2999` (self-signed cert → accept invalid certs, localhost-only by construction; ddoc §5a). Base URL is injectable so tests use a plain-HTTP mock.

**Files:**
- Create: `crates/clipline-lol/src/client.rs`
- Modify: `crates/clipline-lol/src/lib.rs`
- Test: `crates/clipline-lol/tests/client_http.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/clipline-lol/src/lib.rs`:
```rust
pub mod client;

pub use client::{Error, LiveClient};
```

`crates/clipline-lol/tests/client_http.rs`:
```rust
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

    let client = LiveClient::new(server.base_url()).unwrap();
    let data = client.event_data().await.unwrap();
    assert_eq!(data.events.len(), 1);
    assert_eq!(client.active_player_name().await.unwrap(), "Me#NA1");
    assert!((client.game_time_s().await.unwrap() - 123.5).abs() < 1e-9);
}

#[tokio::test]
async fn connection_refused_is_an_error_not_a_panic() {
    // Nothing listens on this port.
    let client = LiveClient::new("http://127.0.0.1:9").unwrap();
    assert!(client.event_data().await.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-lol --test client_http`
Expected: COMPILE ERROR (`LiveClient` not defined).

- [ ] **Step 3: Write the implementation** (`client.rs`)

```rust
use std::time::Duration;

use serde::Deserialize;

use crate::raw::EventData;

/// Riot's local Live Client Data endpoint (ddoc §5a).
pub const DEFAULT_BASE: &str = "https://127.0.0.1:2999";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("live client request failed: {0}")]
    Http(#[from] reqwest::Error),
}

/// Client for the League Live Client Data API. The real endpoint serves a
/// self-signed Riot cert over loopback, so certificate validation is
/// disabled — the client is only ever pointed at 127.0.0.1 (or a test mock).
pub struct LiveClient {
    base: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct GameStats {
    #[serde(rename = "gameTime")]
    game_time: f64,
}

impl LiveClient {
    pub fn new(base: impl Into<String>) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()?;
        Ok(Self { base: base.into(), http })
    }

    /// Client against the real local game endpoint.
    pub fn default_local() -> Result<Self, Error> {
        Self::new(DEFAULT_BASE)
    }

    pub async fn event_data(&self) -> Result<EventData, Error> {
        Ok(self.get_json("/liveclientdata/eventdata").await?)
    }

    /// Riot returns the active player's name as a bare JSON string.
    pub async fn active_player_name(&self) -> Result<String, Error> {
        Ok(self.get_json("/liveclientdata/activeplayername").await?)
    }

    /// Current game clock in seconds, from `gamestats.gameTime` (ddoc §5).
    pub async fn game_time_s(&self) -> Result<f64, Error> {
        let stats: GameStats = self.get_json("/liveclientdata/gamestats").await?;
        Ok(stats.game_time)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, reqwest::Error> {
        self.http
            .get(format!("{}{}", self.base, path))
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-lol`
Expected: all unit tests + 2 integration tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(lol): HTTP client for the Live Client Data API

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Poll-once pipeline (`clipline-lol`)

One poll = sample game clock (fresh anchor, per ddoc §5), fetch events, dedupe, normalize, stamp recording offsets.

**Files:**
- Create: `crates/clipline-lol/src/poller.rs`
- Modify: `crates/clipline-lol/src/lib.rs`
- Test: `crates/clipline-lol/tests/poll_once.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/clipline-lol/src/lib.rs`:
```rust
pub mod poller;

pub use poller::poll_once;
```

`crates/clipline-lol/tests/poll_once.rs`:
```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-lol --test poll_once`
Expected: COMPILE ERROR (`poll_once` not defined).

- [ ] **Step 3: Write the implementation** (`poller.rs`)

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-lol`
Expected: all tests pass.

Note: the offset assertion uses a half-second tolerance window because the
anchor is sampled with a real `Instant::now()` between two mock HTTP calls;
if this flakes under extreme load, widen the window, don't sleep.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(lol): poll-once pipeline with anchored recording offsets

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Replay segments and byte-budgeted ring (`clipline-buffer`)

Encoded, GOP-aligned segments in a ring sized by a byte budget; oldest segments evicted first (ddoc §6).

**Files:**
- Create: `crates/clipline-buffer/src/segment.rs`, `crates/clipline-buffer/src/ring.rs`
- Modify: `crates/clipline-buffer/src/lib.rs`
- Test: inline `#[cfg(test)]` in `ring.rs`

- [ ] **Step 1: Write the failing tests**

`crates/clipline-buffer/src/lib.rs`:
```rust
pub mod ring;
pub mod segment;

pub use ring::ReplayRing;
pub use segment::Segment;
```

`crates/clipline-buffer/src/segment.rs`:
```rust
/// One encoded, GOP-aligned media segment (ddoc §6). `data` is opaque
/// encoded bytes (video+audio interleaved by the encode pipeline).
#[derive(Debug, Clone)]
pub struct Segment {
    /// True when the segment begins with a keyframe (IDR). Saved clips must
    /// start at such a segment so they decode cleanly.
    pub starts_with_keyframe: bool,
    /// Presentation start, seconds since recording t0.
    pub pts_start_s: f64,
    pub duration_s: f64,
    pub data: Vec<u8>,
}

impl Segment {
    pub fn pts_end_s(&self) -> f64 {
        self.pts_start_s + self.duration_s
    }
}
```

`crates/clipline-buffer/src/ring.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::Segment;

    fn seg(pts: f64, dur: f64, bytes: usize, key: bool) -> Segment {
        Segment {
            starts_with_keyframe: key,
            pts_start_s: pts,
            duration_s: dur,
            data: vec![0u8; bytes],
        }
    }

    #[test]
    fn evicts_oldest_when_over_byte_budget() {
        let mut ring = ReplayRing::new(250);
        ring.push(seg(0.0, 2.0, 100, true));
        ring.push(seg(2.0, 2.0, 100, true));
        ring.push(seg(4.0, 2.0, 100, true)); // 300 bytes > 250 → evict front
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.bytes(), 200);
        assert_eq!(ring.segments().next().unwrap().pts_start_s, 2.0);
    }

    #[test]
    fn never_evicts_the_only_segment() {
        let mut ring = ReplayRing::new(10);
        ring.push(seg(0.0, 2.0, 100, true)); // oversized but alone
        assert_eq!(ring.len(), 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-buffer`
Expected: COMPILE ERROR (`ReplayRing` not defined).

- [ ] **Step 3: Write the implementation** (top of `ring.rs`)

```rust
use std::collections::VecDeque;

use crate::segment::Segment;

/// Byte-budgeted ring of encoded segments (ddoc §6). Eviction is
/// oldest-first and whole-segment; segments are GOP-aligned so dropping
/// from the front never strands a partial GOP.
#[derive(Debug)]
pub struct ReplayRing {
    max_bytes: usize,
    segments: VecDeque<Segment>,
    bytes: usize,
}

impl ReplayRing {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes, segments: VecDeque::new(), bytes: 0 }
    }

    pub fn push(&mut self, seg: Segment) {
        self.bytes += seg.data.len();
        self.segments.push_back(seg);
        while self.bytes > self.max_bytes && self.segments.len() > 1 {
            if let Some(front) = self.segments.pop_front() {
                self.bytes -= front.data.len();
            }
        }
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-buffer`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(buffer): byte-budgeted replay ring of GOP-aligned segments

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: Save-window extraction with smart no-overlap mode (`clipline-buffer`)

On Save Replay: flush from the keyframe covering `end − window` to now (ddoc §6). Smart mode never re-saves footage before the last save point — the feature OBS lacks natively.

**Files:**
- Modify: `crates/clipline-buffer/src/ring.rs`
- Test: extend `#[cfg(test)]` in `ring.rs`

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`)

```rust
    #[test]
    fn save_window_starts_at_covering_keyframe() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, true));
        ring.push(seg(4.0, 2.0, 10, true));
        // Window of 3s from end (6.0) → target 3.0, covered by seg@2.0.
        let saved = ring.save_window(3.0, None);
        let starts: Vec<f64> = saved.iter().map(|s| s.pts_start_s).collect();
        assert_eq!(starts, vec![2.0, 4.0]);
    }

    #[test]
    fn save_window_skips_non_keyframe_lead_in() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, false)); // continuation of GOP at 0.0
        ring.push(seg(4.0, 2.0, 10, true));
        // Target 3.0: latest keyframe at/before is 0.0 → include from 0.0
        // so the clip covers the full window and starts decodable.
        let saved = ring.save_window(3.0, None);
        assert_eq!(saved[0].pts_start_s, 0.0);
        assert_eq!(saved.len(), 3);
    }

    #[test]
    fn smart_mode_never_resaves_already_saved_footage() {
        let mut ring = ReplayRing::new(usize::MAX);
        ring.push(seg(0.0, 2.0, 10, true));
        ring.push(seg(2.0, 2.0, 10, true));
        ring.push(seg(4.0, 2.0, 10, true));
        // Previous save consumed up to t=4.0 → only the last segment now.
        let saved = ring.save_window(6.0, Some(4.0));
        let starts: Vec<f64> = saved.iter().map(|s| s.pts_start_s).collect();
        assert_eq!(starts, vec![4.0]);
    }

    #[test]
    fn save_window_on_empty_ring_is_empty() {
        let ring = ReplayRing::new(100);
        assert!(ring.save_window(5.0, None).is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p clipline-buffer`
Expected: COMPILE ERROR (`save_window` not defined).

- [ ] **Step 3: Write the implementation** (append to `impl ReplayRing`)

```rust
    /// Segments for a Save Replay of the trailing `window_s` seconds
    /// (ddoc §6): starts at the latest keyframe segment at-or-before
    /// `end − window` so the clip decodes cleanly and covers the window.
    ///
    /// `exclude_before_s` is the smart no-overlap mode: footage at or
    /// before that point (the previous save's end) is never re-included.
    pub fn save_window(&self, window_s: f64, exclude_before_s: Option<f64>) -> Vec<&Segment> {
        let Some(last) = self.segments.back() else {
            return Vec::new();
        };
        let mut start_target = last.pts_end_s() - window_s;
        if let Some(x) = exclude_before_s {
            start_target = start_target.max(x);
        }

        // Latest keyframe segment starting at or before the target…
        let mut start_idx = self
            .segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.starts_with_keyframe && s.pts_start_s <= start_target)
            .map(|(i, _)| i)
            .next_back();
        // …or, if the buffer is shorter than the window, the first keyframe.
        if start_idx.is_none() {
            start_idx = self.segments.iter().position(|s| s.starts_with_keyframe);
        }
        let Some(mut idx) = start_idx else {
            return Vec::new();
        };

        // Smart mode: drop segments wholly at/before the exclusion point,
        // then re-align to the next keyframe.
        if let Some(x) = exclude_before_s {
            while idx < self.segments.len() && self.segments[idx].pts_end_s() <= x {
                idx += 1;
            }
            while idx < self.segments.len() && !self.segments[idx].starts_with_keyframe {
                idx += 1;
            }
        }

        self.segments.iter().skip(idx).collect()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p clipline-buffer`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(buffer): keyframe-aligned save-window with no-overlap smart mode

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: RAM estimator (`clipline-buffer`)

The user-facing estimator from ddoc §6/§7: buffer RAM ≈ bitrate × duration; the UI uses it to warn and to auto-switch to disk-spill.

**Files:**
- Create: `crates/clipline-buffer/src/estimate.rs`
- Modify: `crates/clipline-buffer/src/lib.rs`
- Test: inline `#[cfg(test)]` in `estimate.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/clipline-buffer/src/lib.rs`:
```rust
pub mod estimate;

pub use estimate::estimate_buffer_bytes;
```

`crates/clipline-buffer/src/estimate.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_ddoc_example() {
        // ddoc §7: 1080p60 @ ~40 Mbps ≈ ~1.5 GB for 5 min.
        assert_eq!(estimate_buffer_bytes(40_000_000, 300.0), 1_500_000_000);
    }

    #[test]
    fn zero_duration_is_zero() {
        assert_eq!(estimate_buffer_bytes(40_000_000, 0.0), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p clipline-buffer`
Expected: COMPILE ERROR (`estimate_buffer_bytes` not defined).

- [ ] **Step 3: Write the implementation** (top of `estimate.rs`)

```rust
/// Estimated RAM for a replay buffer of `duration_s` at `bitrate_bps`
/// (ddoc §6/§7). CBR makes this exact; CQP/VBR makes it an upper-ish bound
/// the UI presents before suggesting disk-spill.
pub fn estimate_buffer_bytes(bitrate_bps: u64, duration_s: f64) -> u64 {
    ((bitrate_bps as f64 / 8.0) * duration_s) as u64
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p clipline-buffer`
Expected: 8 tests pass.

- [ ] **Step 5: Commit, full workspace check**

Run: `cargo test --workspace && cargo clippy --workspace 2>&1 | tail -3` (clippy if installed; skip without failing if not)

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(buffer): replay-buffer RAM estimator

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
EOF
)"
```

---

## Follow-up plans (not in this plan)

1. **Hybrid MP4 muxer** (`clipline-mp4`): fragmented `moof`/`mdat` writer + finalize-to-standard-MP4 (ddoc §10) — platform-neutral, Linux-testable against ffprobe.
2. **Capture/encode platform layer** (`clipline-capture`): `CaptureEngine`/`AudioCapture`/`Encoder` traits, then `#[cfg(windows)]` WGC/WASAPI/NVENC-AMF-QSV implementations via windows-rs (ddoc §3/§4) — requires a Windows machine or CI for verification.
3. **Tauri shell** (`apps/clipline`): tray, hotkeys, settings, library UI, timeline markers (ddoc §3/§4) — UI iterable on Linux, WebView2 specifics verified on Windows.
4. **VALORANT/CS2 adapters** (M3, ddoc §5b/§5c) after the League adapter ships.
