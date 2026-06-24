use clipline_cloud_api::ClipDetailResponse;
use serde::{Deserialize, Serialize};

use crate::app::RuntimeState;
use crate::settings::{normalize_cloud_visibility, CloudSettings, CloudUploadRecord};

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
    connection_status(&state.settings().cloud)
}

#[tauri::command]
pub async fn cloud_connect(
    _state: tauri::State<'_, RuntimeState>,
    request: CloudConnectRequest,
) -> Result<CloudConnectionStatus, String> {
    let _ = (
        request.host_url,
        request.username,
        request.password,
        request.device_name,
        request.plain_http_confirmed,
        request
            .default_visibility
            .as_deref()
            .map(normalize_cloud_visibility),
    );
    Err("Clipline Cloud credential storage is unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub fn cloud_disconnect(
    state: tauri::State<RuntimeState>,
) -> Result<CloudConnectionStatus, String> {
    let settings = state.update_cloud(|cloud| {
        cloud.connected_user_id = None;
        cloud.connected_username = None;
        cloud.credential_target = None;
    })?;
    Ok(connection_status(&settings.cloud))
}

#[tauri::command]
pub async fn upload_clip_to_cloud<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    _state: tauri::State<'_, RuntimeState>,
    _storage: tauri::State<'_, crate::library::StorageSettings>,
    request: UploadClipCommandRequest,
) -> Result<CloudUploadResult, String> {
    let _ = (
        request.path,
        request.visibility,
        request.title,
        request.description,
        request.audio_track_ids,
    );
    Err("Clipline Cloud uploads are unavailable on macOS shell stubs".into())
}

#[tauri::command]
pub async fn sync_cloud_clip_status(
    _state: tauri::State<'_, RuntimeState>,
    request: SyncCloudClipStatusRequest,
) -> Result<CloudClipStatusSyncResult, String> {
    Ok(CloudClipStatusSyncResult {
        path: request.path,
        record: None,
        removed: false,
    })
}

fn connection_status(cloud: &CloudSettings) -> CloudConnectionStatus {
    CloudConnectionStatus {
        connected: cloud.connected(),
        token_present: cloud.credential_target.is_some(),
        host_url: cloud.host_url.clone(),
        public_url: cloud.public_url.clone(),
        username: cloud.connected_username.clone(),
        user_id: cloud.connected_user_id.clone(),
        default_visibility: cloud.default_visibility.clone(),
        delete_local_after_upload: cloud.delete_local_after_upload,
        auto_upload_rules: cloud.auto_upload_rules,
    }
}
