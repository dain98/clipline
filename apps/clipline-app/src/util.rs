//! Shared helpers used by multiple app modules.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use clipline_events::{ClipAudioTrack, ClipMarkers};

/// Read the `.markers.json` sidecar next to a clip file.
pub(crate) fn read_markers_raw(path: &Path) -> Option<ClipMarkers> {
    std::fs::read_to_string(path.with_extension("markers.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
}

pub(crate) fn markers_with_inferred_audio_tracks(
    path: &Path,
    markers: Option<ClipMarkers>,
) -> Option<ClipMarkers> {
    if markers
        .as_ref()
        .is_some_and(|markers| !markers.audio_tracks.is_empty())
    {
        return markers;
    }

    let audio_tracks = infer_audio_tracks_from_file(path).unwrap_or_default();
    if audio_tracks.is_empty() {
        return markers;
    }

    Some(match markers {
        Some(mut markers) => {
            markers.audio_tracks = audio_tracks;
            markers
        }
        None => ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 0.0,
            player_summary: None,
            audio_tracks,
            markers: Vec::new(),
        },
    })
}

fn infer_audio_tracks_from_file(path: &Path) -> Result<Vec<ClipAudioTrack>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read clip audio metadata: {e}"))?;
    let count = clipline_mp4::audio_track_count(&bytes).map_err(|e| e.to_string())?;
    Ok((0..count)
        .map(|index| ClipAudioTrack {
            id: format!("audio:{index}"),
            track_index: index as u32,
            label: format!("Audio Track {}", index + 1),
            kind: Some("inferred".into()),
        })
        .collect())
}

/// Encode an OS string as a null-terminated UTF-16 vector for Win32 wide APIs.
pub(crate) fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

/// Format a Win32 last-OS-error into a human-readable message.
pub(crate) fn last_os_error(action: &str) -> String {
    format!("{action}: {}", std::io::Error::last_os_error())
}

/// Current wall-clock time as seconds since the Unix epoch.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Resolve user-facing audio track IDs to their MP4 track indices, validating
/// for duplicates and unknown IDs.
pub(crate) fn selected_audio_track_indices(
    markers: &ClipMarkers,
    selected_audio_track_ids: &[String],
) -> Result<Vec<u32>, String> {
    let selected_ids: BTreeSet<&str> = selected_audio_track_ids
        .iter()
        .map(String::as_str)
        .collect();
    if selected_ids.len() != selected_audio_track_ids.len() {
        return Err("audio track selection contains duplicates".into());
    }
    let available: BTreeSet<&str> = markers
        .audio_tracks
        .iter()
        .map(|track| track.id.as_str())
        .collect();
    if let Some(unknown) = selected_ids
        .iter()
        .find(|selected| !available.contains(**selected))
    {
        return Err(format!("unknown audio track {unknown:?}"));
    }
    Ok(markers
        .audio_tracks
        .iter()
        .filter(|track| selected_ids.contains(track.id.as_str()))
        .map(|track| track.track_index)
        .collect())
}
