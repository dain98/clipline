//! Clipline Cloud desktop integration: connection state, OS credential storage,
//! and per-clip uploads through the first-party API client.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::slice;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

const DEFAULT_DEVICE_NAME: &str = "Clipline Desktop";
const READY_POLL_ATTEMPTS: usize = 30;
const READY_POLL_DELAY: Duration = Duration::from_secs(1);
const CLOUD_UPLOAD_PROGRESS_EVENT: &str = "cloud-upload-progress";

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

#[tauri::command]
pub fn cloud_status(state: tauri::State<RuntimeState>) -> CloudConnectionStatus {
    let settings = state.settings();
    connection_status(&settings.cloud)
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
        let _ = delete_credential(old_target.as_deref().unwrap());
    }

    Ok(connection_status(&settings.cloud))
}

#[tauri::command]
pub fn cloud_disconnect(
    state: tauri::State<RuntimeState>,
) -> Result<CloudConnectionStatus, String> {
    let old_target = state.settings().cloud.credential_target;
    if let Some(target) = old_target.as_deref() {
        let _ = delete_credential(target);
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
    path: String,
    visibility: Option<String>,
    title: Option<String>,
    description: Option<String>,
) -> Result<CloudUploadResult, String> {
    let target = validate_clip_path(&storage, &path)?;
    let settings = state.settings();
    let cloud = settings.cloud.clone();
    let token_target = cloud
        .credential_target
        .clone()
        .ok_or_else(|| "connect to Clipline Cloud first".to_string())?;
    let token = read_credential(&token_target)?;
    let client = connected_client(&cloud, &token)?;
    let visibility = visibility
        .as_deref()
        .map(normalize_cloud_visibility)
        .unwrap_or_else(|| cloud.default_visibility.clone());
    let description = normalize_upload_description(description.as_deref());

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|e| format!("read clip: {e}"))?;
    let meta = std::fs::metadata(&target).map_err(|e| format!("read clip metadata: {e}"))?;
    if bytes.is_empty() {
        return Err("clip file is empty".into());
    }
    let checksum = sha256_hex(&bytes);
    let local_clip_id = local_clip_id(&target, &meta, &checksum)?;
    let mut record = CloudUploadRecord {
        local_clip_id: local_clip_id.clone(),
        // Store the path exactly as `list_clips` emits it (non-canonical), so the
        // UI can pair this record to its clip row by string equality. `target` is
        // the canonicalized form (`\\?\D:\…` on Windows) and is used only for I/O.
        path: path.clone(),
        remote_clip_id: None,
        remote_url: None,
        visibility: visibility.clone(),
        upload_status: existing_retry_status(&cloud, &local_clip_id),
        error: None,
        updated_at_unix: unix_now(),
    };
    persist_record(&state, &record)?;
    emit_upload_progress(&app, &record, 0, meta.len(), None);

    let markers = read_markers(&target);
    let request = create_upload_request(UploadRequestInput {
        path: &target,
        meta: &meta,
        bytes: &bytes,
        checksum: &checksum,
        visibility: &visibility,
        markers: markers.as_ref(),
        client_clip_id: &local_clip_id,
        title: title.as_deref(),
    })?;
    let upload_result = crate::cloud_upload::upload_mp4_bytes_with_progress(
        &client,
        &token,
        &request,
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
                path: path.clone(),
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
            emit_upload_progress(&app, &record, 0, meta.len(), record.error.clone());
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
        Ok(None) => None,
        Err(error) => return Err(cloud_error(error)),
    };

    if let Some(clip) = &clip {
        record.visibility = clip.visibility.clone();
        record.remote_url = clip
            .public_url
            .clone()
            .or_else(|| cloud_clip_url(&cloud, &clip.id));
        record.upload_status = if clip.visibility == "private" {
            "uploaded_private".to_string()
        } else {
            "uploaded_public".to_string()
        };
        record.updated_at_unix = unix_now();
        persist_record(&state, &record)?;
        emit_upload_progress(
            &app,
            &record,
            progress.file_size_bytes,
            progress.file_size_bytes,
            None,
        );

        if cloud.delete_local_after_upload {
            let _ = std::fs::remove_file(&target);
            let _ = std::fs::remove_file(target.with_extension("markers.json"));
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
        file_size_bytes: input.meta.len(),
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

fn read_markers(path: &Path) -> Option<ClipMarkers> {
    std::fs::read_to_string(path.with_extension("markers.json"))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
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

fn existing_retry_status(cloud: &CloudSettings, local_clip_id: &str) -> String {
    match cloud
        .uploads
        .get(local_clip_id)
        .map(|record| record.upload_status.as_str())
    {
        Some("failed") => "retrying".to_string(),
        Some("uploading") | Some("queued") | Some("processing") => "retrying".to_string(),
        _ => "queued".to_string(),
    }
}

fn persist_record(state: &RuntimeState, record: &CloudUploadRecord) -> Result<(), String> {
    state.update_cloud(|cloud| {
        cloud
            .uploads
            .insert(record.local_clip_id.clone(), record.clone());
    })?;
    Ok(())
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
    let mut target_w = wide_null(target);
    let mut username_w = wide_null(username);
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
    let target_w = wide_null(target);
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
    let target_w = wide_null(target);
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

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain([0]).collect()
}

fn serde_json_string<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn cloud_error(error: CloudApiError) -> String {
    error.to_string()
}

fn last_os_error(action: &str) -> String {
    format!("{action}: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
