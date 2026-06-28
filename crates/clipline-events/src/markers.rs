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

/// One player in a game adapter's match summary roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerParticipant {
    pub player_name: String,
    pub champion_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team: String,
}

/// Per-player match summary shown in library rows when a game adapter can
/// provide it. Extra participant fields are optional so older sidecars remain
/// readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSummary {
    pub champion_name: String,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub player_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<PlayerParticipant>,
}

/// One user-facing audio stream inside a saved clip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipAudioTrack {
    /// Stable id for UI/upload selection, e.g. "output" or "microphone".
    pub id: String,
    /// Zero-based audio-track index in the MP4, excluding the video track.
    pub track_index: u32,
    pub label: String,
    /// Machine-readable source kind for future process/game audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// The `<clip>.markers.json` sidecar document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipMarkers {
    /// Recording-timeline range the clip covers.
    pub recording_start_s: f64,
    pub duration_s: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_summary: Option<PlayerSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_tracks: Vec<ClipAudioTrack>,
    pub markers: Vec<ClipMarker>,
}

impl MarkerLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unanchored events (no recording offset yet) are dropped — they
    /// cannot be placed on the timeline.
    pub fn push(&mut self, event: GameEvent) {
        if event.recording_offset_s.is_some() {
            self.events.push(event);
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Markers within [start, end), re-based to clip time.
    pub fn clip_markers(&self, start_s: f64, end_s: f64) -> ClipMarkers {
        let markers = self
            .events
            .iter()
            .filter_map(|e| {
                let off = e.recording_offset_s?;
                (off >= start_s && off < end_s).then(|| ClipMarker {
                    t_s: off - start_s,
                    event: e.clone(),
                })
            })
            .collect();
        ClipMarkers {
            recording_start_s: start_s,
            duration_s: end_s - start_s,
            player_summary: None,
            audio_tracks: Vec::new(),
            markers,
        }
    }
}

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
        assert_eq!(
            clip.markers.len(),
            1,
            "only the dragon is inside the window"
        );
        assert!(
            (clip.markers[0].t_s - 10.0).abs() < 1e-9,
            "70s − 60s clip start"
        );
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
        let mut clip = log.clip_markers(60.0, 120.0);
        clip.player_summary = Some(PlayerSummary {
            champion_name: "Nautilus".into(),
            kills: 3,
            deaths: 4,
            assists: 23,
            player_name: String::new(),
            team: String::new(),
            participants: Vec::new(),
        });
        clip.audio_tracks = vec![ClipAudioTrack {
            id: "output".into(),
            track_index: 0,
            label: "Output Audio".into(),
            kind: Some("output".into()),
        }];
        let json = serde_json::to_string_pretty(&clip).unwrap();
        let back: ClipMarkers = serde_json::from_str(&json).unwrap();
        assert_eq!(back.markers.len(), 1);
        assert_eq!(back.audio_tracks, clip.audio_tracks);
        assert!((back.markers[0].t_s - 5.0).abs() < 1e-9);
        assert_eq!(
            back.player_summary.as_ref().map(|summary| (
                summary.champion_name.as_str(),
                summary.kills,
                summary.deaths,
                summary.assists
            )),
            Some(("Nautilus", 3, 4, 23))
        );
    }

    #[test]
    fn sidecar_without_player_summary_still_round_trips() {
        let json = r#"{
          "recording_start_s": 0.0,
          "duration_s": 1.0,
          "markers": []
        }"#;
        let back: ClipMarkers = serde_json::from_str(json).unwrap();
        assert!(back.player_summary.is_none());
        assert!(back.audio_tracks.is_empty());
        assert!(back.markers.is_empty());
    }

    #[test]
    fn player_summary_defaults_missing_participant_data() {
        let json = r#"{
          "champion_name": "Nautilus",
          "kills": 3,
          "deaths": 4,
          "assists": 23
        }"#;

        let summary: PlayerSummary = serde_json::from_str(json).unwrap();

        assert_eq!(summary.champion_name, "Nautilus");
        assert_eq!((summary.kills, summary.deaths, summary.assists), (3, 4, 23));
        assert!(summary.player_name.is_empty());
        assert!(summary.team.is_empty());
        assert!(summary.participants.is_empty());
    }
}
