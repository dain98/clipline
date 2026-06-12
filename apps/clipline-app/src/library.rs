//! Clip library commands: inventory of `Videos\Clipline` for the UI and
//! a path-validated delete. The webview never touches the filesystem
//! directly — playback goes through the asset protocol.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use clipline_events::{ClipMarker, ClipMarkers};
use clipline_mp4::trim_keyframe_aligned;
use clipline_mp4::walker::movie_duration_s;
use clipline_storage::storage_status as read_storage_status;

use crate::service::clips_dir;

pub struct StorageSettings {
    quota_bytes: Mutex<Option<u64>>,
}

impl StorageSettings {
    pub fn new(quota_bytes: Option<u64>) -> Self {
        Self {
            quota_bytes: Mutex::new(quota_bytes),
        }
    }

    pub fn quota_bytes(&self) -> Option<u64> {
        self.quota_bytes.lock().map(|q| *q).unwrap_or(None)
    }

    pub fn set_quota_bytes(&self, quota_bytes: Option<u64>) {
        if let Ok(mut q) = self.quota_bytes.lock() {
            *q = quota_bytes;
        }
    }
}

#[derive(serde::Serialize)]
pub struct ClipInfo {
    pub path: String,
    pub name: String,
    /// Session folder name; None for legacy clips at the library root.
    pub session: Option<String>,
    pub size_mb: f64,
    pub modified_unix: u64,
    pub duration_s: Option<f64>,
    pub markers: Option<ClipMarkers>,
}

#[derive(serde::Serialize)]
pub struct StorageInfo {
    pub clip_count: usize,
    pub total_bytes: u64,
    pub quota_bytes: Option<u64>,
    pub over_quota: bool,
}

#[derive(serde::Serialize)]
pub struct ExportedClipInfo {
    pub path: String,
    pub name: String,
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub aligned_start_s: f64,
    pub aligned_end_s: f64,
    pub duration_s: f64,
}

#[tauri::command]
pub fn list_clips() -> Result<Vec<ClipInfo>, String> {
    let dir = clips_dir()?;
    let mut clips = Vec::new();
    push_clips_from(&dir, None, &mut clips)?;
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        if entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
            let session = entry.file_name().to_string_lossy().into_owned();
            push_clips_from(&entry.path(), Some(session), &mut clips)?;
        }
    }
    clips.sort_by_key(|c| std::cmp::Reverse(c.modified_unix));
    Ok(clips)
}

fn push_clips_from(
    dir: &Path,
    session: Option<String>,
    clips: &mut Vec<ClipInfo>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mp4") {
            continue;
        }
        let meta = entry.metadata().ok();
        if meta.as_ref().is_some_and(|m| !m.is_file()) {
            continue;
        }
        let modified_unix = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size_mb = meta
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        // Full read is fine at clip sizes; the moov tail needs the soft-
        // remuxed file anyway. Revisit if listing ever feels slow.
        let duration_s = std::fs::read(&path)
            .ok()
            .and_then(|buf| movie_duration_s(&buf));
        let markers = std::fs::read_to_string(path.with_extension("markers.json"))
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok());
        clips.push(ClipInfo {
            path: path.display().to_string(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            session: session.clone(),
            size_mb,
            modified_unix,
            duration_s,
            markers,
        });
    }
    Ok(())
}

#[tauri::command]
pub fn delete_clip(path: String) -> Result<(), String> {
    let target = validate_clip_path(&path)?;
    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    Ok(())
}

#[tauri::command]
pub fn export_clip(path: String, start_s: f64, end_s: f64) -> Result<ExportedClipInfo, String> {
    let source = validate_clip_path(&path)?;
    let input = std::fs::read(&source).map_err(|e| e.to_string())?;
    let (output, info) =
        trim_keyframe_aligned(&input, start_s, end_s).map_err(|e| e.to_string())?;
    let target = unique_export_path(&source, info.aligned_start_s, info.aligned_end_s)?;
    std::fs::write(&target, output).map_err(|e| e.to_string())?;

    if let Some(markers) = read_markers(&source) {
        let cropped = crop_markers(&markers, info.aligned_start_s, info.aligned_end_s);
        if !cropped.markers.is_empty() {
            let json = serde_json::to_string_pretty(&cropped).map_err(|e| e.to_string())?;
            std::fs::write(target.with_extension("markers.json"), json)
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(ExportedClipInfo {
        path: target.display().to_string(),
        name: target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        requested_start_s: info.requested_start_s,
        requested_end_s: info.requested_end_s,
        aligned_start_s: info.aligned_start_s,
        aligned_end_s: info.aligned_end_s,
        duration_s: info.duration_s,
    })
}

#[tauri::command]
pub fn storage_status(settings: tauri::State<StorageSettings>) -> Result<StorageInfo, String> {
    let status =
        read_storage_status(&clips_dir()?, settings.quota_bytes()).map_err(|e| e.to_string())?;
    Ok(StorageInfo {
        clip_count: status.clip_count,
        total_bytes: status.total_bytes,
        quota_bytes: status.quota_bytes,
        over_quota: status.is_over_quota(),
    })
}

fn validate_clip_path(path: &str) -> Result<PathBuf, String> {
    let dir = clips_dir()?.canonicalize().map_err(|e| e.to_string())?;
    let target = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
    // Legacy clips sit at the root; session clips one folder down.
    let parent_ok = target.parent() == Some(dir.as_path())
        || target.parent().and_then(Path::parent) == Some(dir.as_path());
    if !parent_ok || target.extension().and_then(|e| e.to_str()) != Some("mp4") {
        return Err("refusing to access a clip outside the clips directory".into());
    }
    Ok(target)
}

#[tauri::command]
pub fn reveal_clip(path: String) -> Result<(), String> {
    let target = validate_clip_path(&path)?;
    // Explorer wants `/select,<path>` as a single argument.
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", target.display()))
        .spawn()
        .map_err(|e| format!("open explorer: {e}"))?;
    Ok(())
}

fn read_markers(path: &Path) -> Option<ClipMarkers> {
    std::fs::read_to_string(path.with_extension("markers.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
}

fn crop_markers(markers: &ClipMarkers, start_s: f64, end_s: f64) -> ClipMarkers {
    let cropped = markers
        .markers
        .iter()
        .filter(|m| m.t_s >= start_s && m.t_s < end_s)
        .map(|m| ClipMarker {
            t_s: m.t_s - start_s,
            event: m.event.clone(),
        })
        .collect();
    ClipMarkers {
        recording_start_s: markers.recording_start_s + start_s,
        duration_s: end_s - start_s,
        markers: cropped,
    }
}

fn unique_export_path(source: &Path, start_s: f64, end_s: f64) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "source clip has no parent directory".to_string())?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy())
        .ok_or_else(|| "source clip has no file stem".to_string())?;
    let start_ms = (start_s * 1000.0).round().max(0.0) as u64;
    let end_ms = (end_s * 1000.0).round().max(0.0) as u64;
    for suffix in 0..1000u32 {
        let name = if suffix == 0 {
            format!("{stem}_trim_{start_ms:06}_{end_ms:06}.mp4")
        } else {
            format!("{stem}_trim_{start_ms:06}_{end_ms:06}_{suffix}.mp4")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("could not choose an unused export filename".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_events::{EventKind, GameEvent, GameId};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipline-library-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn marker(t_s: f64) -> ClipMarker {
        ClipMarker {
            t_s,
            event: GameEvent {
                game_id: GameId::LeagueOfLegends,
                kind: EventKind::ChampionKill,
                actor: "Dain".into(),
                victim: None,
                assisters: Vec::new(),
                subtype: None,
                game_time_s: 0.0,
                recording_offset_s: Some(10.0 + t_s),
                importance: 7,
                involves_local_player: true,
            },
        }
    }

    #[test]
    fn crop_markers_rebases_times_and_recording_start() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 5.0,
            markers: vec![marker(0.5), marker(1.5), marker(2.5)],
        };

        let cropped = crop_markers(&markers, 1.0, 2.0);

        assert_eq!(cropped.markers.len(), 1);
        assert!((cropped.markers[0].t_s - 0.5).abs() < 1e-9);
        assert!((cropped.recording_start_s - 11.0).abs() < 1e-9);
        assert!((cropped.duration_s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unique_export_path_appends_suffix_when_needed() {
        let dir = TestDir::new("export-name");
        let source = dir.path().join("clip_1.mp4");
        let first = dir.path().join("clip_1_trim_001000_002000.mp4");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&first, b"existing").unwrap();

        let path = unique_export_path(&source, 1.0, 2.0).unwrap();

        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "clip_1_trim_001000_002000_1.mp4"
        );
    }
}
