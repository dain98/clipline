# Clipline Event Markers in Saved Clips (Milestone 6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The differentiating feature (ddoc §5) goes live: while the replay buffer runs, the
League adapter polls the Live Client Data API; anchored events accumulate in a marker log; on
**Save Replay**, the markers inside the saved window land in `<clip>.markers.json` next to the
MP4, re-based to clip time. **Exit criterion:** the httpmock-driven integration test proves the
whole chain (poll → anchor → log → clip window → sidecar) on CI, and the app writes sidecars
live (verified against a real match when one is running; the mock URL flag covers it
otherwise).

**Architecture:** Everything that thinks is platform-neutral. `clipline-events` gains
`MarkerLog` (append anchored `GameEvent`s; `clip_markers(start_s, end_s)` filters the window
and re-bases offsets to clip start) and `ClipMarkers` (the serializable sidecar document:
clip range + markers). `clipline-lol` gets an end-to-end integration test against httpmock
(the established fixture pattern) running the real `poll_once` into a real `MarkerLog`.
The app side is thin: a poller thread (tokio current-thread runtime — clipline-lol is
async/reqwest) hits `LiveClient::default_local()` (or `--lol-url` for mocks) every second,
retries quietly while no game is running, and forwards events over a channel; the recorder
service owns the `MarkerLog`, drains the channel in its loop, and on save writes the sidecar
and reports the marker count in the `Saved` event.

**Clock bridge:** `recording_offset_s` maps events onto a timeline anchored at an `Instant`
`t0`; capture pts are anchored at the QPC origin minted by `WgcCapture::new_clock()`. On
Windows, `Instant` is QPC — sampling `Instant::now()` adjacent to `new_clock()` makes the two
timelines identical to within microseconds, which is far inside the ±0.5 s a timeline marker
needs. (ddoc §5's `emit_latency_s` nudge stays configurable, default 0.)

**Tech Stack:** no new deps in the libraries. App gains `tokio` (workspace dep, rt only) and
`clipline-lol`/`clipline-events` — all under the existing `cfg(windows)` gate.

**Environment notes:** the Live Client API only exists in-game (HTTPS, self-signed —
`default_local` handles it). Commits end with
`Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: `MarkerLog` + `ClipMarkers` (clipline-events)

**Files:** Create `crates/clipline-events/src/markers.rs`; modify `src/lib.rs`.

- [ ] **Step 1: failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EventKind, GameEvent, GameId};

    fn ev(kind: EventKind, offset_s: f64) -> GameEvent {
        GameEvent {
            game_id: GameId::LeagueOfLegends,
            kind,
            actor: "Dain".into(),
            victim: None,
            assisters: Vec::new(),
            subtype: None,
            game_time_s: 0.0,
            recording_offset_s: Some(offset_s),
            importance: 5,
            involves_local_player: true,
        }
    }

    #[test]
    fn clip_markers_filters_and_rebases_to_clip_start() {
        let mut log = MarkerLog::new();
        log.push(ev(EventKind::ChampionKill, 10.0));
        log.push(ev(EventKind::DragonKill, 70.0));
        log.push(ev(EventKind::BaronKill, 130.0));
        let clip = log.clip_markers(60.0, 120.0);
        assert_eq!(clip.markers.len(), 1, "only the dragon is inside the window");
        assert!((clip.markers[0].t_s - 10.0).abs() < 1e-9, "70s − 60s clip start");
        assert_eq!(clip.markers[0].event.kind, EventKind::DragonKill);
        assert!((clip.duration_s - 60.0).abs() < 1e-9);
    }

    #[test]
    fn unanchored_events_are_ignored() {
        let mut log = MarkerLog::new();
        let mut e = ev(EventKind::ChampionKill, 0.0);
        e.recording_offset_s = None;
        log.push(e);
        assert_eq!(log.clip_markers(0.0, 100.0).markers.len(), 0);
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn boundary_inclusive_start_exclusive_end() {
        let mut log = MarkerLog::new();
        log.push(ev(EventKind::ChampionKill, 60.0));
        log.push(ev(EventKind::Ace, 120.0));
        let clip = log.clip_markers(60.0, 120.0);
        assert_eq!(clip.markers.len(), 1);
        assert_eq!(clip.markers[0].event.kind, EventKind::ChampionKill);
    }

    #[test]
    fn sidecar_serializes_round_trip() {
        let mut log = MarkerLog::new();
        log.push(ev(EventKind::ChampionKill, 65.0));
        let clip = log.clip_markers(60.0, 120.0);
        let json = serde_json::to_string_pretty(&clip).unwrap();
        let back: ClipMarkers = serde_json::from_str(&json).unwrap();
        assert_eq!(back.markers.len(), 1);
        assert!((back.markers[0].t_s - 5.0).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: verify failure → Step 3: implement**

```rust
//! Marker accumulation during a recording and extraction into saved clips
//! (ddoc §5: normalized events land on the recording timeline; saved clips
//! carry the markers inside their window, re-based to clip time).

use serde::{Deserialize, Serialize};

use crate::schema::GameEvent;

/// All anchored events of the current recording session, in arrival order.
#[derive(Debug, Default)]
pub struct MarkerLog {
    events: Vec<GameEvent>, // every entry has recording_offset_s = Some
}

/// One marker inside a saved clip, `t_s` seconds from clip start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipMarker {
    pub t_s: f64,
    #[serde(flatten)]
    pub event: GameEvent,
}

/// The `<clip>.markers.json` sidecar document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipMarkers {
    /// Recording-timeline range the clip covers.
    pub recording_start_s: f64,
    pub duration_s: f64,
    pub markers: Vec<ClipMarker>,
}

impl MarkerLog {
    pub fn new() -> Self { Self::default() }

    /// Unanchored events (no recording offset yet) are dropped — they
    /// cannot be placed on the timeline.
    pub fn push(&mut self, event: GameEvent) {
        if event.recording_offset_s.is_some() {
            self.events.push(event);
        }
    }

    pub fn len(&self) -> usize { self.events.len() }
    pub fn is_empty(&self) -> bool { self.events.is_empty() }

    /// Markers within [start, end), re-based to clip time.
    pub fn clip_markers(&self, start_s: f64, end_s: f64) -> ClipMarkers {
        let markers = self
            .events
            .iter()
            .filter_map(|e| {
                let off = e.recording_offset_s?;
                (off >= start_s && off < end_s)
                    .then(|| ClipMarker { t_s: off - start_s, event: e.clone() })
            })
            .collect();
        ClipMarkers { recording_start_s: start_s, duration_s: end_s - start_s, markers }
    }
}
```
(`serde(flatten)` on `event` keeps the sidecar flat and readable; `serde_json` becomes a
dev-dependency of clipline-events if it isn't one.)

- [ ] **Step 4: pass → Step 5: commit** `feat(events): MarkerLog + clip sidecar extraction`.

---

### Task 2: end-to-end integration test (clipline-lol, httpmock)

**Files:** Create `crates/clipline-lol/tests/markers_e2e.rs` (reuse the existing fixture
helpers/pattern from the current httpmock tests).

- [ ] One test: mock `/liveclientdata/gamestats` (game time) + `/eventdata` +
`/activeplayername`; `poll_once` twice (second poll adds a fresh event — dedupe proves out);
push everything into a `MarkerLog`; `clip_markers` over a window that includes one event and
excludes another; assert the sidecar JSON contains the right marker at the right `t_s`.
This is the CI-proof of the full chain.
- [ ] Commit: `test(lol): markers pipeline e2e against the mock Live Client API`.

---

### Task 3: app wiring

**Files:** `apps/clipline-app/Cargo.toml` (+ `clipline-events`, `clipline-lol`, `tokio` under
the windows gate), `src/service.rs`, `src/markers.rs` (new), `src/app.rs`, `ui/index.html`.

- `markers.rs`: `spawn(base_url: Option<String>, recording_t0: Instant) -> Receiver<GameEvent>`
  — a thread with a current-thread tokio runtime: `LiveClient::new(url)` /`default_local()`;
  loop: `active_player_name` (retry every 5 s until a game exists — quiet), then 1 Hz
  `poll_once`, forwarding events; on error fall back to the waiting state (game ended).
- `service.rs`: mint `recording_t0 = Instant::now()` next to `new_clock()`; create the marker
  receiver + `MarkerLog`; drain into the log each loop turn; on save:
  `log.clip_markers(saved_from, end)` → write `<clip>.markers.json` (only when non-empty),
  add `markers: usize` to `Event::Saved`.
- `app.rs`/UI: pass `--lol-url` through; show the marker count on saved rows.

- [ ] Compile + clippy. Commit: `feat(app): League event markers ride along saved clips`.

---

### Task 4: gates + handoff

- [ ] `cargo test --workspace` (the new e2e runs on both OSes), clippy zero warnings, push,
CI green.
- [ ] If a League match is live on the dev machine: run the app during it, save, confirm the
sidecar contents look right; otherwise note that the mock e2e covers the chain and live
verification is a follow-up moment.
- [ ] `handoff.md`: milestone 6 done; remaining frontier (FFmpeg matrix, per-process audio,
library/timeline UI rendering these markers, installer).

---

## Out of scope (follow-ups)

- Timeline UI rendering of markers (needs the library/editor view).
- Auto-clip on high-importance events (`importance` drives thresholds — ddoc §5).
- VALORANT kill-feed OCR adapter; generic adapters.
- Embedding markers as MP4 chapter atoms (sidecar JSON is the v1 contract).
