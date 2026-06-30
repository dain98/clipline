//! Clip library commands: inventory of the configured media folder for the UI and
//! a path-validated delete. The webview never touches the filesystem
//! directly — playback goes through the asset protocol.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::Mutex;

use clipline_events::{is_review_event, ClipMarker, ClipMarkers};
use clipline_mp4::{
    remux_with_mixed_audio_track, remux_with_selected_audio_tracks, trim_keyframe_aligned_file,
};
use clipline_storage::storage_status as read_storage_status;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_HDROP;
use windows_sys::Win32::UI::Shell::DROPFILES;

use tauri::{AppHandle, Manager, Runtime};

use crate::service::{clips_dir, default_clips_dir};
use crate::util;
use crate::util::last_os_error;

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
        match self.quota_bytes.lock() {
            Ok(q) => *q,
            Err(e) => {
                eprintln!("quota_bytes lock poisoned: {e}");
                None
            }
        }
    }

    pub fn set_quota_bytes(&self, quota_bytes: Option<u64>) {
        match self.quota_bytes.lock() {
            Ok(mut q) => *q = quota_bytes,
            Err(e) => eprintln!("set_quota_bytes lock poisoned: {e}"),
        }
    }

    pub fn media_dir(&self) -> PathBuf {
        match self.media_dir.lock() {
            Ok(dir) => dir.clone(),
            Err(e) => {
                eprintln!("media_dir lock poisoned: {e}");
                default_clips_dir()
            }
        }
    }

    pub fn set_media_dir(&self, media_dir: PathBuf) {
        match self.media_dir.lock() {
            Ok(mut dir) => *dir = media_dir,
            Err(e) => eprintln!("set_media_dir lock poisoned: {e}"),
        }
    }

    fn clips_dir(&self) -> Result<PathBuf, String> {
        clips_dir(&self.media_dir())
    }
}

/// The game a clip's session folder is attributed to (see
/// `clipline-session.json`). Drives the library's per-clip game icon.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ClipGame {
    pub id: String,
    pub name: String,
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
    /// Game this clip's session belongs to, if recorded under a detected game.
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

#[derive(serde::Deserialize)]
pub struct CopyClipToClipboardRequest {
    pub path: String,
    #[serde(default, rename = "audioTrackIds")]
    pub audio_track_ids: Option<Vec<String>>,
}

#[tauri::command]
pub async fn list_clips(
    settings: tauri::State<'_, StorageSettings>,
) -> Result<Vec<ClipInfo>, String> {
    let dir = settings.clips_dir()?;
    tauri::async_runtime::spawn_blocking(move || list_clips_from_dir(dir))
        .await
        .map_err(|e| format!("list clips task: {e}"))?
}

fn list_clips_from_dir(dir: PathBuf) -> Result<Vec<ClipInfo>, String> {
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
    // One game tag per session folder, shared by every clip inside it.
    let session_game: Option<ClipGame> = std::fs::read_to_string(dir.join("clipline-session.json"))
        .ok()
        .and_then(|json| serde_json::from_str::<ClipGame>(&json).ok());
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
        let raw_markers = util::read_markers_raw(&path).map(filter_review_markers);
        let duration_s = raw_markers
            .as_ref()
            .map(|markers| markers.duration_s)
            .filter(|duration| duration.is_finite() && *duration >= 0.0);
        let markers = util::markers_with_inferred_audio_tracks(&path, raw_markers);
        // Prefer the session sidecar; fall back to the game named in markers
        // so clips recorded before session tagging still show an icon.
        let game = session_game
            .clone()
            .or_else(|| game_from_markers(markers.as_ref()));
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
            game,
        });
    }
    Ok(())
}

/// Fall back to the game named in a clip's markers when its session folder has
/// no game sidecar (clips recorded before session tagging existed). Only games
/// with a matching plugin resolve to an icon in the UI.
fn game_from_markers(markers: Option<&ClipMarkers>) -> Option<ClipGame> {
    let game_id = markers?.markers.first()?.event.game_id;
    let plugin_id = crate::game_plugins::plugin_id_for_game_id(game_id);
    let name = crate::game_plugins::all()
        .iter()
        .find(|plugin| plugin.id() == plugin_id)
        .map(|plugin| plugin.manifest.name.clone())?;
    Some(ClipGame {
        id: plugin_id.to_string(),
        name,
    })
}

/// Return (generating on demand) the cached poster JPEG for a clip, as a path
/// the webview loads through the asset protocol. Lazy and per-clip so the
/// library listing never blocks on ffmpeg.
#[tauri::command]
pub async fn clip_poster(
    path: String,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<String, String> {
    let target = validate_clip_path(&settings, &path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let seek_s = poster_seek_seconds(&target);
        crate::poster::ensure_poster(&target, seek_s).map(|poster| poster.display().to_string())
    })
    .await
    .map_err(|e| format!("clip poster task: {e}"))?
}

/// The frame to grab a poster from: the first timeline marker (the action
/// moment that makes the best thumbnail), else a little into the clip to skip
/// the black opening frame.
fn poster_seek_seconds(clip: &Path) -> f64 {
    let Some(markers) = util::read_markers_raw(clip) else {
        return 1.0;
    };
    let duration_ok = markers.duration_s.is_finite() && markers.duration_s > 0.0;
    if let Some(first) = markers.markers.first() {
        let t = first.t_s.max(0.0);
        return if duration_ok {
            t.min((markers.duration_s - 0.2).max(0.0))
        } else {
            t
        };
    }
    if duration_ok {
        (markers.duration_s * 0.15).min(5.0)
    } else {
        1.0
    }
}

#[tauri::command]
pub fn delete_clip(path: String, settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    remove_clip_files(&target)
}

fn remove_clip_files(target: &Path) -> Result<(), String> {
    std::fs::remove_file(target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    let _ = std::fs::remove_file(crate::poster::poster_path(target));
    Ok(())
}

/// A bulk-delete result: the paths that were removed and the (path, reason)
/// pairs that could not be. Surface `failed` to the UI so partial success is
/// visible rather than silently swallowed.
#[derive(serde::Serialize)]
pub struct DeletedClipsReport {
    pub deleted: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Testable core of [`delete_clips`]: deletes each already-validated clip plus
/// its `markers.json` sidecar and cached poster (best effort), recording any
/// removal failures. `failed` carries inputs that already failed validation so
/// the caller's report stays complete in one place.
fn delete_clips_impl(
    validated: Vec<(String, PathBuf)>,
    mut failed: Vec<(String, String)>,
) -> DeletedClipsReport {
    let mut deleted = Vec::new();
    for (path, target) in validated {
        match remove_clip_files(&target) {
            Ok(_) => deleted.push(path),
            Err(e) => failed.push((path, e.to_string())),
        }
    }
    DeletedClipsReport { deleted, failed }
}

/// Delete many clips in one round trip. Validation runs up front while the
/// `StorageSettings` borrow is live; owned `PathBuf`s then move into a single
/// blocking task so the UI does not pay N async hops.
#[tauri::command]
pub async fn delete_clips(
    paths: Vec<String>,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<DeletedClipsReport, String> {
    let mut validated: Vec<(String, PathBuf)> = Vec::with_capacity(paths.len());
    let mut failed: Vec<(String, String)> = Vec::new();
    for path in paths {
        match validate_clip_path(&settings, &path) {
            Ok(target) => validated.push((path, target)),
            Err(e) => failed.push((path, e)),
        }
    }
    let result = tauri::async_runtime::spawn_blocking(move || delete_clips_impl(validated, failed))
        .await
        .map_err(|e| format!("delete clips task: {e}"))?;
    Ok(result)
}

#[tauri::command]
pub async fn rename_clip(
    path: String,
    name: String,
    settings: tauri::State<'_, StorageSettings>,
    state: tauri::State<'_, crate::app::RuntimeState>,
) -> Result<RenamedClipInfo, String> {
    let source = validate_clip_path(&settings, &path)?;
    let target_name = normalized_clip_file_name(&name)?;
    let old_path = path.clone();
    let renamed = tauri::async_runtime::spawn_blocking(move || {
        rename_clip_files(source, old_path, target_name)
    })
    .await
    .map_err(|e| format!("rename clip task: {e}"))??;

    update_cloud_record_paths(&state, &path, &renamed.path);
    Ok(renamed)
}

fn rename_clip_files(
    source: PathBuf,
    old_path: String,
    target_name: String,
) -> Result<RenamedClipInfo, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "clip has no containing folder".to_string())?;
    let target = parent.join(&target_name);

    let target_is_same_file = target
        .canonicalize()
        .is_ok_and(|candidate| candidate == source);
    if target.exists() && !target_is_same_file {
        return Err("a clip with that name already exists".into());
    }

    let source_markers = source.with_extension("markers.json");
    let target_markers = target.with_extension("markers.json");
    let target_markers_same_file = target_markers
        .canonicalize()
        .is_ok_and(|candidate| candidate == source_markers);
    if source_markers.exists() && target_markers.exists() && !target_markers_same_file {
        return Err("a marker sidecar with that name already exists".into());
    }

    if source != target {
        std::fs::rename(&source, &target).map_err(|e| format!("rename clip: {e}"))?;
    }
    if source_markers.exists() && source_markers != target_markers {
        if let Err(error) = std::fs::rename(&source_markers, &target_markers) {
            let _ = std::fs::rename(&target, &source);
            return Err(format!("rename clip markers: {error}"));
        }
    }

    // The poster is a regenerable cache, not user data: move it alongside the
    // clip when we can, otherwise drop the stale one so it rebuilds on demand.
    let source_poster = crate::poster::poster_path(&source);
    if source_poster.exists() {
        let target_poster = crate::poster::poster_path(&target);
        if source_poster != target_poster
            && std::fs::rename(&source_poster, &target_poster).is_err()
        {
            let _ = std::fs::remove_file(&source_poster);
        }
    }

    let new_path = display_renamed_clip_path(&old_path, &target_name, parent);
    Ok(RenamedClipInfo {
        old_path,
        path: new_path,
        name: target_name,
    })
}

#[tauri::command]
pub async fn export_clip(
    path: String,
    start_s: f64,
    end_s: f64,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<ExportedClipInfo, String> {
    let source = validate_clip_path(&settings, &path)?;
    tauri::async_runtime::spawn_blocking(move || export_clip_file(source, start_s, end_s))
        .await
        .map_err(|e| format!("export clip task: {e}"))?
}

#[tauri::command]
pub async fn preview_clip_audio_tracks<R: Runtime>(
    app: AppHandle<R>,
    request: AudioPreviewRequest,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<String, String> {
    let source = validate_clip_path(&settings, &request.path)?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        preview_clip_audio_tracks_file(source, request.path, request.audio_track_ids)
    })
    .await
    .map_err(|e| format!("audio preview task: {e}"))??;
    allow_audio_preview_asset(&app, Path::new(&path))?;
    Ok(path)
}

fn allow_audio_preview_asset<R: Runtime>(app: &AppHandle<R>, preview: &Path) -> Result<(), String> {
    let preview_dir = crate::settings::audio_preview_cache_dir();
    if !preview.starts_with(&preview_dir) {
        return Ok(());
    }
    let canonical_dir = std::fs::canonicalize(&preview_dir)
        .map_err(|e| format!("canonicalize audio preview cache {preview_dir:?}: {e}"))?;
    let canonical_preview = std::fs::canonicalize(preview)
        .map_err(|e| format!("canonicalize audio preview {preview:?}: {e}"))?;
    if !canonical_preview.starts_with(&canonical_dir) {
        return Err(format!(
            "audio preview {canonical_preview:?} escaped cache {canonical_dir:?}"
        ));
    }
    if !canonical_preview
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    {
        return Err(format!("audio preview {canonical_preview:?} is not an MP4"));
    }

    let preview = canonical_preview.as_path();
    app.asset_protocol_scope()
        .allow_file(preview)
        .map_err(|e| format!("scope audio preview {canonical_preview:?} for playback: {e}"))
}

fn preview_clip_audio_tracks_file(
    source: PathBuf,
    display_path: String,
    selected_audio_track_ids: Vec<String>,
) -> Result<String, String> {
    preview_clip_audio_tracks_file_with_mixer(
        source,
        display_path,
        selected_audio_track_ids,
        crate::settings::audio_preview_cache_dir(),
        mix_audio_tracks_with_ffmpeg,
    )
}

fn preview_clip_audio_tracks_file_with_mixer(
    source: PathBuf,
    display_path: String,
    selected_audio_track_ids: Vec<String>,
    preview_dir: PathBuf,
    mix_audio_preview: impl FnOnce(&Path, &Path, &[u32]) -> Result<(), String>,
) -> Result<String, String> {
    let Some(markers) =
        util::markers_with_inferred_audio_tracks(&source, util::read_markers_raw(&source))
    else {
        return Ok(display_path);
    };
    let selected_indices = util::selected_audio_track_indices(&markers, &selected_audio_track_ids)?;
    if selected_indices.len() == markers.audio_tracks.len() && selected_indices.len() <= 1 {
        return Ok(display_path);
    }

    let meta = std::fs::metadata(&source).map_err(|e| format!("read clip metadata: {e}"))?;
    std::fs::create_dir_all(&preview_dir)
        .map_err(|e| format!("create audio preview cache: {e}"))?;
    prune_old_audio_previews(&preview_dir);
    let preview = audio_preview_path(&preview_dir, &source, &meta, &selected_audio_track_ids);
    if preview.exists() {
        return Ok(preview.display().to_string());
    }

    let source_bytes = std::fs::read(&source).map_err(|e| format!("read clip: {e}"))?;
    if selected_indices.len() > 1 {
        match remux_with_mixed_audio_track(&source_bytes, &selected_indices) {
            Ok(preview_bytes) => {
                write_audio_preview(&preview, preview_bytes)?;
                return Ok(preview.display().to_string());
            }
            Err(native_error) => {
                if let Err(external_error) = mix_audio_preview(&source, &preview, &selected_indices)
                {
                    return Err(format!(
                        "{external_error}; native audio mix failed: {native_error}"
                    ));
                }
                return Ok(preview.display().to_string());
            }
        }
    }

    let preview_bytes = remux_with_selected_audio_tracks(&source_bytes, &selected_indices)
        .map_err(|e| e.to_string())?;
    write_audio_preview(&preview, preview_bytes)?;
    Ok(preview.display().to_string())
}

fn write_audio_preview(preview: &Path, preview_bytes: Vec<u8>) -> Result<(), String> {
    let tmp = cached_export_tmp_path(preview)?;
    std::fs::write(&tmp, preview_bytes).map_err(|e| format!("write audio preview: {e}"))?;
    match std::fs::rename(&tmp, preview) {
        Ok(()) => {}
        Err(_) if preview.exists() => {
            let _ = std::fs::remove_file(&tmp);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("finalize audio preview: {e}"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ShareAudioExportMode {
    Remux(Vec<u32>),
    Mix(Vec<u32>),
}

fn clipboard_share_path(
    source: &Path,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<PathBuf, String> {
    clipboard_share_path_with_exporter(
        source,
        selected_audio_track_ids,
        &crate::settings::share_export_cache_dir(),
        |source, mode| {
            let source_bytes = std::fs::read(source).map_err(|e| format!("read clip: {e}"))?;
            match mode {
                ShareAudioExportMode::Remux(indices) => {
                    remux_with_selected_audio_tracks(&source_bytes, &indices)
                        .map_err(|e| e.to_string())
                }
                ShareAudioExportMode::Mix(indices) => {
                    remux_with_mixed_audio_track(&source_bytes, &indices).map_err(|e| e.to_string())
                }
            }
        },
    )
}

fn clipboard_share_path_with_exporter(
    source: &Path,
    selected_audio_track_ids: Option<&[String]>,
    export_dir: &Path,
    mut export_audio: impl FnMut(&Path, ShareAudioExportMode) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let Some(mode) = clipboard_share_export_mode(source, selected_audio_track_ids)? else {
        return Ok(source.to_path_buf());
    };

    let meta = std::fs::metadata(source).map_err(|e| format!("read clip metadata: {e}"))?;
    std::fs::create_dir_all(export_dir).map_err(|e| format!("create share export cache: {e}"))?;
    prune_old_share_exports(export_dir);
    let export = share_export_path(export_dir, source, &meta, selected_audio_track_ids, &mode);
    if export.exists() {
        return Ok(export);
    }

    let bytes = export_audio(source, mode)?;
    let tmp = share_export_tmp_path(&export)?;
    std::fs::write(&tmp, bytes).map_err(|e| format!("write share export: {e}"))?;
    match std::fs::rename(&tmp, &export) {
        Ok(()) => {}
        Err(_) if export.exists() => {
            let _ = std::fs::remove_file(&tmp);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("finalize share export: {e}"));
        }
    }
    Ok(export)
}

fn clipboard_share_export_mode(
    source: &Path,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<Option<ShareAudioExportMode>, String> {
    let Some(selected_audio_track_ids) = selected_audio_track_ids else {
        return Ok(None);
    };
    let Some(markers) =
        util::markers_with_inferred_audio_tracks(source, util::read_markers_raw(source))
    else {
        return Ok(None);
    };
    let tracks = markers.audio_tracks.as_slice();
    if tracks.is_empty() {
        if selected_audio_track_ids.is_empty() {
            return Ok(Some(ShareAudioExportMode::Remux(Vec::new())));
        }
        return Err("this clip has no selectable audio track metadata".into());
    }
    let selected_indices = util::selected_audio_track_indices(&markers, selected_audio_track_ids)?;
    if selected_indices.len() > 1 {
        Ok(Some(ShareAudioExportMode::Mix(selected_indices)))
    } else {
        Ok(Some(ShareAudioExportMode::Remux(selected_indices)))
    }
}

fn share_export_path(
    export_dir: &Path,
    source: &Path,
    meta: &std::fs::Metadata,
    selected_audio_track_ids: Option<&[String]>,
    mode: &ShareAudioExportMode,
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    "share-export-v1".hash(&mut hasher);
    source.display().to_string().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    mode.hash(&mut hasher);
    if let Some(ids) = selected_audio_track_ids {
        for id in ids {
            id.hash(&mut hasher);
        }
    }
    export_dir.join(format!("share-export-{:016x}.mp4", hasher.finish()))
}

fn share_export_tmp_path(export: &Path) -> Result<PathBuf, String> {
    cached_export_tmp_path(export)
}

fn cached_export_tmp_path(target: &Path) -> Result<PathBuf, String> {
    crate::settings::persistence::sibling_tmp_path(target)
}

fn prune_old_share_exports(export_dir: &Path) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    prune_cached_mp4_files(export_dir, MAX_AGE);
}

fn prune_cached_mp4_files(export_dir: &Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(export_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_cached_mp4_file(&path) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn is_cached_mp4_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("mp4")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".mp4.tmp"))
}

pub(crate) fn mix_audio_tracks_with_ffmpeg(
    source: &Path,
    output_path: &Path,
    selected_audio_track_indices: &[u32],
) -> Result<(), String> {
    let ffmpeg = clipline_capture::ffmpeg::locate()
        .ok_or_else(|| "ffmpeg is not available for audio track mixing".to_string())?;
    let filter = ffmpeg_audio_mix_filter(selected_audio_track_indices)?;
    let tmp = cached_export_tmp_path(output_path)?;
    let _ = std::fs::remove_file(&tmp);

    let mut cmd = Command::new(ffmpeg);
    suppress_console(&mut cmd);
    let output = cmd
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-fflags",
            "+bitexact",
            "-i",
        ])
        .arg(source)
        .args(["-filter_complex", &filter])
        .args([
            "-map",
            "0:v:0",
            "-map",
            "[aout]",
            "-map_metadata",
            "-1",
            "-c:v",
            "copy",
            "-c:a",
            "libopus",
            "-b:a",
            "160k",
            "-fflags",
            "+bitexact",
            "-flags",
            "+bitexact",
            "-bitexact",
            "-f",
            "mp4",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg audio mix: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg audio mix failed: {stderr}"));
    }
    match std::fs::rename(&tmp, output_path) {
        Ok(()) => Ok(()),
        Err(_) if output_path.exists() => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("finalize audio mix: {e}"))
        }
    }
}

fn ffmpeg_audio_mix_filter(selected_audio_track_indices: &[u32]) -> Result<String, String> {
    if selected_audio_track_indices.is_empty() {
        return Err("audio mix requires at least one audio stream".into());
    }
    let mut filter = String::new();
    for index in selected_audio_track_indices {
        filter.push_str(&format!("[0:a:{index}]"));
    }
    filter.push_str(&format!(
        "amix=inputs={}:duration=longest:normalize=0[aout]",
        selected_audio_track_indices.len()
    ));
    Ok(filter)
}

pub(crate) use clipline_capture::ffmpeg::suppress_console;

fn export_clip_file(source: PathBuf, start_s: f64, end_s: f64) -> Result<ExportedClipInfo, String> {
    let tmp = unique_temp_export_path(&source)?;
    let info = match trim_keyframe_aligned_file(&source, &tmp, start_s, end_s) {
        Ok(info) => info,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    };
    let target = unique_export_path(&source, info.aligned_start_s, info.aligned_end_s)?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;

    let mut exported_markers = None;
    if let Some(markers) = util::read_markers_raw(&source).map(filter_review_markers) {
        let cropped = crop_markers(&markers, info.aligned_start_s, info.aligned_end_s);
        if has_marker_sidecar_content(&cropped) {
            let json = serde_json::to_string_pretty(&cropped).map_err(|e| e.to_string())?;
            std::fs::write(target.with_extension("markers.json"), json)
                .map_err(|e| e.to_string())?;
            exported_markers = Some(cropped);
        }
    }
    let meta =
        std::fs::metadata(&target).map_err(|e| format!("read exported clip metadata: {e}"))?;
    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(ExportedClipInfo {
        path: target.display().to_string(),
        name: target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size_mb: meta.len() as f64 / (1024.0 * 1024.0),
        modified_unix,
        requested_start_s: info.requested_start_s,
        requested_end_s: info.requested_end_s,
        aligned_start_s: info.aligned_start_s,
        aligned_end_s: info.aligned_end_s,
        duration_s: info.duration_s,
        markers: exported_markers,
    })
}

fn audio_preview_path(
    preview_dir: &Path,
    source: &Path,
    meta: &std::fs::Metadata,
    selected_audio_track_ids: &[String],
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    "audio-preview-mix-v4".hash(&mut hasher);
    source.display().to_string().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified().ok().hash(&mut hasher);
    for id in selected_audio_track_ids {
        id.hash(&mut hasher);
    }
    preview_dir.join(format!("audio-preview-{:016x}.mp4", hasher.finish()))
}

fn prune_old_audio_previews(preview_dir: &Path) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    prune_cached_mp4_files(preview_dir, MAX_AGE);
}

#[tauri::command]
pub async fn storage_status(
    settings: tauri::State<'_, StorageSettings>,
) -> Result<StorageInfo, String> {
    let dir = settings.clips_dir()?;
    let quota_bytes = settings.quota_bytes();
    tauri::async_runtime::spawn_blocking(move || storage_status_for_dir(dir, quota_bytes))
        .await
        .map_err(|e| format!("storage status task: {e}"))?
}

fn storage_status_for_dir(dir: PathBuf, quota_bytes: Option<u64>) -> Result<StorageInfo, String> {
    let status = read_storage_status(&dir, quota_bytes).map_err(|e| e.to_string())?;
    Ok(StorageInfo {
        clip_count: status.clip_count,
        total_bytes: status.total_bytes,
        quota_bytes: status.quota_bytes,
        over_quota: status.is_over_quota(),
    })
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
    // Legacy clips sit at the root; session clips one folder down.
    let parent_ok = target.parent() == Some(dir.as_path())
        || target.parent().and_then(Path::parent) == Some(dir.as_path());
    if !parent_ok || target.extension().and_then(|e| e.to_str()) != Some("mp4") {
        return Err("refusing to access a clip outside the clips directory".into());
    }
    Ok(target)
}

#[tauri::command]
pub fn reveal_clip(path: String, settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    let dir = target
        .parent()
        .ok_or_else(|| "clip has no containing folder".to_string())?;
    open_folder_path(dir)
}

#[tauri::command]
pub async fn copy_clip_to_clipboard(
    request: CopyClipToClipboardRequest,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<(), String> {
    let target = validate_clip_path(&settings, &request.path)?;
    let audio_track_ids = request.audio_track_ids;
    tauri::async_runtime::spawn_blocking(move || {
        let share_path = clipboard_share_path(&target, audio_track_ids.as_deref())?;
        copy_file_to_clipboard(&share_path)
    })
    .await
    .map_err(|e| format!("copy clip task: {e}"))?
}

#[tauri::command]
pub fn open_media_folder(settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let dir = settings.clips_dir()?;
    open_folder_path(&dir)
}

fn open_folder_path(dir: &Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .spawn()
        .map_err(|e| format!("open explorer: {e}"))?;
    Ok(())
}

fn normalized_clip_file_name(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("clip name cannot be empty".into());
    }
    if trimmed.contains(['/', '\\']) {
        return Err("clip name cannot contain folders".into());
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_ascii_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("clip name contains a character Windows cannot use in filenames".into());
    }

    let suffix = trimmed
        .get(trimmed.len().saturating_sub(4)..)
        .filter(|suffix| suffix.eq_ignore_ascii_case(".mp4"));
    let stem = if suffix.is_some() {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    }
    .trim();
    if stem.is_empty() || stem == "." || stem == ".." {
        return Err("clip name cannot be empty".into());
    }
    if stem.ends_with(['.', ' ']) {
        return Err("clip name cannot end with a dot or space".into());
    }
    if is_reserved_windows_file_name(stem) {
        return Err("clip name is reserved by Windows".into());
    }
    Ok(format!("{stem}.mp4"))
}

fn is_reserved_windows_file_name(stem: &str) -> bool {
    let base = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

fn display_renamed_clip_path(old_path: &str, name: &str, fallback_parent: &Path) -> String {
    Path::new(old_path)
        .parent()
        .map(|parent| parent.join(name))
        .unwrap_or_else(|| fallback_parent.join(name))
        .display()
        .to_string()
}

fn update_cloud_record_paths(state: &crate::app::RuntimeState, old_path: &str, new_path: &str) {
    if old_path == new_path {
        return;
    }
    if let Err(error) = state.update_cloud(|cloud| {
        for record in cloud.uploads.values_mut() {
            if record.path == old_path {
                record.path = new_path.to_string();
            }
        }
    }) {
        eprintln!("update cloud records after rename: {error}");
    }
}

fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    let payload = dropfiles_payload(path);
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, payload.len()) };
    if handle.is_null() {
        return Err(last_os_error("allocate clipboard memory"));
    }

    let mem = unsafe { GlobalLock(handle) };
    if mem.is_null() {
        let err = last_os_error("lock clipboard memory");
        unsafe {
            GlobalFree(handle);
        }
        return Err(err);
    }
    unsafe {
        ptr::copy_nonoverlapping(payload.as_ptr(), mem.cast::<u8>(), payload.len());
        GlobalUnlock(handle);
    }

    let mut transfer = ClipboardTransfer::new(handle);
    if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
        return Err(last_os_error("open clipboard"));
    }
    let _close = ClipboardClose;
    // CF_HDROP can be replaced format-by-format. Avoid EmptyClipboard so a
    // rare SetClipboardData failure does not discard the user's clipboard.
    if unsafe { SetClipboardData(CF_HDROP as u32, transfer.handle()) }.is_null() {
        return Err(last_os_error("set clipboard data"));
    }
    transfer.release();
    Ok(())
}

fn dropfiles_payload(path: &Path) -> Vec<u8> {
    let mut wide = shell_clipboard_path_wide(path);
    wide.extend([0, 0]);

    let header_len = size_of::<DROPFILES>();
    let byte_len = header_len + wide.len() * size_of::<u16>();
    let mut payload = vec![0u8; byte_len];
    let header = DROPFILES {
        pFiles: header_len as u32,
        pt: Default::default(),
        fNC: 0,
        fWide: 1,
    };
    unsafe {
        ptr::write_unaligned(payload.as_mut_ptr().cast::<DROPFILES>(), header);
        ptr::copy_nonoverlapping(
            wide.as_ptr().cast::<u8>(),
            payload.as_mut_ptr().add(header_len),
            wide.len() * size_of::<u16>(),
        );
    }
    payload
}

fn shell_clipboard_path_wide(path: &Path) -> Vec<u16> {
    const BACKSLASH: u16 = b'\\' as u16;
    const QUESTION: u16 = b'?' as u16;
    const U: u16 = b'U' as u16;
    const N: u16 = b'N' as u16;
    const C: u16 = b'C' as u16;
    const VERBATIM: [u16; 4] = [BACKSLASH, BACKSLASH, QUESTION, BACKSLASH];
    const VERBATIM_UNC: [u16; 8] = [
        BACKSLASH, BACKSLASH, QUESTION, BACKSLASH, U, N, C, BACKSLASH,
    ];

    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.starts_with(&VERBATIM_UNC) {
        let mut plain = vec![BACKSLASH, BACKSLASH];
        plain.extend_from_slice(&wide[VERBATIM_UNC.len()..]);
        plain
    } else if wide.starts_with(&VERBATIM) {
        wide[VERBATIM.len()..].to_vec()
    } else {
        wide
    }
}

struct ClipboardTransfer {
    handle: HGLOBAL,
}

impl ClipboardTransfer {
    fn new(handle: HGLOBAL) -> Self {
        Self { handle }
    }

    fn handle(&self) -> HANDLE {
        self.handle
    }

    fn release(&mut self) {
        self.handle = ptr::null_mut();
    }
}

impl Drop for ClipboardTransfer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                GlobalFree(self.handle);
            }
        }
    }
}

struct ClipboardClose;

impl Drop for ClipboardClose {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn filter_review_markers(mut markers: ClipMarkers) -> ClipMarkers {
    markers.markers.retain(|m| is_review_event(&m.event));
    markers
}

fn has_marker_sidecar_content(markers: &ClipMarkers) -> bool {
    !markers.markers.is_empty()
        || markers.player_summary.is_some()
        || !markers.audio_tracks.is_empty()
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
        player_summary: markers.player_summary.clone(),
        audio_tracks: markers.audio_tracks.clone(),
        markers: cropped,
    }
}

fn unique_temp_export_path(source: &Path) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .ok_or_else(|| "source clip has no parent directory".to_string())?;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy())
        .ok_or_else(|| "source clip has no file stem".to_string())?;
    for suffix in 0..1000u32 {
        let name = format!("{stem}_trim_pending_{suffix:03}.mp4.tmp");
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("could not choose an unused temporary export filename".into())
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
    use std::io::Cursor;

    use audiopus::coder::Encoder;
    use audiopus::{Application, Channels, SampleRate};
    use clipline_events::{ClipAudioTrack, EventKind, GameEvent, GameId, PlayerSummary};
    use clipline_mp4::{
        AudioTrackConfig, FragSample, HybridMp4Writer, TrackConfig, VideoTrackConfig,
    };
    use clipline_test_utils::TestDir;

    fn marker(t_s: f64) -> ClipMarker {
        marker_with(t_s, EventKind::ChampionKill, true)
    }

    fn marker_with(t_s: f64, kind: EventKind, involves_local_player: bool) -> ClipMarker {
        ClipMarker {
            t_s,
            event: GameEvent {
                game_id: GameId::LeagueOfLegends,
                kind,
                actor: "Dain".into(),
                victim: None,
                assisters: Vec::new(),
                subtype: None,
                game_time_s: 0.0,
                recording_offset_s: Some(10.0 + t_s),
                importance: 7,
                involves_local_player,
            },
        }
    }

    #[test]
    fn crop_markers_rebases_times_and_recording_start() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 5.0,
            player_summary: Some(PlayerSummary {
                champion_name: "Nautilus".into(),
                kills: 3,
                deaths: 4,
                assists: 23,
                creep_score: None,
                game_time_s: None,
                player_name: String::new(),
                team: String::new(),
                participants: Vec::new(),
                summoner_spells: Vec::new(),
                items: Vec::new(),
            }),
            audio_tracks: Vec::new(),
            markers: vec![marker(0.5), marker(1.5), marker(2.5)],
        };

        let cropped = crop_markers(&markers, 1.0, 2.0);

        assert_eq!(cropped.markers.len(), 1);
        assert!((cropped.markers[0].t_s - 0.5).abs() < 1e-9);
        assert!((cropped.recording_start_s - 11.0).abs() < 1e-9);
        assert!((cropped.duration_s - 1.0).abs() < 1e-9);
        assert_eq!(
            cropped.player_summary.as_ref().map(|summary| (
                summary.champion_name.as_str(),
                summary.kills,
                summary.deaths,
                summary.assists
            )),
            Some(("Nautilus", 3, 4, 23))
        );
    }

    #[test]
    fn filter_review_markers_keeps_match_event_sources_and_drops_noise() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 100.0,
            player_summary: Some(PlayerSummary {
                champion_name: "Nautilus".into(),
                kills: 3,
                deaths: 4,
                assists: 23,
                creep_score: None,
                game_time_s: None,
                player_name: String::new(),
                team: String::new(),
                participants: Vec::new(),
                summoner_spells: Vec::new(),
                items: Vec::new(),
            }),
            audio_tracks: Vec::new(),
            markers: vec![
                marker_with(1.0, EventKind::ChampionKill, true),
                marker_with(2.0, EventKind::ChampionKill, false),
                marker_with(2.5, EventKind::ChampionDeath, true),
                marker_with(3.0, EventKind::TurretKilled, false),
                marker_with(4.0, EventKind::DragonKill, false),
                marker_with(5.0, EventKind::BaronKill, false),
                marker_with(5.5, EventKind::HeraldKill, false),
                marker_with(6.0, EventKind::MinionsSpawning, true),
                marker_with(7.0, EventKind::FirstBlood, true),
                marker_with(8.0, EventKind::FirstBrick, true),
                marker_with(9.0, EventKind::Ace, true),
            ],
        };

        let filtered = filter_review_markers(markers);
        let kinds: Vec<_> = filtered.markers.iter().map(|m| m.event.kind).collect();

        assert_eq!(
            kinds,
            vec![
                EventKind::ChampionKill,
                EventKind::ChampionKill,
                EventKind::ChampionDeath,
                EventKind::TurretKilled,
                EventKind::DragonKill,
                EventKind::BaronKill,
                EventKind::HeraldKill,
            ]
        );
        assert!(filtered.markers[0].event.involves_local_player);
        assert!(!filtered.markers[1].event.involves_local_player);
        assert_eq!(
            filtered.player_summary.as_ref().map(|summary| (
                summary.champion_name.as_str(),
                summary.kills,
                summary.deaths,
                summary.assists
            )),
            Some(("Nautilus", 3, 4, 23))
        );
    }

    #[test]
    fn summary_only_markers_are_export_sidecar_content() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 20.0,
            player_summary: Some(PlayerSummary {
                champion_name: "Nautilus".into(),
                kills: 3,
                deaths: 4,
                assists: 23,
                creep_score: None,
                game_time_s: None,
                player_name: String::new(),
                team: String::new(),
                participants: Vec::new(),
                summoner_spells: Vec::new(),
                items: Vec::new(),
            }),
            audio_tracks: Vec::new(),
            markers: Vec::new(),
        };

        assert!(has_marker_sidecar_content(&markers));
    }

    #[test]
    fn empty_markers_are_not_export_sidecar_content() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 20.0,
            player_summary: None,
            audio_tracks: Vec::new(),
            markers: Vec::new(),
        };

        assert!(!has_marker_sidecar_content(&markers));
    }

    #[test]
    fn audio_tracks_are_export_sidecar_content_and_survive_cropping() {
        let tracks = vec![ClipAudioTrack {
            id: "microphone".into(),
            track_index: 1,
            label: "Microphone".into(),
            kind: Some("microphone".into()),
        }];
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 20.0,
            player_summary: None,
            audio_tracks: tracks.clone(),
            markers: Vec::new(),
        };

        assert!(has_marker_sidecar_content(&markers));
        let cropped = crop_markers(&markers, 3.0, 7.0);

        assert_eq!(cropped.audio_tracks, tracks);
        assert_eq!(cropped.markers.len(), 0);
        assert!((cropped.duration_s - 4.0).abs() < 1e-9);
    }

    #[test]
    fn ffmpeg_audio_mix_filter_targets_selected_audio_streams() {
        let filter = ffmpeg_audio_mix_filter(&[0, 2, 5]).unwrap();

        assert_eq!(
            filter,
            "[0:a:0][0:a:2][0:a:5]amix=inputs=3:duration=longest:normalize=0[aout]"
        );
    }

    #[test]
    fn ffmpeg_audio_mix_filter_requires_at_least_one_stream() {
        let err = ffmpeg_audio_mix_filter(&[]).expect_err("empty selection is invalid");

        assert!(err.contains("at least one"), "{err}");
    }

    #[test]
    fn audio_mix_ffmpeg_command_requests_deterministic_mp4_output() {
        let source = include_str!("library.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source before tests");

        assert!(production_source.contains("\"-fflags\""));
        assert!(production_source.contains("\"+bitexact\""));
        assert!(production_source.contains("\"-map_metadata\""));
        assert!(production_source.contains("\"-flags\""));
        assert!(production_source.contains("\"-bitexact\""));
    }

    #[test]
    fn all_audio_tracks_selected_mixes_preview_when_source_has_multiple_tracks() {
        let dir = TestDir::new("clipline-library", "audio-preview-all-mixed");
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"not an mp4").unwrap();
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 10.0,
            player_summary: None,
            audio_tracks: vec![
                ClipAudioTrack {
                    id: "output".into(),
                    track_index: 0,
                    label: "Output Audio".into(),
                    kind: Some("output".into()),
                },
                ClipAudioTrack {
                    id: "process:1".into(),
                    track_index: 1,
                    label: "Game".into(),
                    kind: Some("process_output".into()),
                },
                ClipAudioTrack {
                    id: "microphone".into(),
                    track_index: 2,
                    label: "Microphone".into(),
                    kind: Some("microphone".into()),
                },
            ],
            markers: Vec::new(),
        };
        std::fs::write(
            source.with_extension("markers.json"),
            serde_json::to_string(&markers).unwrap(),
        )
        .unwrap();

        let preview_dir = dir.path().join("previews");
        let mixed_preview = std::cell::RefCell::new(None);
        let source_for_mixer = source.clone();
        let path = preview_clip_audio_tracks_file_with_mixer(
            source.clone(),
            source.display().to_string(),
            vec!["output".into(), "process:1".into(), "microphone".into()],
            preview_dir.clone(),
            |input, output, selected| {
                assert_eq!(input, source_for_mixer.as_path());
                assert_eq!(selected, &[0, 1, 2]);
                assert_eq!(output.parent(), Some(preview_dir.as_path()));
                mixed_preview.replace(Some(output.to_path_buf()));
                Ok(())
            },
        )
        .expect("all selected should get an audible preview mix");

        assert_eq!(
            path,
            mixed_preview
                .borrow()
                .as_ref()
                .expect("mixer output path captured")
                .display()
                .to_string()
        );
    }

    #[test]
    fn partial_multi_track_preview_returns_mix_failure_instead_of_unmixed_mp4() {
        let dir = TestDir::new("clipline-library", "audio-preview-mix-failure");
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"not an mp4").unwrap();
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 10.0,
            player_summary: None,
            audio_tracks: vec![
                ClipAudioTrack {
                    id: "output".into(),
                    track_index: 0,
                    label: "Output Audio".into(),
                    kind: Some("output".into()),
                },
                ClipAudioTrack {
                    id: "process:1".into(),
                    track_index: 1,
                    label: "Game".into(),
                    kind: Some("process_output".into()),
                },
                ClipAudioTrack {
                    id: "microphone".into(),
                    track_index: 2,
                    label: "Microphone".into(),
                    kind: Some("microphone".into()),
                },
            ],
            markers: Vec::new(),
        };
        std::fs::write(
            source.with_extension("markers.json"),
            serde_json::to_string(&markers).unwrap(),
        )
        .unwrap();

        let err = preview_clip_audio_tracks_file_with_mixer(
            source.clone(),
            source.display().to_string(),
            vec!["process:1".into(), "microphone".into()],
            dir.path().join("previews"),
            |_, _, _| Err("forced mix failure".into()),
        )
        .expect_err("multi-track preview must require a mixed preview");

        assert!(err.contains("forced mix failure"), "{err}");
    }

    #[test]
    fn multi_track_review_preview_uses_native_mixer_without_ffmpeg() {
        let dir = TestDir::new("clipline-library", "audio-preview-native-mix");
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, two_real_opus_audio_mp4()).unwrap();
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 1.0,
            player_summary: None,
            audio_tracks: vec![
                ClipAudioTrack {
                    id: "output".into(),
                    track_index: 0,
                    label: "Output Audio".into(),
                    kind: Some("output".into()),
                },
                ClipAudioTrack {
                    id: "microphone".into(),
                    track_index: 1,
                    label: "Microphone".into(),
                    kind: Some("microphone".into()),
                },
            ],
            markers: Vec::new(),
        };
        std::fs::write(
            source.with_extension("markers.json"),
            serde_json::to_string(&markers).unwrap(),
        )
        .unwrap();

        let preview_dir = dir.path().join("previews");
        let display_path = source.display().to_string();
        let path = preview_clip_audio_tracks_file_with_mixer(
            source,
            display_path,
            vec!["output".into(), "microphone".into()],
            preview_dir.clone(),
            |_, _, _| Err("external mixer should not be required".into()),
        )
        .expect("Clipline-authored output+mic preview should use the native mixer");
        let preview = PathBuf::from(path);

        assert_eq!(preview.parent(), Some(preview_dir.as_path()));
        let preview_bytes = std::fs::read(preview).unwrap();
        remux_with_selected_audio_tracks(&preview_bytes, &[0]).expect("mixed audio track exists");
        let err = remux_with_selected_audio_tracks(&preview_bytes, &[1])
            .expect_err("mixed preview should have exactly one audio track");
        assert!(
            err.to_string()
                .contains("outside the clip's 1 audio tracks"),
            "{err}"
        );
    }

    #[test]
    fn selected_audio_track_indices_follow_sidecar_order_and_reject_unknown_ids() {
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 10.0,
            player_summary: None,
            audio_tracks: vec![
                ClipAudioTrack {
                    id: "output".into(),
                    track_index: 0,
                    label: "Output Audio".into(),
                    kind: Some("output".into()),
                },
                ClipAudioTrack {
                    id: "microphone".into(),
                    track_index: 1,
                    label: "Microphone".into(),
                    kind: Some("microphone".into()),
                },
            ],
            markers: Vec::new(),
        };

        assert_eq!(
            util::selected_audio_track_indices(&markers, &["microphone".into()]).unwrap(),
            vec![1]
        );
        assert_eq!(
            util::selected_audio_track_indices(&markers, &["microphone".into(), "output".into()])
                .unwrap(),
            vec![0, 1]
        );

        let err = util::selected_audio_track_indices(&markers, &["discord".into()]).unwrap_err();
        assert!(err.contains("unknown audio track"), "{err}");
    }

    #[test]
    fn clipboard_share_export_mixes_multiple_selected_tracks() {
        let dir = TestDir::new("clipline-library", "clipboard-share-mix");
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"source mp4").unwrap();
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 10.0,
            player_summary: None,
            audio_tracks: vec![
                ClipAudioTrack {
                    id: "output".into(),
                    track_index: 0,
                    label: "Output Audio".into(),
                    kind: Some("output".into()),
                },
                ClipAudioTrack {
                    id: "microphone".into(),
                    track_index: 1,
                    label: "Microphone".into(),
                    kind: Some("microphone".into()),
                },
            ],
            markers: Vec::new(),
        };
        std::fs::write(
            source.with_extension("markers.json"),
            serde_json::to_string(&markers).unwrap(),
        )
        .unwrap();

        let selected = vec!["output".to_string(), "microphone".to_string()];
        let export_dir = dir.path().join("share-exports");
        let exported = clipboard_share_path_with_exporter(
            &source,
            Some(&selected),
            &export_dir,
            |input, mode| {
                assert_eq!(input, source.as_path());
                assert_eq!(mode, ShareAudioExportMode::Mix(vec![0, 1]));
                Ok(b"mixed share mp4".to_vec())
            },
        )
        .unwrap();

        assert!(exported.starts_with(&export_dir));
        assert_eq!(std::fs::read(exported).unwrap(), b"mixed share mp4");
    }

    #[test]
    fn clipboard_share_without_audio_selection_uses_original_path() {
        let dir = TestDir::new("clipline-library", "clipboard-share-original");
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"source mp4").unwrap();

        let selected = None::<&[String]>;
        let chosen = clipboard_share_path_with_exporter(
            &source,
            selected,
            &dir.path().join("share"),
            |_, _| panic!("clipboard copy without explicit audio selection must not export"),
        )
        .unwrap();

        assert_eq!(chosen, source);
    }

    #[test]
    fn share_export_tmp_path_is_unique_per_writer() {
        let dir = TestDir::new("clipline-library", "share-export-temp");
        let export = dir.path().join("share-export-abc.mp4");

        let first = share_export_tmp_path(&export).unwrap();
        let second = share_export_tmp_path(&export).unwrap();

        assert_ne!(first, second);
        assert_ne!(first, export.with_extension("mp4.tmp"));
        assert_eq!(first.parent(), export.parent());
    }

    #[test]
    fn share_export_prune_removes_orphaned_tmp_files() {
        let dir = TestDir::new("clipline-library", "share-export-prune-tmp");
        let export = dir.path().join("share-export-old.mp4");
        let orphan = dir.path().join("share-export-old.mp4.tmp");
        std::fs::write(&export, b"old export").unwrap();
        std::fs::write(&orphan, b"orphan").unwrap();

        prune_cached_mp4_files(dir.path(), std::time::Duration::ZERO);

        assert!(!export.exists());
        assert!(!orphan.exists());
    }

    #[test]
    fn unique_export_path_appends_suffix_when_needed() {
        let dir = TestDir::new("clipline-library", "export-name");
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

    #[test]
    fn list_clips_uses_marker_duration_without_parsing_mp4() {
        let dir = TestDir::new("clipline-library", "list-marker-duration");
        let media = dir.path().join("media");
        let clip = media.join("broken-but-listed.mp4");
        touch_mp4(&clip);
        let markers = ClipMarkers {
            recording_start_s: 0.0,
            duration_s: 42.5,
            player_summary: None,
            audio_tracks: Vec::new(),
            markers: vec![marker(1.0)],
        };
        std::fs::write(
            clip.with_extension("markers.json"),
            serde_json::to_string(&markers).unwrap(),
        )
        .unwrap();

        let clips = list_clips_from_dir(media).unwrap();

        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].duration_s, Some(42.5));
        assert_eq!(clips[0].markers.as_ref().unwrap().markers.len(), 1);
    }

    #[test]
    fn list_clips_infers_audio_tracks_for_legacy_multitrack_clip() {
        let dir = TestDir::new("clipline-library", "list-infer-audio-tracks");
        let clip = dir.path().join("legacy.mp4");
        std::fs::write(&clip, two_real_opus_audio_mp4()).unwrap();

        let mut clips = Vec::new();
        push_clips_from(dir.path(), None, &mut clips).unwrap();

        assert_eq!(clips[0].duration_s, None);
        let tracks = &clips[0]
            .markers
            .as_ref()
            .expect("legacy clip gets inferred audio metadata")
            .audio_tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, "audio:0");
        assert_eq!(tracks[0].track_index, 0);
        assert_eq!(tracks[0].label, "Audio Track 1");
        assert_eq!(tracks[1].id, "audio:1");
        assert_eq!(tracks[1].track_index, 1);
        assert_eq!(tracks[1].label, "Audio Track 2");
    }

    #[test]
    fn legacy_multitrack_review_preview_mixes_inferred_audio_tracks() {
        let dir = TestDir::new("clipline-library", "audio-preview-legacy-inferred");
        let source = dir.path().join("legacy.mp4");
        std::fs::write(&source, two_real_opus_audio_mp4()).unwrap();

        let preview_dir = dir.path().join("previews");
        let display_path = source.display().to_string();
        let path = preview_clip_audio_tracks_file_with_mixer(
            source,
            display_path,
            vec!["audio:0".into(), "audio:1".into()],
            preview_dir.clone(),
            |_, _, _| Err("external mixer should not be required".into()),
        )
        .expect("legacy multi-audio preview should mix inferred tracks");
        let preview = PathBuf::from(path);

        assert_eq!(preview.parent(), Some(preview_dir.as_path()));
        let preview_bytes = std::fs::read(preview).unwrap();
        remux_with_selected_audio_tracks(&preview_bytes, &[0]).expect("mixed audio track exists");
        let err = remux_with_selected_audio_tracks(&preview_bytes, &[1])
            .expect_err("mixed preview should have exactly one audio track");
        assert!(
            err.to_string()
                .contains("outside the clip's 1 audio tracks"),
            "{err}"
        );
    }

    #[test]
    fn normalized_clip_file_name_adds_mp4_and_preserves_valid_text() {
        assert_eq!(
            normalized_clip_file_name("Ranked win").unwrap(),
            "Ranked win.mp4"
        );
        assert_eq!(
            normalized_clip_file_name("Ranked win.Mp4").unwrap(),
            "Ranked win.mp4"
        );
        assert_eq!(
            normalized_clip_file_name("solo.queue.vod").unwrap(),
            "solo.queue.vod.mp4"
        );
    }

    #[test]
    fn normalized_clip_file_name_rejects_paths_reserved_names_and_invalid_chars() {
        for name in [
            "",
            "..",
            "folder/clip",
            r"folder\clip",
            "bad:name",
            "clip?",
            "clip.",
            "CON",
            "LPT1.mp4",
        ] {
            assert!(
                normalized_clip_file_name(name).is_err(),
                "{name:?} should be rejected"
            );
        }
    }

    fn touch_mp4(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"\0\0\0\0").unwrap();
    }

    fn two_real_opus_audio_mp4() -> Vec<u8> {
        let tracks = vec![
            TrackConfig::Video(VideoTrackConfig::h264(
                128,
                72,
                90_000,
                vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
                vec![0x68, 0xEE, 0x38, 0x80],
            )),
            TrackConfig::Audio(AudioTrackConfig {
                channels: 2,
                sample_rate: 48_000,
                pre_skip: 312,
            }),
            TrackConfig::Audio(AudioTrackConfig {
                channels: 2,
                sample_rate: 48_000,
                pre_skip: 312,
            }),
        ];
        let mut writer = HybridMp4Writer::new_multi(Cursor::new(Vec::new()), tracks).unwrap();
        let video: Vec<_> = (0..10)
            .map(|i| FragSample {
                data: format!("V{i:05}").into_bytes(),
                duration: 9_000,
                is_sync: i == 0,
            })
            .collect();
        writer
            .write_fragment_multi(&[&video, &opus_audio_packets(0.20), &opus_audio_packets(0.25)])
            .unwrap();
        writer.finalize().unwrap().into_inner()
    }

    fn opus_audio_packets(amplitude: f32) -> Vec<FragSample> {
        let encoder =
            Encoder::new(SampleRate::Hz48000, Channels::Stereo, Application::Audio).unwrap();
        (0..50)
            .map(|frame_idx| {
                let mut pcm = Vec::with_capacity(960 * 2);
                for sample_idx in 0..960 {
                    let t = (frame_idx * 960 + sample_idx) as f32 / 48_000.0;
                    let sample = (t * 440.0 * std::f32::consts::TAU).sin() * amplitude;
                    pcm.extend([sample, sample]);
                }
                let mut encoded = vec![0u8; 4000];
                let len = encoder.encode_float(&pcm, &mut encoded).unwrap();
                encoded.truncate(len);
                FragSample {
                    data: encoded,
                    duration: 960,
                    is_sync: true,
                }
            })
            .collect()
    }

    #[test]
    fn validate_clip_path_accepts_root_and_session_clips() {
        let dir = TestDir::new("clipline-library", "validate-accept");
        let root = dir.path().join("media");
        let settings = StorageSettings::new(None, root.clone());

        let legacy = root.join("clip.mp4");
        touch_mp4(&legacy);
        let session = root.join("2026-06-12").join("clip.mp4");
        touch_mp4(&session);

        assert!(validate_clip_path(&settings, legacy.to_str().unwrap()).is_ok());
        assert!(validate_clip_path(&settings, session.to_str().unwrap()).is_ok());
    }

    #[test]
    fn validate_clip_path_rejects_escapes_and_non_mp4() {
        let dir = TestDir::new("clipline-library", "validate-reject");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        let settings = StorageSettings::new(None, root.clone());

        // Two folders below the root — deeper than a session clip.
        let too_deep = root.join("a").join("b").join("clip.mp4");
        touch_mp4(&too_deep);
        assert!(validate_clip_path(&settings, too_deep.to_str().unwrap()).is_err());

        // A sibling directory outside the configured root.
        let outside = dir.path().join("elsewhere").join("clip.mp4");
        touch_mp4(&outside);
        assert!(validate_clip_path(&settings, outside.to_str().unwrap()).is_err());

        // Correct location, wrong extension.
        let not_mp4 = root.join("clip.txt");
        touch_mp4(&not_mp4);
        assert!(validate_clip_path(&settings, not_mp4.to_str().unwrap()).is_err());
    }

    #[test]
    fn delete_clips_impl_handles_partial_success_and_sidecars() {
        let dir = TestDir::new("clipline-library", "delete-clips-impl");
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();

        // Two real clips, each with a markers sidecar and a cached poster.
        let a = root.join("a.mp4");
        let b = root.join("b.mp4");
        touch_mp4(&a);
        touch_mp4(&b);
        std::fs::write(a.with_extension("markers.json"), b"{}").unwrap();
        std::fs::write(b.with_extension("markers.json"), b"{}").unwrap();
        std::fs::write(crate::poster::poster_path(&a), b"poster").unwrap();
        std::fs::write(crate::poster::poster_path(&b), b"poster").unwrap();

        // A third clip that should be left untouched (not in the deleted set).
        let c = root.join("c.mp4");
        touch_mp4(&c);
        std::fs::write(c.with_extension("markers.json"), b"{}").unwrap();

        let validated = vec![
            (a.to_str().unwrap().to_string(), a.clone()),
            (b.to_str().unwrap().to_string(), b.clone()),
        ];
        // One path already failed validation upstream — passed through as failed.
        let failed_in = vec![("bogus".to_string(), "refused".to_string())];

        let report = delete_clips_impl(validated, failed_in);

        assert_eq!(report.deleted.len(), 2);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "bogus");
        assert!(!a.exists(), "a.mp4 should be removed");
        assert!(!b.exists(), "b.mp4 should be removed");
        assert!(
            !a.with_extension("markers.json").exists(),
            "a.mp4 markers sidecar should be removed"
        );
        assert!(
            !crate::poster::poster_path(&b).exists(),
            "b.mp4 poster should be removed"
        );
        assert!(c.exists(), "c.mp4 must be left untouched");
        assert!(
            c.with_extension("markers.json").exists(),
            "c.mp4 markers sidecar must be left untouched"
        );
    }

    #[test]
    fn dropfiles_payload_strips_verbatim_prefix_and_marks_unicode() {
        let path = Path::new(r"\\?\C:\Users\dain\Videos\Clipline\clïp 雪.mp4");
        let payload = dropfiles_payload(path);
        let p_files = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;

        assert_eq!(p_files, size_of::<DROPFILES>());
        assert_eq!(i32::from_le_bytes(payload[12..16].try_into().unwrap()), 0);
        assert_eq!(i32::from_le_bytes(payload[16..20].try_into().unwrap()), 1);

        let path_units: Vec<u16> = payload[p_files..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes(pair.try_into().unwrap()))
            .collect();
        assert_eq!(&path_units[path_units.len() - 2..], &[0, 0]);
        let decoded = String::from_utf16(&path_units[..path_units.len() - 2]).unwrap();
        assert_eq!(decoded, r"C:\Users\dain\Videos\Clipline\clïp 雪.mp4");
    }

    #[test]
    fn shell_clipboard_path_wide_converts_verbatim_unc_paths() {
        let path = Path::new(r"\\?\UNC\nas\clips\clïp 雪.mp4");
        let decoded = String::from_utf16(&shell_clipboard_path_wide(path)).unwrap();

        assert_eq!(decoded, r"\\nas\clips\clïp 雪.mp4");
    }
}
