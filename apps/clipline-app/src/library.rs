//! Clip library commands: inventory of the configured media folder for the UI and
//! a path-validated delete. The webview never touches the filesystem
//! directly — playback goes through the asset protocol.

use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;

use clipline_events::{is_timeline_marker, ClipMarker, ClipMarkers, GameId};
use clipline_mp4::trim_keyframe_aligned_file;
use clipline_mp4::walker::movie_duration_s;
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
    pub requested_start_s: f64,
    pub requested_end_s: f64,
    pub aligned_start_s: f64,
    pub aligned_end_s: f64,
    pub duration_s: f64,
}

#[tauri::command]
pub fn list_clips(settings: tauri::State<StorageSettings>) -> Result<Vec<ClipInfo>, String> {
    let dir = settings.clips_dir()?;
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
        // Full read is fine at clip sizes; the moov tail needs the soft-
        // remuxed file anyway. Revisit if listing ever feels slow.
        let duration_s = std::fs::read(&path)
            .ok()
            .and_then(|buf| movie_duration_s(&buf));
        let markers = read_markers(&path);
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
pub fn export_clip(
    path: String,
    start_s: f64,
    end_s: f64,
    settings: tauri::State<StorageSettings>,
) -> Result<ExportedClipInfo, String> {
    let source = validate_clip_path(&settings, &path)?;
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

    if let Some(markers) = read_markers(&source) {
        let cropped = crop_markers(&markers, info.aligned_start_s, info.aligned_end_s);
        if has_marker_sidecar_content(&cropped) {
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
    let status = read_storage_status(&settings.clips_dir()?, settings.quota_bytes())
        .map_err(|e| e.to_string())?;
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
    !markers.markers.is_empty() || markers.player_summary.is_some()
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
    use clipline_events::{EventKind, GameEvent, GameId, PlayerSummary};
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
            markers: vec![
                marker_with(1.0, EventKind::ChampionKill, true),
                marker_with(2.0, EventKind::ChampionKill, false),
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
            markers: Vec::new(),
        };

        assert!(!has_marker_sidecar_content(&markers));
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
