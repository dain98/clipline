//! Shared helpers used by multiple app modules.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use clipline_events::ClipMarkers;

/// Read the `.markers.json` sidecar next to a clip file.
pub(crate) fn read_markers_raw(path: &Path) -> Option<ClipMarkers> {
    std::fs::read_to_string(path.with_extension("markers.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
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
