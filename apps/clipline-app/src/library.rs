//! Clip library commands: inventory of `Videos\Clipline` for the UI and
//! a path-validated delete. The webview never touches the filesystem
//! directly — playback goes through the asset protocol.

use std::path::Path;

use clipline_events::ClipMarkers;
use clipline_mp4::walker::movie_duration_s;

use crate::service::clips_dir;

#[derive(serde::Serialize)]
pub struct ClipInfo {
    pub path: String,
    pub name: String,
    pub size_mb: f64,
    pub modified_unix: u64,
    pub duration_s: Option<f64>,
    pub markers: Option<ClipMarkers>,
}

#[tauri::command]
pub fn list_clips() -> Result<Vec<ClipInfo>, String> {
    let dir = clips_dir()?;
    let mut clips = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mp4") {
            continue;
        }
        let meta = entry.metadata().ok();
        let modified_unix = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size_mb =
            meta.map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0);
        // Full read is fine at clip sizes; the moov tail needs the soft-
        // remuxed file anyway. Revisit if listing ever feels slow.
        let duration_s = std::fs::read(&path).ok().and_then(|buf| movie_duration_s(&buf));
        let markers = std::fs::read_to_string(path.with_extension("markers.json"))
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());
        clips.push(ClipInfo {
            path: path.display().to_string(),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size_mb,
            modified_unix,
            duration_s,
            markers,
        });
    }
    clips.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix));
    Ok(clips)
}

#[tauri::command]
pub fn delete_clip(path: String) -> Result<(), String> {
    let dir = clips_dir()?.canonicalize().map_err(|e| e.to_string())?;
    let target = Path::new(&path).canonicalize().map_err(|e| e.to_string())?;
    if target.parent() != Some(dir.as_path())
        || target.extension().and_then(|e| e.to_str()) != Some("mp4")
    {
        return Err("refusing to delete outside the clips directory".into());
    }
    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    Ok(())
}
