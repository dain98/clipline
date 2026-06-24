//! Clipline Cloud desktop integration: connection state, OS credential storage,
//! and per-clip uploads through the first-party API client.

use std::ffi::OsStr;
use std::path::Path;
use std::ptr;
use std::slice;
use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use clipline_cloud_api::{
    sha256_hex, ClipDetailResponse, CloudApiError, CloudClient, CreateUploadRequest,
};
use clipline_events::{ClipMarkers, GameId};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use crate::app::RuntimeState;
use crate::library::{validate_clip_path, StorageSettings};
use crate::settings::{normalize_cloud_visibility, CloudSettings, CloudUploadRecord};
use crate::util::{last_os_error, unix_now, wide_null};

const DEFAULT_DEVICE_NAME: &str = "Clipline Desktop";
const READY_POLL_ATTEMPTS: usize = 30;
const READY_POLL_DELAY: Duration = Duration::from_secs(1);
const CLOUD_UPLOAD_PROGRESS_EVENT: &str = "cloud-upload-progress";
const REMOTE_NOT_FOUND_SYNC_MARKER: &str = "remote clip not found during status sync";

#[derive(Debug, Deserialize)]
pub struct CloudConnectRequest {
    pub host_url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub plain_http_confirmed: bool,
    #[serde(default)]
    pub default_visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UploadClipCommandRequest {
    pub path: String,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "audioTrackIds")]
    pub audio_track_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SyncCloudClipStatusRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct CloudConnectionStatus {
    pub connected: bool,
    pub token_present: bool,
    pub host_url: String,
    pub public_url: Option<String>,
    pub username: Option<String>,
    pub user_id: Option<String>,
    pub default_visibility: String,
    pub delete_local_after_upload: bool,
    pub auto_upload_rules: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct CloudUploadProgressEvent {
    pub local_clip_id: String,
    pub path: String,
    pub upload_status: String,
    pub received_size_bytes: u64,
    pub file_size_bytes: u64,
    pub remote_clip_id: Option<String>,
    pub remote_url: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CloudUploadResult {
    pub record: CloudUploadRecord,
    pub clip: Option<ClipDetailResponse>,
}

#[derive(Debug, Serialize)]
pub struct CloudClipStatusSyncResult {
    pub path: String,
    pub record: Option<CloudUploadRecord>,
    pub removed: bool,
}

#[tauri::command]
pub fn cloud_status(state: tauri::State<RuntimeState>) -> CloudConnectionStatus {
    let settings = state.settings();
    connection_status(&settings.cloud)
}

#[tauri::command]
pub async fn sync_cloud_clip_status(
    state: tauri::State<'_, RuntimeState>,
    request: SyncCloudClipStatusRequest,
) -> Result<CloudClipStatusSyncResult, String> {
    let settings = state.settings();
    let cloud = settings.cloud.clone();
    let Some(record) = cloud_record_for_path(&cloud, &request.path) else {
        return Ok(CloudClipStatusSyncResult {
            path: request.path,
            record: None,
            removed: false,
        });
    };
    let Some(remote_clip_id) = record.remote_clip_id.clone() else {
        return Ok(CloudClipStatusSyncResult {
            path: request.path,
            record: Some(record),
            removed: false,
        });
    };
    let token_target = cloud
        .credential_target
        .clone()
        .ok_or_else(|| "connect to Clipline Cloud first".to_string())?;
    let token = read_credential(&token_target)?;
    let client = connected_client(&cloud, &token)?;

    match client.get_clip(&remote_clip_id).await {
        Ok(clip) => {
            let mut updated = record;
            apply_remote_clip_to_record(&cloud, &mut updated, &clip);
            persist_record(&state, &updated)?;
            Ok(CloudClipStatusSyncResult {
                path: request.path,
                record: Some(updated),
                removed: false,
            })
        }
        Err(error) if cloud_error_is_not_found(&error) => match missing_remote_sync_action(&record)
        {
            MissingRemoteSyncAction::Keep => Ok(CloudClipStatusSyncResult {
                path: request.path,
                record: Some(record),
                removed: false,
            }),
            MissingRemoteSyncAction::ConfirmMissing => {
                let mut updated = record;
                mark_remote_not_found_once(&mut updated);
                persist_record(&state, &updated)?;
                Ok(CloudClipStatusSyncResult {
                    path: request.path,
                    record: Some(updated),
                    removed: false,
                })
            }
            MissingRemoteSyncAction::Remove => {
                state.update_cloud(|cloud| {
                    remove_upload_record(cloud, &record);
                })?;
                Ok(CloudClipStatusSyncResult {
                    path: request.path,
                    record: None,
                    removed: true,
                })
            }
        },
        Err(error) => Err(cloud_error(error)),
    }
}

#[tauri::command]
pub async fn cloud_connect(
    state: tauri::State<'_, RuntimeState>,
    request: CloudConnectRequest,
) -> Result<CloudConnectionStatus, String> {
    let visibility = request
        .default_visibility
        .as_deref()
        .map(normalize_cloud_visibility)
        .unwrap_or_else(|| "private".to_string());
    let device_name = request
        .device_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_DEVICE_NAME)
        .to_string();

    let connected = clipline_cloud_api::connect_with_device_token(
        request.host_url.trim(),
        request.username.trim().to_string(),
        request.password,
        device_name,
        request.plain_http_confirmed,
    )
    .await
    .map_err(cloud_error)?;

    let host_url = connected
        .client
        .base_url()
        .as_str()
        .trim_end_matches('/')
        .to_string();
    let public_url = connected
        .discovery
        .public_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    let target = credential_target(&host_url, &connected.user.id);
    write_credential(&target, &connected.user.username, &connected.token)?;

    let old_target = state.settings().cloud.credential_target;
    let settings = state.update_cloud(|cloud| {
        let identity_changed = cloud.host_url != host_url
            || cloud.connected_user_id.as_deref() != Some(connected.user.id.as_str());
        cloud.host_url = host_url.clone();
        cloud.public_url = Some(public_url.clone());
        cloud.connected_user_id = Some(connected.user.id.clone());
        cloud.connected_username = Some(connected.user.username.clone());
        cloud.credential_target = Some(target.clone());
        cloud.default_visibility = visibility.clone();
        if identity_changed {
            cloud.uploads.clear();
        }
    })?;

    if old_target.as_deref().is_some_and(|old| old != target) {
        if let Err(e) = delete_credential(old_target.as_deref().unwrap()) {
            eprintln!("delete old cloud credential: {e}");
        }
    }

    Ok(connection_status(&settings.cloud))
}

#[tauri::command]
pub fn cloud_disconnect(
    state: tauri::State<RuntimeState>,
) -> Result<CloudConnectionStatus, String> {
    let old_target = state.settings().cloud.credential_target;
    if let Some(target) = old_target.as_deref() {
        if let Err(e) = delete_credential(target) {
            eprintln!("delete cloud credential on disconnect: {e}");
        }
    }
    let settings = state.update_cloud(|cloud| {
        cloud.connected_user_id = None;
        cloud.connected_username = None;
        cloud.credential_target = None;
    })?;
    Ok(connection_status(&settings.cloud))
}

#[tauri::command]
pub async fn upload_clip_to_cloud<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, RuntimeState>,
    storage: tauri::State<'_, StorageSettings>,
    request: UploadClipCommandRequest,
) -> Result<CloudUploadResult, String> {
    let target = validate_clip_path(&storage, &request.path)?;
    let settings = state.settings();
    let cloud = settings.cloud.clone();
    let token_target = cloud
        .credential_target
        .clone()
        .ok_or_else(|| "connect to Clipline Cloud first".to_string())?;
    let token = read_credential(&token_target)?;
    let client = connected_client(&cloud, &token)?;
    let visibility = request
        .visibility
        .as_deref()
        .map(normalize_cloud_visibility)
        .unwrap_or_else(|| cloud.default_visibility.clone());
    let description = normalize_upload_description(request.description.as_deref());

    let meta = std::fs::metadata(&target).map_err(|e| format!("read clip metadata: {e}"))?;
    if meta.len() == 0 {
        return Err("clip file is empty".into());
    }
    let markers = crate::util::read_markers_raw(&target);
    let bytes = upload_bytes_for_audio_selection_from_path(
        &target,
        markers.as_ref(),
        request.audio_track_ids.as_deref(),
    )
    .await?;
    let checksum = sha256_hex(&bytes);
    let local_clip_id = local_clip_id(&target, &meta, &checksum)?;
    let mut record = CloudUploadRecord {
        local_clip_id: local_clip_id.clone(),
        // Store the path exactly as `list_clips` emits it (non-canonical), so the
        // UI can pair this record to its clip row by string equality. `target` is
        // the canonicalized form (`\\?\D:\…` on Windows) and is used only for I/O.
        path: request.path.clone(),
        remote_clip_id: None,
        remote_url: None,
        visibility: visibility.clone(),
        upload_status: existing_retry_status(&cloud, &local_clip_id, &request.path),
        error: None,
        updated_at_unix: unix_now(),
    };
    persist_record(&state, &record)?;
    emit_upload_progress(&app, &record, 0, bytes.len() as u64, None);

    let upload_request = create_upload_request(UploadRequestInput {
        path: &target,
        meta: &meta,
        bytes: &bytes,
        checksum: &checksum,
        visibility: &visibility,
        markers: markers.as_ref(),
        client_clip_id: &local_clip_id,
        title: request.title.as_deref(),
    })?;
    let progress_path = request.path.clone();
    let upload_result = crate::cloud_upload::upload_mp4_bytes_with_progress(
        &client,
        &token,
        &upload_request,
        description.as_deref(),
        &bytes,
        |progress| {
            let url = cloud_clip_url(&cloud, &progress.clip_id);
            let status = if progress.status == "completed" {
                "processing"
            } else {
                "uploading"
            };
            let event = CloudUploadProgressEvent {
                local_clip_id: local_clip_id.clone(),
                path: progress_path.clone(),
                upload_status: status.to_string(),
                received_size_bytes: progress.received_size_bytes,
                file_size_bytes: progress.file_size_bytes,
                remote_clip_id: Some(progress.clip_id.clone()),
                remote_url: url,
                error: None,
            };
            let _ = app.emit(CLOUD_UPLOAD_PROGRESS_EVENT, event);
        },
    )
    .await;

    let progress = match upload_result {
        Ok(progress) => progress,
        Err(error) => {
            record.upload_status = "failed".to_string();
            record.error = Some(cloud_error(error));
            record.updated_at_unix = unix_now();
            persist_record(&state, &record)?;
            emit_upload_progress(&app, &record, 0, bytes.len() as u64, record.error.clone());
            return Ok(CloudUploadResult { record, clip: None });
        }
    };

    record.remote_clip_id = Some(progress.clip_id.clone());
    record.remote_url = cloud_clip_url(&cloud, &progress.clip_id);
    record.upload_status = "processing".to_string();
    record.error = None;
    record.updated_at_unix = unix_now();
    persist_record(&state, &record)?;
    emit_upload_progress(
        &app,
        &record,
        progress.received_size_bytes,
        progress.file_size_bytes,
        None,
    );

    let clip = match wait_for_ready_clip(&client, &progress.clip_id).await {
        Ok(Some(clip)) if visibility == "private" => Some(clip),
        Ok(Some(clip)) => Some(
            client
                .set_visibility(&clip.id, visibility.clone())
                .await
                .map_err(cloud_error)?,
        ),
        Ok(None) => {
            mark_ready_timeout(&mut record);
            persist_record(&state, &record)?;
            emit_upload_progress(
                &app,
                &record,
                progress.file_size_bytes,
                progress.file_size_bytes,
                record.error.clone(),
            );
            None
        }
        Err(error) => return Err(cloud_error(error)),
    };

    if let Some(clip) = &clip {
        apply_remote_clip_to_record(&cloud, &mut record, clip);
        persist_record(&state, &record)?;
        emit_upload_progress(
            &app,
            &record,
            progress.file_size_bytes,
            progress.file_size_bytes,
            None,
        );

        if cloud.delete_local_after_upload {
            delete_uploaded_local_files(&target);
        }
    }

    Ok(CloudUploadResult { record, clip })
}

fn connection_status(cloud: &CloudSettings) -> CloudConnectionStatus {
    let token_present = cloud
        .credential_target
        .as_deref()
        .is_some_and(|target| read_credential(target).is_ok());
    CloudConnectionStatus {
        connected: cloud.connected() && token_present,
        token_present,
        host_url: cloud.host_url.clone(),
        public_url: cloud.public_url.clone(),
        username: cloud.connected_username.clone(),
        user_id: cloud.connected_user_id.clone(),
        default_visibility: cloud.default_visibility.clone(),
        delete_local_after_upload: cloud.delete_local_after_upload,
        auto_upload_rules: cloud.auto_upload_rules,
    }
}

fn connected_client(cloud: &CloudSettings, token: &str) -> Result<CloudClient, String> {
    if !cloud.connected() {
        return Err("connect to Clipline Cloud first".into());
    }
    let base_url =
        clipline_cloud_api::validate_cloud_host(&cloud.host_url, true).map_err(cloud_error)?;
    Ok(CloudClient::with_device_token(base_url, token))
}

struct UploadRequestInput<'a> {
    path: &'a Path,
    meta: &'a std::fs::Metadata,
    bytes: &'a [u8],
    checksum: &'a str,
    visibility: &'a str,
    markers: Option<&'a ClipMarkers>,
    client_clip_id: &'a str,
    title: Option<&'a str>,
}

fn create_upload_request(input: UploadRequestInput<'_>) -> Result<CreateUploadRequest, String> {
    let game = read_clip_game(input.path, input.markers);
    Ok(CreateUploadRequest {
        client_clip_id: Some(input.client_clip_id.to_string()),
        title: upload_title(input.title, input.path),
        game_name: game.as_ref().map(|game| game.name.clone()),
        game_id: game.as_ref().map(|game| game.id.clone()),
        game_executable: None,
        source_type: Some(source_type(input.path)),
        recorded_at: input.meta.modified().ok().map(DateTime::<Utc>::from),
        duration_ms: clip_duration_ms(input.bytes, input.markers),
        file_size_bytes: input.bytes.len() as u64,
        checksum_sha256: input.checksum.to_string(),
        container: "mp4".to_string(),
        video_codec: None,
        audio_codec: None,
        width: None,
        height: None,
        fps: None,
        visibility: Some(input.visibility.to_string()),
        markers: None,
    })
}

fn upload_title(title: Option<&str>, path: &Path) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| clip_title(path))
}

fn normalize_upload_description(description: Option<&str>) -> Option<String> {
    description
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UploadAudioSelectionPlan {
    Original,
    Remux(Vec<u32>),
}

async fn upload_bytes_for_audio_selection_from_path(
    source_path: &Path,
    markers: Option<&ClipMarkers>,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<Vec<u8>, String> {
    match upload_audio_selection_plan(markers, selected_audio_track_ids)? {
        UploadAudioSelectionPlan::Original => tokio::fs::read(source_path)
            .await
            .map_err(|e| format!("read clip: {e}")),
        UploadAudioSelectionPlan::Remux(selected_indices) => {
            let source_bytes = tokio::fs::read(source_path)
                .await
                .map_err(|e| format!("read clip: {e}"))?;
            clipline_mp4::remux_with_selected_audio_tracks(&source_bytes, &selected_indices)
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
fn upload_bytes_for_audio_selection(
    source_bytes: Vec<u8>,
    markers: Option<&ClipMarkers>,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<Vec<u8>, String> {
    match upload_audio_selection_plan(markers, selected_audio_track_ids)? {
        UploadAudioSelectionPlan::Original => Ok(source_bytes),
        UploadAudioSelectionPlan::Remux(selected_indices) => {
            clipline_mp4::remux_with_selected_audio_tracks(&source_bytes, &selected_indices)
                .map_err(|e| e.to_string())
        }
    }
}

fn upload_audio_selection_plan(
    markers: Option<&ClipMarkers>,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<UploadAudioSelectionPlan, String> {
    let Some(selected_audio_track_ids) = selected_audio_track_ids else {
        return Ok(UploadAudioSelectionPlan::Original);
    };
    let tracks = markers.map(|m| m.audio_tracks.as_slice()).unwrap_or(&[]);
    if tracks.is_empty() {
        if selected_audio_track_ids.is_empty() {
            return Ok(UploadAudioSelectionPlan::Remux(Vec::new()));
        }
        return Err("this clip has no selectable audio track metadata".into());
    }

    let selected_indices =
        crate::util::selected_audio_track_indices(markers.unwrap(), selected_audio_track_ids)?;
    Ok(UploadAudioSelectionPlan::Remux(selected_indices))
}

fn read_clip_game(path: &Path, markers: Option<&ClipMarkers>) -> Option<crate::library::ClipGame> {
    path.parent()
        .and_then(|dir| std::fs::read_to_string(dir.join("clipline-session.json")).ok())
        .and_then(|json| serde_json::from_str::<crate::library::ClipGame>(&json).ok())
        .or_else(|| markers.and_then(game_from_markers))
}

fn game_from_markers(markers: &ClipMarkers) -> Option<crate::library::ClipGame> {
    let game_id = markers.markers.first()?.event.game_id;
    let id = match game_id {
        GameId::LeagueOfLegends => "league_of_legends",
        GameId::Valorant => "valorant",
        GameId::Cs2 => "cs2",
    };
    Some(crate::library::ClipGame {
        id: id.to_string(),
        name: serde_json_string(&game_id).unwrap_or_else(|| id.to_string()),
    })
}

fn clip_duration_ms(bytes: &[u8], markers: Option<&ClipMarkers>) -> Option<i64> {
    clipline_mp4::walker::movie_duration_s(bytes)
        .or_else(|| markers.map(|markers| markers.duration_s))
        .map(|seconds| (seconds * 1000.0).round())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as i64)
}

fn source_type(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.contains("session") {
        "session"
    } else if name.contains("trim") {
        "trim"
    } else {
        "replay"
    }
    .to_string()
}

fn clip_title(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Clipline clip".to_string())
}

fn local_clip_id(path: &Path, meta: &std::fs::Metadata, checksum: &str) -> Result<String, String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("resolve clip path: {e}"))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let payload = format!(
        "clipline-local-v1\0{}\0{}\0{}\0{}",
        canonical.display(),
        meta.len(),
        modified,
        checksum
    );
    Ok(format!("clipline-local-{}", sha256_hex(payload.as_bytes())))
}

fn existing_retry_status(cloud: &CloudSettings, local_clip_id: &str, path: &str) -> String {
    let existing = cloud.uploads.get(local_clip_id).or_else(|| {
        cloud
            .uploads
            .values()
            .filter(|record| record.path == path)
            .max_by_key(|record| record.updated_at_unix)
    });
    match existing.map(|record| record.upload_status.as_str()) {
        Some("failed") => "retrying".to_string(),
        Some("uploading") | Some("queued") | Some("processing") => "retrying".to_string(),
        _ => "queued".to_string(),
    }
}

fn cloud_record_for_path(cloud: &CloudSettings, path: &str) -> Option<CloudUploadRecord> {
    cloud
        .uploads
        .values()
        .filter(|record| record.path == path)
        .max_by_key(|record| record.updated_at_unix)
        .cloned()
}

fn replace_upload_record(cloud: &mut CloudSettings, record: CloudUploadRecord) {
    cloud
        .uploads
        .retain(|key, existing| key == &record.local_clip_id || existing.path != record.path);
    cloud.uploads.insert(record.local_clip_id.clone(), record);
}

fn remove_upload_record(cloud: &mut CloudSettings, record: &CloudUploadRecord) {
    cloud
        .uploads
        .retain(|key, existing| key != &record.local_clip_id && existing.path != record.path);
}

fn persist_record(state: &RuntimeState, record: &CloudUploadRecord) -> Result<(), String> {
    state.update_cloud(|cloud| {
        replace_upload_record(cloud, record.clone());
    })?;
    Ok(())
}

fn mark_ready_timeout(record: &mut CloudUploadRecord) {
    record.upload_status = "uploaded_processing".to_string();
    record.error = Some(format!(
        "cloud upload completed, but cloud processing did not become ready within {} seconds; the cloud link may finish processing shortly",
        READY_POLL_ATTEMPTS as u64 * READY_POLL_DELAY.as_secs()
    ));
    record.updated_at_unix = unix_now();
}

fn apply_remote_clip_to_record(
    cloud: &CloudSettings,
    record: &mut CloudUploadRecord,
    clip: &ClipDetailResponse,
) {
    record.visibility = clip.visibility.clone();
    record.remote_clip_id = Some(clip.id.clone());
    record.remote_url = clip
        .public_url
        .clone()
        .or_else(|| cloud_clip_url(cloud, &clip.id));
    record.upload_status = upload_status_for_remote_clip(clip);
    record.error = None;
    record.updated_at_unix = unix_now();
}

fn upload_status_for_remote_clip(clip: &ClipDetailResponse) -> String {
    if clip.status != "ready" {
        "uploaded_processing".to_string()
    } else if clip.visibility == "private" {
        "uploaded_private".to_string()
    } else {
        "uploaded_public".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingRemoteSyncAction {
    Keep,
    ConfirmMissing,
    Remove,
}

fn missing_remote_sync_action(record: &CloudUploadRecord) -> MissingRemoteSyncAction {
    if !record.upload_status.starts_with("uploaded_")
        || record.upload_status == "uploaded_processing"
    {
        return MissingRemoteSyncAction::Keep;
    }
    if record.error.as_deref() == Some(REMOTE_NOT_FOUND_SYNC_MARKER) {
        MissingRemoteSyncAction::Remove
    } else {
        MissingRemoteSyncAction::ConfirmMissing
    }
}

fn mark_remote_not_found_once(record: &mut CloudUploadRecord) {
    record.error = Some(REMOTE_NOT_FOUND_SYNC_MARKER.to_string());
    record.updated_at_unix = unix_now();
}

fn delete_uploaded_local_files(target: &Path) {
    if let Err(e) = std::fs::remove_file(target) {
        eprintln!("delete local clip after upload {target:?}: {e}");
    }
    // Sidecars may not exist — ignore missing-file errors.
    let _ = std::fs::remove_file(target.with_extension("markers.json"));
    let _ = std::fs::remove_file(crate::poster::poster_path(target));
}

fn emit_upload_progress<R: Runtime>(
    app: &AppHandle<R>,
    record: &CloudUploadRecord,
    received_size_bytes: u64,
    file_size_bytes: u64,
    error: Option<String>,
) {
    let _ = app.emit(
        CLOUD_UPLOAD_PROGRESS_EVENT,
        CloudUploadProgressEvent {
            local_clip_id: record.local_clip_id.clone(),
            path: record.path.clone(),
            upload_status: record.upload_status.clone(),
            received_size_bytes,
            file_size_bytes,
            remote_clip_id: record.remote_clip_id.clone(),
            remote_url: record.remote_url.clone(),
            error,
        },
    );
}

async fn wait_for_ready_clip(
    client: &CloudClient,
    clip_id: &str,
) -> Result<Option<ClipDetailResponse>, CloudApiError> {
    for _ in 0..READY_POLL_ATTEMPTS {
        match client.get_clip(clip_id).await {
            Ok(clip) => return Ok(Some(clip)),
            Err(CloudApiError::Api { status, .. }) if status.as_u16() == 404 => {
                tokio::time::sleep(READY_POLL_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn cloud_clip_url(cloud: &CloudSettings, clip_id: &str) -> Option<String> {
    if clip_id.is_empty() {
        return None;
    }
    let base = cloud.public_url.as_deref().unwrap_or(&cloud.host_url);
    clipline_cloud_api::validate_cloud_host(base, true)
        .ok()
        .and_then(|url| url.join(&format!("clip/{clip_id}")).ok())
        .map(|url| url.to_string())
}

fn credential_target(host_url: &str, user_id: &str) -> String {
    format!("Clipline Cloud:{host_url}:{user_id}")
}

fn write_credential(target: &str, username: &str, token: &str) -> Result<(), String> {
    let mut target_w = wide_null(OsStr::new(target));
    let mut username_w = wide_null(OsStr::new(username));
    let mut blob = token.as_bytes().to_vec();
    let blob_len = u32::try_from(blob.len()).map_err(|_| "cloud token is too large".to_string())?;
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_w.as_mut_ptr(),
        Comment: ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob_len,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: ptr::null_mut(),
        TargetAlias: ptr::null_mut(),
        UserName: username_w.as_mut_ptr(),
    };
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        return Err(last_os_error("store cloud token"));
    }
    Ok(())
}

fn read_credential(target: &str) -> Result<String, String> {
    let target_w = wide_null(OsStr::new(target));
    let mut raw: *mut CREDENTIALW = ptr::null_mut();
    if unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
        return Err(last_os_error("read cloud token"));
    }
    let _free = CredentialFree(raw);
    let credential = unsafe { &*raw };
    let bytes = unsafe {
        slice::from_raw_parts(
            credential.CredentialBlob,
            credential.CredentialBlobSize as usize,
        )
    };
    String::from_utf8(bytes.to_vec()).map_err(|_| "cloud token is not valid UTF-8".to_string())
}

fn delete_credential(target: &str) -> Result<(), String> {
    let target_w = wide_null(OsStr::new(target));
    if unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        return Err(last_os_error("delete cloud token"));
    }
    Ok(())
}

struct CredentialFree(*mut CREDENTIALW);

impl Drop for CredentialFree {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CredFree(self.0.cast());
            }
        }
    }
}

fn serde_json_string<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
}

fn cloud_error(error: CloudApiError) -> String {
    error.to_string()
}

fn cloud_error_is_not_found(error: &CloudApiError) -> bool {
    matches!(error, CloudApiError::Api { status, .. } if status.as_u16() == 404)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_events::ClipAudioTrack;
    use clipline_mp4::{
        AudioTrackConfig, FragSample, HybridMp4Writer, TrackConfig, VideoTrackConfig,
    };
    use std::io::Cursor;

    #[test]
    fn credential_target_includes_server_and_user() {
        assert_eq!(
            credential_target("https://clips.example.com", "user_1"),
            "Clipline Cloud:https://clips.example.com:user_1"
        );
    }

    #[test]
    fn source_type_falls_back_to_replay() {
        assert_eq!(source_type(Path::new("clipline-2026-06-16.mp4")), "replay");
        assert_eq!(source_type(Path::new("full-session.mp4")), "session");
        assert_eq!(source_type(Path::new("ranked-trim.mp4")), "trim");
    }

    #[test]
    fn upload_audio_selection_remuxes_multiple_selected_tracks() {
        let source = two_audio_mp4();
        let markers = audio_markers();
        let selected = vec!["output".to_string(), "microphone".to_string()];

        let out =
            upload_bytes_for_audio_selection(source, Some(&markers), Some(&selected)).unwrap();

        assert!(out.windows(6).any(|w| w == b"V00000"));
        assert!(out.windows(6).any(|w| w == b"A00000"));
        assert!(out.windows(6).any(|w| w == b"B00000"));
    }

    #[test]
    fn upload_audio_selection_plan_remuxes_without_source_bytes_for_multiple_tracks() {
        let markers = audio_markers();
        let selected = vec!["output".to_string(), "microphone".to_string()];

        assert_eq!(
            upload_audio_selection_plan(Some(&markers), Some(&selected)).unwrap(),
            UploadAudioSelectionPlan::Remux(vec![0, 1])
        );
    }

    #[test]
    fn upload_audio_selection_remuxes_only_selected_track() {
        let source = two_audio_mp4();
        let markers = audio_markers();
        let selected = vec!["microphone".to_string()];

        let out =
            upload_bytes_for_audio_selection(source, Some(&markers), Some(&selected)).unwrap();

        assert!(out.windows(6).any(|w| w == b"V00000"));
        assert!(!out.windows(6).any(|w| w == b"A00000"));
        assert!(out.windows(6).any(|w| w == b"B00000"));
    }

    #[test]
    fn upload_audio_selection_rejects_unknown_track_id() {
        let source = two_audio_mp4();
        let markers = audio_markers();
        let selected = vec!["discord".to_string()];

        let err = upload_bytes_for_audio_selection(source, Some(&markers), Some(&selected))
            .expect_err("unknown track");

        assert!(err.contains("unknown audio track"), "{err}");
    }

    #[test]
    fn upload_record_supersedes_older_record_for_same_path() {
        let mut cloud = CloudSettings::default();
        cloud.uploads.insert(
            "old".into(),
            upload_record("old", "D:\\Videos\\clip.mp4", "failed", 10),
        );
        cloud.uploads.insert(
            "other".into(),
            upload_record("other", "D:\\Videos\\other.mp4", "uploaded_public", 11),
        );

        let newer = upload_record("new", "D:\\Videos\\clip.mp4", "queued", 12);
        replace_upload_record(&mut cloud, newer.clone());

        assert!(!cloud.uploads.contains_key("old"));
        assert_eq!(cloud.uploads.get("new"), Some(&newer));
        assert_eq!(
            cloud
                .uploads
                .get("other")
                .map(|record| record.path.as_str()),
            Some("D:\\Videos\\other.mp4")
        );
    }

    #[test]
    fn existing_retry_status_uses_same_path_when_audio_selection_changed() {
        let mut cloud = CloudSettings::default();
        cloud.uploads.insert(
            "old".into(),
            upload_record("old", "D:\\Videos\\clip.mp4", "failed", 10),
        );

        assert_eq!(
            existing_retry_status(&cloud, "new", "D:\\Videos\\clip.mp4"),
            "retrying"
        );
        assert_eq!(
            existing_retry_status(&cloud, "new", "D:\\Videos\\other.mp4"),
            "queued"
        );
    }

    #[test]
    fn ready_timeout_keeps_remote_link_available_without_retry_upload() {
        let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "processing", 10);
        record.remote_clip_id = Some("remote-1".into());
        record.remote_url = Some("https://clips.example.com/clip/remote-1".into());

        mark_ready_timeout(&mut record);

        assert_eq!(record.upload_status, "uploaded_processing");
        assert_eq!(record.remote_clip_id.as_deref(), Some("remote-1"));
        assert_eq!(
            record.remote_url.as_deref(),
            Some("https://clips.example.com/clip/remote-1")
        );
        assert!(
            record
                .error
                .as_deref()
                .is_some_and(|error| error.contains("processing") && !error.contains("retry the upload")),
            "timeout should explain that cloud processing is still pending without forcing a reupload"
        );
    }

    #[test]
    fn cloud_clip_detail_updates_record_visibility_status_and_url() {
        let cloud = CloudSettings {
            public_url: Some("https://clips.example.com".into()),
            ..CloudSettings::default()
        };
        let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_public", 10);
        record.remote_clip_id = Some("remote-1".into());
        record.remote_url = Some("https://clips.example.com/old".into());

        apply_remote_clip_to_record(
            &cloud,
            &mut record,
            &clip_detail(
                "remote-1",
                "unlisted",
                "ready",
                Some("https://share.example.com/c/1"),
            ),
        );

        assert_eq!(record.visibility, "unlisted");
        assert_eq!(record.upload_status, "uploaded_public");
        assert_eq!(
            record.remote_url.as_deref(),
            Some("https://share.example.com/c/1")
        );
        assert!(record.error.is_none());
    }

    #[test]
    fn missing_remote_clip_keeps_unconfirmed_and_processing_records() {
        assert_eq!(
            missing_remote_sync_action(&upload_record(
                "local",
                "D:\\Videos\\clip.mp4",
                "uploaded_public",
                10
            )),
            MissingRemoteSyncAction::ConfirmMissing
        );
        assert_eq!(
            missing_remote_sync_action(&upload_record(
                "local",
                "D:\\Videos\\clip.mp4",
                "uploaded_processing",
                10
            )),
            MissingRemoteSyncAction::Keep
        );
        assert_eq!(
            missing_remote_sync_action(&upload_record(
                "local",
                "D:\\Videos\\clip.mp4",
                "processing",
                10
            )),
            MissingRemoteSyncAction::Keep
        );
    }

    #[test]
    fn missing_remote_clip_requires_confirmation_before_removing_finalized_record() {
        let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "uploaded_public", 10);

        assert_eq!(
            missing_remote_sync_action(&record),
            MissingRemoteSyncAction::ConfirmMissing
        );

        mark_remote_not_found_once(&mut record);

        assert_eq!(
            missing_remote_sync_action(&record),
            MissingRemoteSyncAction::Remove
        );
    }

    #[test]
    fn delete_uploaded_local_files_removes_poster_sidecar() {
        let dir = test_dir("cloud-delete");
        let clip = dir.join("clip.mp4");
        let markers = clip.with_extension("markers.json");
        let poster = crate::poster::poster_path(&clip);
        std::fs::write(&clip, b"mp4").unwrap();
        std::fs::write(&markers, b"{}").unwrap();
        std::fs::write(&poster, b"jpg").unwrap();

        delete_uploaded_local_files(&clip);

        assert!(!clip.exists());
        assert!(!markers.exists());
        assert!(!poster.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn audio_markers() -> ClipMarkers {
        ClipMarkers {
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
        }
    }

    fn two_audio_mp4() -> Vec<u8> {
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
        let output = audio_samples("A");
        let mic = audio_samples("B");
        writer
            .write_fragment_multi(&[&video, &output, &mic])
            .unwrap();
        writer.finalize().unwrap().into_inner()
    }

    fn audio_samples(prefix: &str) -> Vec<FragSample> {
        (0..50)
            .map(|i| FragSample {
                data: format!("{prefix}{i:05}").into_bytes(),
                duration: 960,
                is_sync: true,
            })
            .collect()
    }

    fn upload_record(
        local_clip_id: &str,
        path: &str,
        upload_status: &str,
        updated_at_unix: u64,
    ) -> CloudUploadRecord {
        CloudUploadRecord {
            local_clip_id: local_clip_id.into(),
            path: path.into(),
            remote_clip_id: None,
            remote_url: None,
            visibility: "private".into(),
            upload_status: upload_status.into(),
            error: None,
            updated_at_unix,
        }
    }

    fn clip_detail(
        id: &str,
        visibility: &str,
        status: &str,
        public_url: Option<&str>,
    ) -> ClipDetailResponse {
        let now = Utc::now();
        ClipDetailResponse {
            id: id.into(),
            client_clip_id: Some("local".into()),
            title: "Clip".into(),
            game_name: None,
            game_id: None,
            game_executable: None,
            source_type: Some("replay".into()),
            recorded_at: None,
            uploaded_at: Some(now),
            duration_ms: None,
            file_size_bytes: None,
            width: None,
            height: None,
            fps: None,
            container: Some("mp4".into()),
            video_codec: None,
            audio_codec: None,
            checksum_sha256: None,
            visibility: visibility.into(),
            status: status.into(),
            public_share_id: None,
            public_url: public_url.map(str::to_string),
            markers: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "clipline-cloud-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
