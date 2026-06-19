//! Clip library commands: inventory of the configured media folder for the UI and
//! a path-validated delete. The webview never touches the filesystem
//! directly — playback goes through the asset protocol.

use std::collections::{hash_map::DefaultHasher, BTreeSet};
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::Mutex;

use clipline_events::{is_timeline_marker, ClipMarker, ClipMarkers, GameId};
use clipline_mp4::{remux_with_selected_audio_tracks, trim_keyframe_aligned_file};
use clipline_storage::storage_status as read_storage_status;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard, SetClipboardData};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::CF_HDROP;
use windows_sys::Win32::UI::Shell::DROPFILES;

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
        self.quota_bytes.lock().map(|q| *q).unwrap_or(None)
    }

    pub fn set_quota_bytes(&self, quota_bytes: Option<u64>) {
        if let Ok(mut q) = self.quota_bytes.lock() {
            *q = quota_bytes;
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
        let markers = read_markers(&path);
        let duration_s = markers
            .as_ref()
            .map(|markers| markers.duration_s)
            .filter(|duration| duration.is_finite() && *duration >= 0.0);
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
    let plugin_id = match game_id {
        GameId::LeagueOfLegends => crate::game_plugins::LEAGUE_OF_LEGENDS_ID,
        // Valorant / CS2 have no plugin (and no icon) yet.
        _ => return None,
    };
    let name = crate::game_plugins::all()
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .map(|plugin| plugin.name.to_string())?;
    Some(ClipGame {
        id: plugin_id.to_string(),
        name,
    })
}

#[tauri::command]
pub fn delete_clip(path: String, settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    Ok(())
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
pub async fn preview_clip_audio_tracks(
    request: AudioPreviewRequest,
    settings: tauri::State<'_, StorageSettings>,
) -> Result<String, String> {
    let source = validate_clip_path(&settings, &request.path)?;
    tauri::async_runtime::spawn_blocking(move || {
        preview_clip_audio_tracks_file(source, request.path, request.audio_track_ids)
    })
    .await
    .map_err(|e| format!("audio preview task: {e}"))?
}

fn preview_clip_audio_tracks_file(
    source: PathBuf,
    display_path: String,
    selected_audio_track_ids: Vec<String>,
) -> Result<String, String> {
    let Some(markers) = read_markers(&source) else {
        return Ok(display_path);
    };
    let selected_indices = selected_audio_track_indices(&markers, &selected_audio_track_ids)?;
    if selected_indices.len() == markers.audio_tracks.len() && selected_indices.len() <= 1 {
        return Ok(display_path);
    }

    let meta = std::fs::metadata(&source).map_err(|e| format!("read clip metadata: {e}"))?;
    let preview_dir = crate::settings::audio_preview_cache_dir();
    std::fs::create_dir_all(&preview_dir)
        .map_err(|e| format!("create audio preview cache: {e}"))?;
    prune_old_audio_previews(&preview_dir);
    let preview = audio_preview_path(&preview_dir, &source, &meta, &selected_audio_track_ids);
    if preview.exists() {
        return Ok(preview.display().to_string());
    }

    if selected_indices.len() > 1
        && mix_audio_preview_with_ffmpeg(&source, &preview, &selected_indices).is_ok()
    {
        return Ok(preview.display().to_string());
    }

    let source_bytes = std::fs::read(&source).map_err(|e| format!("read clip: {e}"))?;
    let preview_bytes = remux_with_selected_audio_tracks(&source_bytes, &selected_indices)
        .map_err(|e| e.to_string())?;
    let tmp = preview.with_extension("mp4.tmp");
    std::fs::write(&tmp, preview_bytes).map_err(|e| format!("write audio preview: {e}"))?;
    match std::fs::rename(&tmp, &preview) {
        Ok(()) => {}
        Err(_) if preview.exists() => {
            let _ = std::fs::remove_file(&tmp);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("finalize audio preview: {e}"));
        }
    }
    Ok(preview.display().to_string())
}

fn mix_audio_preview_with_ffmpeg(
    source: &Path,
    preview: &Path,
    selected_audio_track_indices: &[u32],
) -> Result<(), String> {
    let ffmpeg = clipline_capture::ffmpeg::locate()
        .ok_or_else(|| "ffmpeg is not available for audio preview mixing".to_string())?;
    let filter = ffmpeg_audio_mix_filter(selected_audio_track_indices)?;
    let tmp = preview.with_extension("mp4.tmp");
    let _ = std::fs::remove_file(&tmp);

    let mut cmd = Command::new(ffmpeg);
    suppress_console(&mut cmd);
    let output = cmd
        .args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(source)
        .args(["-filter_complex", &filter])
        .args([
            "-map", "0:v:0", "-map", "[aout]", "-c:v", "copy", "-c:a", "libopus", "-b:a", "160k",
            "-f", "mp4",
        ])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg audio preview: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg audio preview failed: {stderr}"));
    }
    match std::fs::rename(&tmp, preview) {
        Ok(()) => Ok(()),
        Err(_) if preview.exists() => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("finalize audio preview: {e}"))
        }
    }
}

fn ffmpeg_audio_mix_filter(selected_audio_track_indices: &[u32]) -> Result<String, String> {
    if selected_audio_track_indices.is_empty() {
        return Err("audio preview mix requires at least one audio stream".into());
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

#[cfg(windows)]
fn suppress_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn suppress_console(_cmd: &mut Command) {}

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
    if let Some(markers) = read_markers(&source) {
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

fn selected_audio_track_indices(
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

fn audio_preview_path(
    preview_dir: &Path,
    source: &Path,
    meta: &std::fs::Metadata,
    selected_audio_track_ids: &[String],
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    "audio-preview-mix-v3".hash(&mut hasher);
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
    let Ok(entries) = std::fs::read_dir(preview_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("mp4") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > MAX_AGE);
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
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
pub fn copy_clip_to_clipboard(
    path: String,
    settings: tauri::State<StorageSettings>,
) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    copy_file_to_clipboard(&target)
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

fn last_os_error(action: &str) -> String {
    format!("{action}: {}", std::io::Error::last_os_error())
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

fn read_markers(path: &Path) -> Option<ClipMarkers> {
    std::fs::read_to_string(path.with_extension("markers.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .map(filter_timeline_markers)
}

fn filter_timeline_markers(mut markers: ClipMarkers) -> ClipMarkers {
    markers.markers.retain(|m| is_timeline_marker(&m.event));
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
    use clipline_events::{ClipAudioTrack, EventKind, GameEvent, GameId, PlayerSummary};
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
    fn filter_timeline_markers_drops_non_user_kills_and_noise() {
        let markers = ClipMarkers {
            recording_start_s: 10.0,
            duration_s: 100.0,
            player_summary: Some(PlayerSummary {
                champion_name: "Nautilus".into(),
                kills: 3,
                deaths: 4,
                assists: 23,
            }),
            audio_tracks: Vec::new(),
            markers: vec![
                marker_with(1.0, EventKind::ChampionKill, true),
                marker_with(2.0, EventKind::ChampionKill, false),
                marker_with(2.5, EventKind::ChampionDeath, true),
                marker_with(3.0, EventKind::TurretKilled, false),
                marker_with(4.0, EventKind::DragonKill, false),
                marker_with(5.0, EventKind::BaronKill, false),
                marker_with(6.0, EventKind::MinionsSpawning, true),
                marker_with(7.0, EventKind::FirstBlood, true),
                marker_with(8.0, EventKind::FirstBrick, true),
                marker_with(9.0, EventKind::Ace, true),
            ],
        };

        let filtered = filter_timeline_markers(markers);
        let kinds: Vec<_> = filtered.markers.iter().map(|m| m.event.kind).collect();

        assert_eq!(
            kinds,
            vec![
                EventKind::ChampionKill,
                EventKind::ChampionDeath,
                EventKind::TurretKilled,
                EventKind::DragonKill,
                EventKind::BaronKill,
            ]
        );
        assert!(filtered.markers[0].event.involves_local_player);
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
            selected_audio_track_indices(&markers, &["microphone".into()]).unwrap(),
            vec![1]
        );
        assert_eq!(
            selected_audio_track_indices(&markers, &["microphone".into(), "output".into()])
                .unwrap(),
            vec![0, 1]
        );

        let err = selected_audio_track_indices(&markers, &["discord".into()]).unwrap_err();
        assert!(err.contains("unknown audio track"), "{err}");
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

    #[test]
    fn list_clips_uses_marker_duration_without_parsing_mp4() {
        let dir = TestDir::new("list-marker-duration");
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

    #[test]
    fn validate_clip_path_accepts_root_and_session_clips() {
        let dir = TestDir::new("validate-accept");
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
        let dir = TestDir::new("validate-reject");
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
