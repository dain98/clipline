use std::path::{Path, PathBuf};
use std::sync::Mutex;

use clipline_events::ClipMarkers;

use crate::service::{clips_dir, default_clips_dir};

pub struct StorageSettings {
    quota_bytes: Mutex<Option<u64>>,
    media_dir: Mutex<PathBuf>,
}

impl StorageSettings {
    pub fn new(quota_bytes: Option<u64>, media_dir: PathBuf) -> Self {
        Self {
            quota_bytes: Mutex::new(quota_bytes),
            media_dir: Mutex::new(media_dir),
        }
    }

    pub fn quota_bytes(&self) -> Option<u64> {
        self.quota_bytes.lock().map(|quota| *quota).unwrap_or(None)
    }

    pub fn set_quota_bytes(&self, quota_bytes: Option<u64>) {
        if let Ok(mut quota) = self.quota_bytes.lock() {
            *quota = quota_bytes;
        }
    }

    pub fn media_dir(&self) -> PathBuf {
        self.media_dir
            .lock()
            .map(|dir| dir.clone())
            .unwrap_or_else(|_| default_clips_dir())
    }

    pub fn set_media_dir(&self, media_dir: PathBuf) {
        if let Ok(mut dir) = self.media_dir.lock() {
            *dir = media_dir;
        }
    }

    fn clips_dir(&self) -> Result<PathBuf, String> {
        clips_dir(&self.media_dir())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ClipGame {
    pub id: String,
    pub name: String,
}

#[derive(serde::Serialize)]
pub struct ClipInfo {
    pub path: String,
    pub name: String,
    pub session: Option<String>,
    pub size_mb: f64,
    pub modified_unix: u64,
    pub duration_s: Option<f64>,
    pub markers: Option<ClipMarkers>,
    pub game: Option<ClipGame>,
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
    pub size_mb: f64,
    pub modified_unix: u64,
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub aligned_start_s: f64,
    pub aligned_end_s: f64,
    pub duration_s: f64,
    pub markers: Option<ClipMarkers>,
}

#[derive(serde::Serialize)]
pub struct RenamedClipInfo {
    pub old_path: String,
    pub path: String,
    pub name: String,
}

#[derive(serde::Deserialize)]
pub struct AudioPreviewRequest {
    pub path: String,
    #[serde(default, rename = "audioTrackIds")]
    pub audio_track_ids: Vec<String>,
}

#[tauri::command]
pub async fn list_clips(
    settings: tauri::State<'_, StorageSettings>,
) -> Result<Vec<ClipInfo>, String> {
    let dir = settings.clips_dir()?;
    tauri::async_runtime::spawn_blocking(move || list_clips_from_dir(&dir))
        .await
        .map_err(|e| format!("list clips task: {e}"))?
}

fn list_clips_from_dir(dir: &Path) -> Result<Vec<ClipInfo>, String> {
    let mut clips = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(clips),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("mp4") {
            continue;
        }
        let meta = entry.metadata().ok();
        let modified_unix = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let size_mb = meta
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        clips.push(ClipInfo {
            path: path.display().to_string(),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            session: None,
            size_mb,
            modified_unix,
            duration_s: None,
            markers: crate::util::read_markers_raw(&path),
            game: None,
        });
    }
    clips.sort_by_key(|clip| std::cmp::Reverse(clip.modified_unix));
    Ok(clips)
}

#[tauri::command]
pub async fn clip_poster(
    path: String,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<String, String> {
    let target = validate_clip_path(&settings, &path)?;
    crate::poster::ensure_poster(&target, 1.0).map(|poster| poster.display().to_string())
}

#[tauri::command]
pub fn delete_clip(path: String, settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    let _ = std::fs::remove_file(crate::poster::poster_path(&target));
    Ok(())
}

#[tauri::command]
pub async fn rename_clip(
    _path: String,
    _name: String,
    _settings: tauri::State<'_, StorageSettings>,
    _state: tauri::State<'_, crate::app::RuntimeState>,
) -> Result<RenamedClipInfo, String> {
    Err("clip rename is unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub async fn export_clip(
    _path: String,
    _start_s: f64,
    _end_s: f64,
    _settings: tauri::State<'_, StorageSettings>,
) -> Result<ExportedClipInfo, String> {
    Err("clip export is unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub async fn preview_clip_audio_tracks<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    request: AudioPreviewRequest,
    _settings: tauri::State<'_, StorageSettings>,
) -> Result<String, String> {
    let _ = request.audio_track_ids;
    Ok(request.path)
}

pub(crate) fn validate_clip_path(
    settings: &StorageSettings,
    path: &str,
) -> Result<PathBuf, String> {
    let dir = settings
        .clips_dir()?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let target = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
    let parent_ok = target.parent() == Some(dir.as_path())
        || target.parent().and_then(Path::parent) == Some(dir.as_path());
    if !parent_ok || target.extension().and_then(|ext| ext.to_str()) != Some("mp4") {
        return Err("refusing to access a clip outside the clips directory".into());
    }
    Ok(target)
}

#[tauri::command]
pub fn reveal_clip(_path: String, _settings: tauri::State<StorageSettings>) -> Result<(), String> {
    Err("showing clips in Finder is unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub fn copy_clip_to_clipboard(
    _path: String,
    _settings: tauri::State<StorageSettings>,
) -> Result<(), String> {
    Err("copying clips to the clipboard is unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub fn open_media_folder(settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let dir = settings.clips_dir()?;
    std::process::Command::new("open")
        .arg(dir)
        .spawn()
        .map_err(|e| format!("open Finder: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn storage_status(
    settings: tauri::State<'_, StorageSettings>,
) -> Result<StorageInfo, String> {
    let dir = settings.clips_dir()?;
    let quota_bytes = settings.quota_bytes();
    tauri::async_runtime::spawn_blocking(move || {
        let status =
            clipline_storage::storage_status(&dir, quota_bytes).map_err(|e| e.to_string())?;
        Ok(StorageInfo {
            clip_count: status.clip_count,
            total_bytes: status.total_bytes,
            quota_bytes: status.quota_bytes,
            over_quota: status.is_over_quota(),
        })
    })
    .await
    .map_err(|e| format!("storage status task: {e}"))?
}
