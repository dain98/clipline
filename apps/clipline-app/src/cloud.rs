//! Clipline Cloud desktop integration: connection state, OS credential storage,
//! and per-clip uploads through the first-party API client.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use clipline_cloud_api::{
    sha256_hex, ClipDetailResponse, ClipSummaryResponse, CloudApiError, CloudClient,
    CreateUploadRequest, ListClipsRequest,
};
use clipline_events::ClipMarkers;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::io::AsyncWriteExt;
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::app::RuntimeState;
use crate::library::{validate_clip_path, StorageSettings};
use crate::settings::{normalize_cloud_visibility, CloudSettings, CloudUploadRecord};
use crate::util::{last_os_error, unix_now, wide_null};

const DEFAULT_DEVICE_NAME: &str = "Clipline Desktop";
const READY_POLL_ATTEMPTS: usize = 30;
const READY_POLL_DELAY: Duration = Duration::from_secs(1);
const READY_MEDIA_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READY_MEDIA_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const CLOUD_LIBRARY_PAGE_SIZE: i64 = 100;
const CLOUD_UPLOAD_PROGRESS_EVENT: &str = "cloud-upload-progress";
const REMOTE_NOT_FOUND_SYNC_MARKER: &str = "remote clip not found during status sync";
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
const CLOUD_CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const UPLOAD_PAYLOAD_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const CLOUD_THUMBNAIL_MAX_BYTES: u64 = 10 * 1024 * 1024;
const CLOUD_MEDIA_FALLBACK_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const CLOUD_MEDIA_SIZE_SLACK_BYTES: u64 = 64 * 1024 * 1024;
static CLOUD_CACHE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static CLOUD_USER_AVATAR_CACHE: OnceLock<Mutex<Option<CachedCloudUserAvatar>>> = OnceLock::new();

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
    pub display_name: Option<String>,
    pub user_id: Option<String>,
    pub default_visibility: String,
    pub delete_local_after_upload: bool,
    pub auto_upload_rules: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct CloudUserProfile {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub profile_url: String,
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

#[derive(Debug)]
enum ReadyClipOutcome {
    Ready(ClipDetailResponse),
    Failed(ClipDetailResponse),
    TimedOut,
}

#[derive(Debug, Serialize)]
pub struct CloudClipStatusSyncResult {
    pub path: String,
    pub record: Option<CloudUploadRecord>,
    pub removed: bool,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct CloudLibraryClip {
    pub remote_clip_id: String,
    pub local_clip_id: Option<String>,
    pub path: String,
    pub title: String,
    pub remote_url: String,
    pub visibility: String,
    pub upload_status: String,
    pub updated_at_unix: u64,
    pub uploaded_at_unix: Option<u64>,
    pub duration_ms: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub source_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CloudLibraryListResult {
    pub clips: Vec<CloudLibraryClip>,
}

#[derive(Debug, Deserialize)]
pub struct CloudClipAssetRequest {
    pub remote_clip_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub file_size_bytes: Option<i64>,
    #[serde(default)]
    pub updated_at_unix: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CachedCloudClip {
    pub path: String,
    pub name: String,
    pub size_mb: f64,
    pub modified_unix: u64,
    pub duration_s: Option<f64>,
}

struct CloudAssetDownload<'a> {
    remote_clip_id: &'a str,
    asset: &'a str,
    extension: &'a str,
    version: Option<u64>,
    expected_size_bytes: Option<i64>,
    max_size_bytes: u64,
    missing_ok: bool,
}

#[derive(Clone)]
struct CachedCloudUserAvatar {
    key: String,
    etag: Option<String>,
    data_url: String,
}

#[tauri::command]
pub fn cloud_status(state: tauri::State<RuntimeState>) -> CloudConnectionStatus {
    let settings = state.settings();
    connection_status(&settings.cloud)
}

#[tauri::command]
pub async fn list_cloud_clips(
    state: tauri::State<'_, RuntimeState>,
) -> Result<CloudLibraryListResult, String> {
    let settings = state.settings();
    let cloud = settings.cloud.clone();
    let token_target = cloud
        .credential_target
        .clone()
        .ok_or_else(|| "connect to Clipline Cloud first".to_string())?;
    let token = read_credential(&token_target)?;
    let client = connected_client(&cloud, &token)?;

    let mut page = 1;
    let mut clips = Vec::new();
    loop {
        let response = client
            .list_clips(&ListClipsRequest {
                sort: Some("uploaded_at_desc".to_string()),
                page: Some(page),
                page_size: Some(CLOUD_LIBRARY_PAGE_SIZE),
                ..Default::default()
            })
            .await
            .map_err(cloud_error)?;
        let clip_count = response.clips.len();
        for clip in response.clips {
            let local_record = clip
                .client_clip_id
                .as_deref()
                .and_then(|local_clip_id| cloud.uploads.get(local_clip_id));
            clips.push(cloud_library_clip_from_summary(&cloud, &clip, local_record));
        }
        if clip_count < CLOUD_LIBRARY_PAGE_SIZE as usize {
            break;
        }
        page += 1;
    }

    Ok(CloudLibraryListResult { clips })
}

#[tauri::command]
pub async fn cloud_clip_thumbnail<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, RuntimeState>,
    request: CloudClipAssetRequest,
) -> Result<Option<String>, String> {
    let (cloud, token) = cloud_asset_context(&state)?;
    let Some(path) = download_cloud_asset_to_cache(
        &cloud,
        &token,
        CloudAssetDownload {
            remote_clip_id: &request.remote_clip_id,
            asset: "thumbnail",
            extension: "jpg",
            version: request.updated_at_unix,
            expected_size_bytes: None,
            max_size_bytes: CLOUD_THUMBNAIL_MAX_BYTES,
            missing_ok: true,
        },
    )
    .await?
    else {
        return Ok(None);
    };
    allow_cloud_cache_asset(&app, &path)?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub async fn cache_cloud_clip_media<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, RuntimeState>,
    request: CloudClipAssetRequest,
) -> Result<CachedCloudClip, String> {
    let (cloud, token) = cloud_asset_context(&state)?;
    let path = download_cloud_asset_to_cache(
        &cloud,
        &token,
        CloudAssetDownload {
            remote_clip_id: &request.remote_clip_id,
            asset: "media",
            extension: "mp4",
            version: request.updated_at_unix,
            expected_size_bytes: request.file_size_bytes,
            max_size_bytes: cloud_media_cache_max_bytes(request.file_size_bytes),
            missing_ok: false,
        },
    )
    .await?
    .ok_or_else(|| "cloud clip media is not available".to_string())?;
    allow_cloud_cache_asset(&app, &path)?;
    cached_cloud_clip_from_path(&path, &request)
}

#[tauri::command]
pub async fn cloud_user_avatar(
    state: tauri::State<'_, RuntimeState>,
) -> Result<Option<String>, String> {
    let (cloud, token) = cloud_asset_context(&state)?;
    let cache_key = cloud_user_avatar_cache_key(&cloud)?;
    let cached = cached_cloud_user_avatar(&cache_key);
    let url = cloud_user_avatar_url(&cloud)?;
    let mut request = reqwest::Client::new().get(url).bearer_auth(token);
    if let Some(etag) = cached.as_ref().and_then(|avatar| avatar.etag.as_deref()) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("download cloud avatar: {e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        clear_cached_cloud_user_avatar(&cache_key);
        return Ok(None);
    }
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(cached.map(|avatar| avatar.data_url));
    }
    if !status.is_success() {
        let message = response.text().await.unwrap_or_else(|_| status.to_string());
        return Err(format!(
            "download cloud avatar failed with {status}: {message}"
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length as usize > MAX_AVATAR_BYTES)
    {
        return Err("cloud avatar is too large".to_string());
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read cloud avatar: {e}"))?;
    let data_url = cloud_user_avatar_data_url(content_type.as_deref(), &bytes)?;
    store_cached_cloud_user_avatar(CachedCloudUserAvatar {
        key: cache_key,
        etag,
        data_url: data_url.clone(),
    });
    Ok(Some(data_url))
}

#[tauri::command]
pub async fn cloud_user_profile(
    state: tauri::State<'_, RuntimeState>,
) -> Result<CloudUserProfile, String> {
    let (cloud, token) = cloud_asset_context(&state)?;
    let client = connected_client(&cloud, &token)?;
    let response = client.me().await.map_err(cloud_error)?;
    let profile = cloud_user_profile_from_response(&cloud, &response.user)?;
    let profile_for_settings = profile.clone();
    let _settings = state.update_cloud(|cloud| {
        cloud.connected_user_id = Some(profile_for_settings.user_id.clone());
        cloud.connected_username = Some(profile_for_settings.username.clone());
        cloud.connected_display_name = profile_for_settings.display_name.clone();
    })?;
    Ok(profile)
}

#[tauri::command]
pub fn open_cloud_user_profile(state: tauri::State<RuntimeState>) -> Result<(), String> {
    let cloud = state.settings().cloud;
    let username = cloud
        .connected_username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Clipline Cloud username is unknown".to_string())?;
    let url = cloud_user_profile_url(&cloud, username)?;
    open_cloud_url(url.as_str(), "cloud user profile")
}

#[tauri::command]
pub fn open_cloud_clip_url(url: String) -> Result<(), String> {
    let url = validate_cloud_link_url(&url)?;
    open_cloud_url(&url, "cloud clip URL")
}

fn open_cloud_url(url: &str, context: &str) -> Result<(), String> {
    let operation = wide_null(OsStr::new("open"));
    let target = wide_null(OsStr::new(url));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(format!("{context} failed with shell code {result:?}"));
    }
    Ok(())
}

fn validate_cloud_link_url(input: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(input).map_err(|e| format!("cloud clip URL is invalid: {e}"))?;
    match url.scheme() {
        "http" | "https" => Ok(url.to_string()),
        scheme => Err(format!("cloud clip URL scheme is not supported: {scheme}")),
    }
}

fn cloud_asset_context(
    state: &tauri::State<'_, RuntimeState>,
) -> Result<(CloudSettings, String), String> {
    let cloud = state.settings().cloud;
    let token_target = cloud
        .credential_target
        .as_deref()
        .ok_or_else(|| "Clipline Cloud is not connected".to_string())?;
    let token = read_credential(token_target)?;
    Ok((cloud, token))
}

async fn download_cloud_asset_to_cache(
    cloud: &CloudSettings,
    token: &str,
    request: CloudAssetDownload<'_>,
) -> Result<Option<PathBuf>, String> {
    prune_old_cloud_cache_files(&cloud_clip_cache_root_dir(), CLOUD_CACHE_MAX_AGE);
    let target = cloud_clip_cache_path(
        cloud,
        request.remote_clip_id,
        request.asset,
        request.extension,
        request.version,
    )?;
    if cached_asset_matches(&target, request.expected_size_bytes) {
        return Ok(Some(target));
    }
    let url = cloud_clip_asset_url(cloud, request.remote_clip_id, request.asset)?;
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create cloud cache: {e}"))?;
    }

    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("download cloud {}: {e}", request.asset))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND && request.missing_ok {
        return Ok(None);
    }
    if !status.is_success() {
        let message = response.text().await.unwrap_or_else(|_| status.to_string());
        return Err(format!(
            "download cloud {} failed with {status}: {message}",
            request.asset
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > request.max_size_bytes)
    {
        return Err(format!(
            "download cloud {} is too large (limit {:.1} MB)",
            request.asset,
            request.max_size_bytes as f64 / (1024.0 * 1024.0)
        ));
    }

    let tmp = cloud_clip_cache_tmp_path(&target)?;
    let mut response = response;
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| format!("create cloud cache file: {e}"))?;
    let mut written = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("read cloud {}: {e}", request.asset))?
    {
        written += chunk.len() as u64;
        if written > request.max_size_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!(
                "download cloud {} is too large (limit {:.1} MB)",
                request.asset,
                request.max_size_bytes as f64 / (1024.0 * 1024.0)
            ));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write cloud cache file: {e}"))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("flush cloud cache file: {e}"))?;
    drop(file);
    if written == 0 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(format!(
            "download cloud {} returned an empty body",
            request.asset
        ));
    }

    if target.exists() && !cached_asset_matches(&target, request.expected_size_bytes) {
        let _ = tokio::fs::remove_file(cloud_cache_marker_path(&target)).await;
        let _ = tokio::fs::remove_file(&target).await;
    }

    match tokio::fs::rename(&tmp, &target).await {
        Ok(()) => {
            write_cloud_cache_marker(&target, written).await?;
            Ok(Some(target))
        }
        Err(error) if target.exists() => {
            let _ = tokio::fs::remove_file(&tmp).await;
            if cached_asset_matches(&target, request.expected_size_bytes) {
                Ok(Some(target))
            } else {
                Err(format!("finalize cloud cache file: {error}"))
            }
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(format!("finalize cloud cache file: {error}"))
        }
    }
}

fn cloud_clip_asset_url(
    cloud: &CloudSettings,
    remote_clip_id: &str,
    asset: &str,
) -> Result<reqwest::Url, String> {
    let remote_clip_id = validate_cloud_cache_component(remote_clip_id, "remote clip id")?;
    let asset = validate_cloud_cache_component(asset, "cloud asset")?;
    let base =
        clipline_cloud_api::validate_cloud_host(&cloud.host_url, true).map_err(cloud_error)?;
    base.join(&format!("api/v1/clips/{remote_clip_id}/{asset}"))
        .map_err(|e| format!("cloud asset URL is invalid: {e}"))
}

fn cloud_user_avatar_url(cloud: &CloudSettings) -> Result<reqwest::Url, String> {
    let base =
        clipline_cloud_api::validate_cloud_host(&cloud.host_url, true).map_err(cloud_error)?;
    base.join("api/v1/me/avatar")
        .map_err(|e| format!("cloud avatar URL is invalid: {e}"))
}

fn cloud_user_profile_from_response(
    cloud: &CloudSettings,
    user: &clipline_cloud_api::UserResponse,
) -> Result<CloudUserProfile, String> {
    let display_name = user
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(CloudUserProfile {
        user_id: user.id.clone(),
        username: user.username.clone(),
        display_name,
        profile_url: cloud_user_profile_url(cloud, &user.username)?.to_string(),
    })
}

fn cloud_user_profile_url(cloud: &CloudSettings, username: &str) -> Result<reqwest::Url, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("Clipline Cloud username is unknown".to_string());
    }
    let base = cloud.public_url.as_deref().unwrap_or(&cloud.host_url);
    let mut url = clipline_cloud_api::validate_cloud_host(base, true).map_err(cloud_error)?;
    url = url
        .join("u/")
        .map_err(|e| format!("cloud user profile URL is invalid: {e}"))?;
    url.path_segments_mut()
        .map_err(|_| "cloud user profile URL cannot be a base".to_string())?
        .pop_if_empty()
        .push(username);
    Ok(url)
}

fn cloud_user_avatar_cache_key(cloud: &CloudSettings) -> Result<String, String> {
    let base =
        clipline_cloud_api::validate_cloud_host(&cloud.host_url, true).map_err(cloud_error)?;
    let user_id = cloud
        .connected_user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Clipline Cloud user is unknown".to_string())?;
    Ok(format!("{}|{user_id}", base.as_str().trim_end_matches('/')))
}

fn cloud_user_avatar_data_url(content_type: Option<&str>, bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("cloud avatar returned an empty body".to_string());
    }
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err("cloud avatar is too large".to_string());
    }
    let mime = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("image/jpeg")
        .to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return Err(format!("cloud avatar response is not an image: {mime}"));
    }
    Ok(format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    ))
}

fn cloud_user_avatar_cache() -> &'static Mutex<Option<CachedCloudUserAvatar>> {
    CLOUD_USER_AVATAR_CACHE.get_or_init(|| Mutex::new(None))
}

fn cached_cloud_user_avatar(key: &str) -> Option<CachedCloudUserAvatar> {
    cloud_user_avatar_cache()
        .lock()
        .ok()
        .and_then(|avatar| avatar.as_ref().filter(|cached| cached.key == key).cloned())
}

fn store_cached_cloud_user_avatar(avatar: CachedCloudUserAvatar) {
    if let Ok(mut cached) = cloud_user_avatar_cache().lock() {
        *cached = Some(avatar);
    }
}

fn clear_cached_cloud_user_avatar(key: &str) {
    if let Ok(mut cached) = cloud_user_avatar_cache().lock() {
        if cached.as_ref().is_some_and(|avatar| avatar.key == key) {
            *cached = None;
        }
    }
}

fn cloud_clip_cache_path(
    cloud: &CloudSettings,
    remote_clip_id: &str,
    asset: &str,
    extension: &str,
    version: Option<u64>,
) -> Result<PathBuf, String> {
    let remote_clip_id = validate_cloud_cache_component(remote_clip_id, "remote clip id")?;
    let asset = validate_cloud_cache_component(asset, "cloud asset")?;
    let extension = validate_cloud_cache_component(extension, "cloud asset extension")?;
    let version = version.unwrap_or(0);
    Ok(
        cloud_clip_cache_dir(cloud)?
            .join(format!("{remote_clip_id}-{asset}-{version}.{extension}")),
    )
}

fn cloud_clip_cache_dir(cloud: &CloudSettings) -> Result<PathBuf, String> {
    Ok(cloud_clip_cache_root_dir().join(cloud_cache_namespace(cloud)?))
}

fn cloud_clip_cache_root_dir() -> PathBuf {
    crate::settings::persistence::config_base().join("cloud-cache")
}

fn cloud_cache_namespace(cloud: &CloudSettings) -> Result<String, String> {
    let base =
        clipline_cloud_api::validate_cloud_host(&cloud.host_url, true).map_err(cloud_error)?;
    let account = cloud
        .connected_user_id
        .as_deref()
        .or(cloud.connected_username.as_deref())
        .or(cloud.credential_target.as_deref())
        .unwrap_or("anonymous")
        .trim();
    let key = format!("{}|{account}", base.as_str().trim_end_matches('/'));
    Ok(sha256_hex(key.as_bytes())[..16].to_string())
}

fn validate_cloud_cache_component<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(trimmed)
}

fn cached_asset_matches(path: &Path, expected_size_bytes: Option<i64>) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    if cloud_cache_marker_matches(path, meta.len()) {
        return true;
    }
    match expected_size_bytes {
        Some(expected) if expected > 0 => meta.len() == expected as u64,
        _ => true,
    }
}

fn cloud_cache_marker_path(path: &Path) -> PathBuf {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if extension.is_empty() {
        path.with_extension("ok")
    } else {
        path.with_extension(format!("{extension}.ok"))
    }
}

fn cloud_cache_marker_matches(path: &Path, size_bytes: u64) -> bool {
    std::fs::read_to_string(cloud_cache_marker_path(path))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        == Some(size_bytes)
}

async fn write_cloud_cache_marker(path: &Path, size_bytes: u64) -> Result<(), String> {
    tokio::fs::write(cloud_cache_marker_path(path), size_bytes.to_string())
        .await
        .map_err(|e| format!("write cloud cache marker: {e}"))
}

fn cloud_clip_cache_tmp_path(target: &Path) -> Result<PathBuf, String> {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "cloud cache target has no filename".to_string())?;
    let count = CLOUD_CACHE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(target.with_file_name(format!("{file_name}.{}.{}.tmp", std::process::id(), count)))
}

fn cloud_media_cache_max_bytes(expected_size_bytes: Option<i64>) -> u64 {
    expected_size_bytes
        .filter(|bytes| *bytes > 0)
        .map(|bytes| {
            (bytes as u64)
                .saturating_mul(2)
                .saturating_add(CLOUD_MEDIA_SIZE_SLACK_BYTES)
        })
        .unwrap_or(CLOUD_MEDIA_FALLBACK_MAX_BYTES)
}

fn prune_old_cloud_cache_files(cache_dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune_old_cloud_cache_files(&path, max_age);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let is_tmp = file_name.ends_with(".tmp");
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= max_age);
        if is_tmp || old {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn allow_cloud_cache_asset<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), String> {
    let cache_dir = crate::settings::persistence::config_base().join("cloud-cache");
    let canonical_dir = cache_dir
        .canonicalize()
        .map_err(|e| format!("canonicalize cloud cache {cache_dir:?}: {e}"))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("canonicalize cloud cache asset {path:?}: {e}"))?;
    if !canonical_path.starts_with(&canonical_dir) {
        return Err(format!(
            "cloud cache asset {canonical_path:?} escaped cache {canonical_dir:?}"
        ));
    }
    app.asset_protocol_scope()
        .allow_directory(&canonical_dir, true)
        .map_err(|e| format!("scope cloud cache for playback: {e}"))
}

fn cached_cloud_clip_from_path(
    path: &Path,
    request: &CloudClipAssetRequest,
) -> Result<CachedCloudClip, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("read cached cloud clip: {e}"))?;
    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or_else(unix_now);
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Cloud clip");
    let name = if title.to_ascii_lowercase().ends_with(".mp4") {
        title.to_string()
    } else {
        format!("{title}.mp4")
    };
    Ok(CachedCloudClip {
        path: path.display().to_string(),
        name,
        size_mb: meta.len() as f64 / (1024.0 * 1024.0),
        modified_unix,
        duration_s: request
            .duration_ms
            .filter(|duration| *duration >= 0)
            .map(|duration| duration as f64 / 1000.0),
    })
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
        cloud.connected_display_name = connected
            .user
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
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
        cloud.connected_display_name = None;
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
    let payload = upload_payload_for_audio_selection_from_path(
        &target,
        markers.as_ref(),
        request.audio_track_ids.as_deref(),
    )
    .await?;
    let payload_meta = tokio::fs::metadata(payload.path())
        .await
        .map_err(|e| format!("read upload payload metadata: {e}"))?;
    let payload_size = payload_meta.len();
    let checksum = crate::cloud_upload::sha256_file(payload.path())
        .await
        .map_err(cloud_error)?;
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
    emit_upload_progress(&app, &record, 0, payload_size, None);

    let upload_request = create_upload_request(UploadRequestInput {
        path: &target,
        meta: &meta,
        file_size_bytes: payload_size,
        duration_ms: clip_duration_ms_file(payload.path(), markers.as_ref()),
        checksum: &checksum,
        visibility: &visibility,
        markers: markers.as_ref(),
        client_clip_id: &local_clip_id,
        title: request.title.as_deref(),
    })?;
    let progress_path = request.path.clone();
    let upload_result = crate::cloud_upload::upload_mp4_file_with_progress(
        &client,
        &token,
        &upload_request,
        description.as_deref(),
        payload.path(),
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
            emit_upload_progress(&app, &record, 0, payload_size, record.error.clone());
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
        Ok(ReadyClipOutcome::Ready(clip)) => clip,
        Ok(ReadyClipOutcome::Failed(clip)) => {
            apply_remote_clip_to_record(&cloud, &mut record, &clip);
            record.upload_status = "failed".to_string();
            record.error = Some(
                "cloud upload completed, but cloud media processing failed; the local clip was preserved"
                    .to_string(),
            );
            record.updated_at_unix = unix_now();
            persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
            return Ok(CloudUploadResult { record, clip: None });
        }
        Ok(ReadyClipOutcome::TimedOut) => {
            mark_ready_timeout(&mut record);
            persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
            return Ok(CloudUploadResult { record, clip: None });
        }
        Err(error) => {
            mark_post_upload_problem(
                &mut record,
                format!(
                    "cloud upload completed, but checking cloud processing failed: {}; the local clip was preserved",
                    cloud_error(error)
                ),
            );
            persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
            return Ok(CloudUploadResult { record, clip: None });
        }
    };

    let clip = if visibility == "private" {
        clip
    } else {
        match client.set_visibility(&clip.id, visibility.clone()).await {
            Ok(updated) if updated.status == "ready" => updated,
            Ok(updated) => {
                apply_remote_clip_to_record(&cloud, &mut record, &updated);
                mark_post_upload_problem(
                    &mut record,
                    format!(
                        "cloud upload completed, but visibility update returned status {:?}; the local clip was preserved",
                        updated.status
                    ),
                );
                persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
                return Ok(CloudUploadResult { record, clip: None });
            }
            Err(error) => {
                mark_post_upload_problem(
                    &mut record,
                    format!(
                        "cloud upload completed, but updating visibility failed: {}; the local clip was preserved",
                        cloud_error(error)
                    ),
                );
                persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
                return Ok(CloudUploadResult { record, clip: None });
            }
        }
    };

    apply_remote_clip_to_record(&cloud, &mut record, &clip);
    persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;

    if cloud.delete_local_after_upload {
        if let Err(error) = verify_ready_cloud_media(&cloud, &token, &clip.id).await {
            mark_post_upload_problem(
                &mut record,
                format!(
                    "cloud reported the upload ready, but its media could not be verified: {error}; the local clip was preserved"
                ),
            );
            persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
            return Ok(CloudUploadResult {
                record,
                clip: Some(clip),
            });
        }
        if let Err(error) = delete_uploaded_local_files(&target) {
            record.error = Some(format!(
                "cloud upload is ready, but local cleanup failed: {error}"
            ));
            record.updated_at_unix = unix_now();
            persist_post_upload_record(&app, &state, &record, progress.file_size_bytes)?;
        }
    }

    Ok(CloudUploadResult {
        record,
        clip: Some(clip),
    })
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
        display_name: cloud.connected_display_name.clone(),
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
    file_size_bytes: u64,
    duration_ms: Option<i64>,
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
        description: None,
        game_name: game.as_ref().map(|game| game.name.clone()),
        game_id: game.as_ref().map(|game| game.id.clone()),
        game_executable: None,
        source_type: Some(source_type(input.path)),
        recorded_at: input.meta.modified().ok().map(DateTime::<Utc>::from),
        duration_ms: input.duration_ms,
        file_size_bytes: input.file_size_bytes,
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
        .unwrap_or_else(|| crate::library::clip_title_for_path(path))
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
    Mix(Vec<u32>),
}

struct UploadPayload {
    path: PathBuf,
    owned: bool,
}

impl UploadPayload {
    fn original(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            owned: false,
        }
    }

    fn owned(path: PathBuf) -> Self {
        Self { path, owned: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UploadPayload {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

async fn upload_payload_for_audio_selection_from_path(
    source_path: &Path,
    markers: Option<&ClipMarkers>,
    selected_audio_track_ids: Option<&[String]>,
) -> Result<UploadPayload, String> {
    let markers_with_audio = selected_audio_track_ids.and_then(|_| {
        crate::util::markers_with_inferred_audio_tracks(source_path, markers.cloned())
    });
    let selection_markers = markers_with_audio.as_ref().or(markers);
    match upload_audio_selection_plan(selection_markers, selected_audio_track_ids)? {
        UploadAudioSelectionPlan::Original => Ok(UploadPayload::original(source_path)),
        UploadAudioSelectionPlan::Remux(selected_indices) => {
            let target = reserve_upload_payload_path(source_path)?;
            let payload = UploadPayload::owned(target.clone());
            let source = source_path.to_path_buf();
            tokio::task::spawn_blocking(move || {
                clipline_mp4::remux_with_selected_audio_tracks_file(
                    &source,
                    &target,
                    &selected_indices,
                )
            })
            .await
            .map_err(|e| format!("audio remux task failed: {e}"))?
            .map_err(|e| e.to_string())?;
            Ok(payload)
        }
        UploadAudioSelectionPlan::Mix(selected_indices) => {
            let target = reserve_upload_payload_path(source_path)?;
            let payload = UploadPayload::owned(target.clone());
            let source = source_path.to_path_buf();
            tokio::task::spawn_blocking(move || {
                clipline_mp4::remux_with_mixed_audio_track_file(&source, &target, &selected_indices)
            })
            .await
            .map_err(|e| format!("audio mix task failed: {e}"))?
            .map_err(|e| e.to_string())?;
            Ok(payload)
        }
    }
}

fn reserve_upload_payload_path(source: &Path) -> Result<PathBuf, String> {
    let file_name = source
        .file_name()
        .ok_or_else(|| "clip path must include a file name".to_string())?;
    let parent = source
        .parent()
        .ok_or_else(|| "clip path must include a parent directory".to_string())?;
    prune_abandoned_upload_payloads(parent);
    for _ in 0..128 {
        let suffix = CLOUD_CACHE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = file_name.to_os_string();
        name.push(format!(
            ".clipline-upload-{}-{suffix}.tmp",
            std::process::id()
        ));
        let path = source.with_file_name(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                drop(file);
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("reserve upload payload: {error}")),
        }
    }
    Err("could not reserve a unique upload payload path".into())
}

fn prune_abandoned_upload_payloads(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_upload_temp = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".clipline-upload-") && name.ends_with(".tmp"));
        let abandoned = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= UPLOAD_PAYLOAD_MAX_AGE);
        if is_upload_temp && abandoned {
            let _ = std::fs::remove_file(path);
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
        UploadAudioSelectionPlan::Mix(selected_indices) => {
            clipline_mp4::remux_with_mixed_audio_track(&source_bytes, &selected_indices)
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
    if selected_indices.len() > 1 {
        Ok(UploadAudioSelectionPlan::Mix(selected_indices))
    } else {
        Ok(UploadAudioSelectionPlan::Remux(selected_indices))
    }
}

fn read_clip_game(path: &Path, markers: Option<&ClipMarkers>) -> Option<crate::library::ClipGame> {
    path.parent()
        .and_then(|dir| std::fs::read_to_string(dir.join("clipline-session.json")).ok())
        .and_then(|json| serde_json::from_str::<crate::library::ClipGame>(&json).ok())
        .or_else(|| markers.and_then(game_from_markers))
}

fn game_from_markers(markers: &ClipMarkers) -> Option<crate::library::ClipGame> {
    let game_id = markers.markers.first()?.event.game_id;
    let id = crate::game_plugins::plugin_id_for_game_id(game_id);
    Some(crate::library::ClipGame {
        id: id.to_string(),
        name: crate::game_plugins::display_name_for_game_id(game_id).to_string(),
    })
}

fn clip_duration_ms_file(path: &Path, markers: Option<&ClipMarkers>) -> Option<i64> {
    clipline_mp4::movie_duration_s_file(path)
        .ok()
        .flatten()
        .or_else(|| markers.map(|markers| markers.duration_s))
        .map(|seconds| (seconds * 1000.0).round())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as i64)
}

fn source_type(path: &Path) -> String {
    crate::library::clip_kind_for_path(path)
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
        "cloud upload completed, but cloud processing did not become ready within {} seconds; the local clip was preserved and the cloud link may finish processing shortly",
        READY_POLL_ATTEMPTS as u64 * READY_POLL_DELAY.as_secs()
    ));
    record.updated_at_unix = unix_now();
}

fn mark_post_upload_problem(record: &mut CloudUploadRecord, message: String) {
    record.upload_status = "uploaded_processing".to_string();
    record.error = Some(message);
    record.updated_at_unix = unix_now();
}

fn persist_post_upload_record<R: Runtime>(
    app: &AppHandle<R>,
    state: &RuntimeState,
    record: &CloudUploadRecord,
    file_size_bytes: u64,
) -> Result<(), String> {
    persist_record(state, record)?;
    emit_upload_progress(
        app,
        record,
        file_size_bytes,
        file_size_bytes,
        record.error.clone(),
    );
    Ok(())
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

fn cloud_library_clip_from_summary(
    cloud: &CloudSettings,
    clip: &ClipSummaryResponse,
    local_record: Option<&CloudUploadRecord>,
) -> CloudLibraryClip {
    CloudLibraryClip {
        remote_clip_id: clip.id.clone(),
        local_clip_id: clip.client_clip_id.clone(),
        path: local_record
            .map(|record| record.path.clone())
            .unwrap_or_default(),
        title: clip.title.clone(),
        remote_url: clip
            .public_url
            .clone()
            .or_else(|| cloud_clip_url(cloud, &clip.id))
            .unwrap_or_default(),
        visibility: clip.visibility.clone(),
        upload_status: upload_status_for_summary_clip(clip),
        updated_at_unix: datetime_to_unix_seconds(clip.updated_at),
        uploaded_at_unix: clip.uploaded_at.map(datetime_to_unix_seconds),
        duration_ms: clip.duration_ms,
        file_size_bytes: clip.file_size_bytes,
        source_type: clip.source_type.clone(),
    }
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

fn upload_status_for_summary_clip(clip: &ClipSummaryResponse) -> String {
    match clip.status.as_str() {
        "failed" => "failed".to_string(),
        "ready" if clip.visibility == "private" => "uploaded_private".to_string(),
        "ready" => "uploaded_public".to_string(),
        _ => "uploaded_processing".to_string(),
    }
}

fn datetime_to_unix_seconds(value: DateTime<Utc>) -> u64 {
    value.timestamp().max(0) as u64
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

fn delete_uploaded_local_files(target: &Path) -> std::io::Result<()> {
    std::fs::remove_file(target).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("delete uploaded local clip {target:?}: {error}"),
        )
    })?;
    // Sidecars may not exist — ignore missing-file errors.
    let mut first_error = None;
    for sidecar in crate::library::clip_sidecar_paths(target) {
        if let Err(error) = std::fs::remove_file(&sidecar) {
            if error.kind() != std::io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(std::io::Error::new(
                    error.kind(),
                    format!("delete uploaded clip sidecar {sidecar:?}: {error}"),
                ));
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
) -> Result<ReadyClipOutcome, CloudApiError> {
    wait_for_ready_clip_with_policy(client, clip_id, READY_POLL_ATTEMPTS, READY_POLL_DELAY).await
}

async fn wait_for_ready_clip_with_policy(
    client: &CloudClient,
    clip_id: &str,
    attempts: usize,
    delay: Duration,
) -> Result<ReadyClipOutcome, CloudApiError> {
    for attempt in 0..attempts {
        match client.get_clip(clip_id).await {
            Ok(clip) if clip.status == "ready" => return Ok(ReadyClipOutcome::Ready(clip)),
            Ok(clip) if clip.status == "failed" => return Ok(ReadyClipOutcome::Failed(clip)),
            Ok(_)
            | Err(CloudApiError::Api {
                status: reqwest::StatusCode::NOT_FOUND,
                ..
            }) => {}
            Err(error) => return Err(error),
        }
        if attempt + 1 < attempts && !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
    Ok(ReadyClipOutcome::TimedOut)
}

async fn verify_ready_cloud_media(
    cloud: &CloudSettings,
    token: &str,
    remote_clip_id: &str,
) -> Result<(), String> {
    let url = cloud_clip_asset_url(cloud, remote_clip_id, "media")?;
    let client = reqwest::Client::builder()
        .connect_timeout(READY_MEDIA_PROBE_CONNECT_TIMEOUT)
        .timeout(READY_MEDIA_PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("create media verification client: {error}"))?;
    let mut response = client
        .get(url)
        .bearer_auth(token)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| format!("request ready cloud media: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "ready cloud media returned HTTP {}",
            response.status()
        ));
    }
    let first_chunk = response
        .chunk()
        .await
        .map_err(|error| format!("read ready cloud media: {error}"))?;
    if first_chunk.as_ref().is_none_or(|bytes| bytes.is_empty()) {
        return Err("ready cloud media returned no bytes".to_string());
    }
    Ok(())
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

fn cloud_error(error: CloudApiError) -> String {
    error.to_string()
}

fn cloud_error_is_not_found(error: &CloudApiError) -> bool {
    matches!(error, CloudApiError::Api { status, .. } if status.as_u16() == 404)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_cloud_api::ClipSummaryResponse;
    use clipline_events::ClipAudioTrack;
    use clipline_mp4::{
        AudioTrackConfig, FragSample, HybridMp4Writer, TrackConfig, VideoTrackConfig,
    };
    use clipline_test_utils::TestDir;
    use httpmock::prelude::*;
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
        assert_eq!(source_type(Path::new("full-session.mp4")), "replay");
        assert_eq!(source_type(Path::new("ranked-trim.mp4")), "replay");
        assert_eq!(source_type(Path::new("session_1781377615.mp4")), "session");
        assert_eq!(
            source_type(Path::new("clip_1_trim_001000_002000.mp4")),
            "trim"
        );
    }

    #[test]
    fn upload_metadata_uses_clip_title_and_kind_sidecar() {
        let dir = TestDir::new("clipline-cloud", "clip-metadata-upload");
        let clip = dir.path().join("Ranked win.mp4");
        std::fs::write(&clip, b"mp4").unwrap();
        std::fs::write(
            clip.with_extension("clipline.json"),
            r#"{"title":"Ranked win vs Lux","kind":"session"}"#,
        )
        .unwrap();

        assert_eq!(upload_title(None, &clip), "Ranked win vs Lux");
        assert_eq!(source_type(&clip), "session");
    }

    #[test]
    fn upload_audio_selection_plan_mixes_multiple_selected_tracks() {
        let markers = audio_markers();
        let selected = vec!["output".to_string(), "microphone".to_string()];

        assert_eq!(
            upload_audio_selection_plan(Some(&markers), Some(&selected)).unwrap(),
            UploadAudioSelectionPlan::Mix(vec![0, 1])
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
    fn owned_upload_payload_is_removed_but_original_is_preserved() {
        let dir = TestDir::new("clipline-cloud", "upload-payload-ownership");
        let original = dir.path().join("original.mp4");
        let temporary = dir.path().join("temporary.mp4");
        std::fs::write(&original, b"original").unwrap();
        std::fs::write(&temporary, b"temporary").unwrap();

        drop(UploadPayload::original(&original));
        drop(UploadPayload::owned(temporary.clone()));

        assert!(original.exists());
        assert!(!temporary.exists());
    }

    #[tokio::test]
    async fn selected_audio_upload_uses_and_cleans_file_backed_payload() {
        let dir = TestDir::new("clipline-cloud", "selected-upload-payload");
        let source = dir.path().join("source.mp4");
        std::fs::write(&source, two_audio_mp4()).unwrap();
        let markers = audio_markers();
        let selected = vec!["microphone".to_string()];

        let payload =
            upload_payload_for_audio_selection_from_path(&source, Some(&markers), Some(&selected))
                .await
                .unwrap();
        let payload_path = payload.path().to_path_buf();
        let payload_bytes = std::fs::read(&payload_path).unwrap();

        assert_ne!(payload_path, source);
        assert!(payload_bytes.windows(6).any(|window| window == b"V00000"));
        assert!(!payload_bytes.windows(6).any(|window| window == b"A00000"));
        assert!(payload_bytes.windows(6).any(|window| window == b"B00000"));
        drop(payload);
        assert!(!payload_path.exists());
        assert!(source.exists());
    }

    #[test]
    fn abandoned_upload_payload_prune_is_scoped_and_age_gated() {
        let dir = TestDir::new("clipline-cloud", "upload-payload-prune");
        let abandoned = dir.path().join("clip.mp4.clipline-upload-1-1.tmp");
        let active = dir.path().join("clip.mp4.clipline-upload-1-2.tmp");
        let unrelated = dir.path().join("editor.tmp");
        for path in [&abandoned, &active, &unrelated] {
            std::fs::write(path, b"temp").unwrap();
        }
        std::fs::File::options()
            .write(true)
            .open(&abandoned)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();

        prune_abandoned_upload_payloads(dir.path());

        assert!(!abandoned.exists());
        assert!(active.exists());
        assert!(unrelated.exists());
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

    #[tokio::test]
    async fn readiness_poll_does_not_accept_processing_as_ready() {
        let server = MockServer::start();
        let response = clip_detail("remote-1", "private", "processing", None);
        let request = server.mock(|when, then| {
            when.method(GET).path("/api/v1/clips/remote-1");
            then.status(200)
                .header("content-type", "application/json")
                .json_body_obj(&response);
        });

        let outcome = wait_for_ready_clip_with_policy(
            &test_cloud_client(&server),
            "remote-1",
            3,
            Duration::ZERO,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ReadyClipOutcome::TimedOut));
        request.assert_hits(3);
    }

    #[tokio::test]
    async fn readiness_poll_treats_remote_processing_failure_as_terminal() {
        let server = MockServer::start();
        let response = clip_detail("remote-1", "private", "failed", None);
        let request = server.mock(|when, then| {
            when.method(GET).path("/api/v1/clips/remote-1");
            then.status(200)
                .header("content-type", "application/json")
                .json_body_obj(&response);
        });

        let outcome = wait_for_ready_clip_with_policy(
            &test_cloud_client(&server),
            "remote-1",
            3,
            Duration::ZERO,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ReadyClipOutcome::Failed(clip) if clip.status == "failed"));
        request.assert_hits(1);
    }

    #[tokio::test]
    async fn readiness_poll_returns_only_an_explicitly_ready_clip() {
        let server = MockServer::start();
        let response = clip_detail("remote-1", "private", "ready", None);
        let request = server.mock(|when, then| {
            when.method(GET).path("/api/v1/clips/remote-1");
            then.status(200)
                .header("content-type", "application/json")
                .json_body_obj(&response);
        });

        let outcome = wait_for_ready_clip_with_policy(
            &test_cloud_client(&server),
            "remote-1",
            3,
            Duration::ZERO,
        )
        .await
        .unwrap();

        assert!(matches!(outcome, ReadyClipOutcome::Ready(clip) if clip.status == "ready"));
        request.assert_hits(1);
    }

    #[tokio::test]
    async fn ready_media_probe_requires_retrievable_nonempty_content() {
        let server = MockServer::start();
        let media = server.mock(|when, then| {
            when.method(GET)
                .path("/api/v1/clips/remote-1/media")
                .header("authorization", "Bearer token")
                .header("range", "bytes=0-0");
            then.status(206).body("x");
        });
        let cloud = CloudSettings {
            host_url: server.base_url(),
            ..CloudSettings::default()
        };

        verify_ready_cloud_media(&cloud, "token", "remote-1")
            .await
            .unwrap();

        media.assert_hits(1);
    }

    #[tokio::test]
    async fn ready_media_probe_rejects_empty_and_failed_responses() {
        let empty_server = MockServer::start();
        empty_server.mock(|when, then| {
            when.method(GET).path("/api/v1/clips/remote-1/media");
            then.status(206);
        });
        let empty_cloud = CloudSettings {
            host_url: empty_server.base_url(),
            ..CloudSettings::default()
        };
        let empty_error = verify_ready_cloud_media(&empty_cloud, "token", "remote-1")
            .await
            .expect_err("empty media is not durable");
        assert!(empty_error.contains("no bytes"), "{empty_error}");

        let failed_server = MockServer::start();
        failed_server.mock(|when, then| {
            when.method(GET).path("/api/v1/clips/remote-1/media");
            then.status(404);
        });
        let failed_cloud = CloudSettings {
            host_url: failed_server.base_url(),
            ..CloudSettings::default()
        };
        let failed_error = verify_ready_cloud_media(&failed_cloud, "token", "remote-1")
            .await
            .expect_err("missing media is not durable");
        assert!(failed_error.contains("404"), "{failed_error}");
    }

    #[test]
    fn post_upload_problem_keeps_remote_identity_for_reconciliation() {
        let mut record = upload_record("local", "D:\\Videos\\clip.mp4", "processing", 10);
        record.remote_clip_id = Some("remote-1".into());
        record.remote_url = Some("https://clips.example.com/clip/remote-1".into());

        mark_post_upload_problem(&mut record, "visibility update failed".into());

        assert_eq!(record.upload_status, "uploaded_processing");
        assert_eq!(record.remote_clip_id.as_deref(), Some("remote-1"));
        assert_eq!(
            record.remote_url.as_deref(),
            Some("https://clips.example.com/clip/remote-1")
        );
        assert_eq!(record.error.as_deref(), Some("visibility update failed"));
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
    fn cloud_summary_maps_to_library_clip_with_client_id_and_owned_url() {
        let cloud = CloudSettings {
            public_url: Some("https://clips.example.com".into()),
            ..CloudSettings::default()
        };
        let local = upload_record("local-1", "D:\\Videos\\known.mp4", "uploaded_public", 10);

        let entry = cloud_library_clip_from_summary(
            &cloud,
            &clip_summary(
                "remote-1",
                Some("local-1"),
                "Server Title",
                "private",
                "ready",
                None,
            ),
            Some(&local),
        );

        assert_eq!(entry.remote_clip_id, "remote-1");
        assert_eq!(entry.local_clip_id.as_deref(), Some("local-1"));
        assert_eq!(entry.path, "D:\\Videos\\known.mp4");
        assert_eq!(entry.title, "Server Title");
        assert_eq!(entry.remote_url, "https://clips.example.com/clip/remote-1");
        assert_eq!(entry.visibility, "private");
        assert_eq!(entry.upload_status, "uploaded_private");
        assert_eq!(entry.source_type.as_deref(), Some("replay"));
        assert!(entry.updated_at_unix > 0);
    }

    #[test]
    fn cloud_link_url_accepts_only_http_or_https() {
        assert_eq!(
            validate_cloud_link_url("https://clips.example.com/clip/remote-1").as_deref(),
            Ok("https://clips.example.com/clip/remote-1")
        );
        assert_eq!(
            validate_cloud_link_url("http://localhost:8080/clip/remote-1").as_deref(),
            Ok("http://localhost:8080/clip/remote-1")
        );
        assert!(validate_cloud_link_url("file:///C:/Windows/win.ini").is_err());
        assert!(validate_cloud_link_url("clipline://remote-1").is_err());
    }

    #[test]
    fn cloud_clip_asset_url_uses_api_host_and_safe_clip_ids() {
        let cloud = CloudSettings {
            host_url: "https://clips.example.com/base".into(),
            ..CloudSettings::default()
        };
        let url = cloud_clip_asset_url(&cloud, "remote-1_ABC", "media").expect("asset URL");
        assert_eq!(
            url.as_str(),
            "https://clips.example.com/base/api/v1/clips/remote-1_ABC/media"
        );
        assert!(cloud_clip_asset_url(&cloud, "../escape", "media").is_err());
        assert!(cloud_clip_asset_url(&cloud, "remote/escape", "thumbnail").is_err());
    }

    #[test]
    fn cloud_clip_cache_path_keeps_remote_ids_inside_cache() {
        let cloud = CloudSettings {
            host_url: "https://clips.example.com".into(),
            connected_user_id: Some("user-1".into()),
            ..CloudSettings::default()
        };
        let path = cloud_clip_cache_path(&cloud, "remote-1_ABC", "media", "mp4", Some(42))
            .expect("cache path");
        assert!(path.ends_with("remote-1_ABC-media-42.mp4"));
        assert!(cloud_clip_cache_path(&cloud, "../escape", "media", "mp4", None).is_err());
        assert!(cloud_clip_cache_path(&cloud, "remote-1", "../asset", "mp4", None).is_err());
    }

    #[test]
    fn cloud_clip_cache_path_is_namespaced_by_account() {
        let first = CloudSettings {
            host_url: "https://clips.example.com".into(),
            connected_user_id: Some("user-1".into()),
            ..CloudSettings::default()
        };
        let second = CloudSettings {
            host_url: "https://clips.example.com".into(),
            connected_user_id: Some("user-2".into()),
            ..CloudSettings::default()
        };

        let first_path =
            cloud_clip_cache_path(&first, "remote-1", "media", "mp4", Some(1)).unwrap();
        let second_path =
            cloud_clip_cache_path(&second, "remote-1", "media", "mp4", Some(1)).unwrap();

        assert_ne!(first_path.parent(), second_path.parent());
        assert_eq!(
            first_path.file_name().and_then(|name| name.to_str()),
            second_path.file_name().and_then(|name| name.to_str())
        );
    }

    #[test]
    fn cached_asset_marker_accepts_actual_download_size() {
        let dir = TestDir::new("clipline-cloud", "cached-asset-marker");
        let asset = dir.path().join("remote-media-42.mp4");
        std::fs::write(&asset, b"served bytes").unwrap();
        std::fs::write(cloud_cache_marker_path(&asset), b"12").unwrap();

        assert!(
            cached_asset_matches(&asset, Some(999)),
            "a completed cloud-cache download should not be invalidated by a stale server size"
        );
    }

    #[test]
    fn prune_cloud_cache_removes_old_and_tmp_files() {
        let dir = TestDir::new("clipline-cloud", "cloud-cache-prune");
        let keep = dir.path().join("keep.mp4");
        let old = dir.path().join("old.mp4");
        let tmp = dir.path().join("new.mp4.1.tmp");
        let nested = dir.path().join("account").join("nested.mp4");
        std::fs::write(&keep, b"keep").unwrap();
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&tmp, b"tmp").unwrap();
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, b"nested").unwrap();

        prune_old_cloud_cache_files(dir.path(), Duration::ZERO);

        assert!(!old.exists());
        assert!(!tmp.exists());
        assert!(!keep.exists());
        assert!(!nested.exists());
    }

    #[test]
    fn cloud_user_avatar_url_uses_api_host() {
        let cloud = CloudSettings {
            host_url: "https://clips.example.com/base".into(),
            ..CloudSettings::default()
        };
        let url = cloud_user_avatar_url(&cloud).expect("avatar URL");
        assert_eq!(
            url.as_str(),
            "https://clips.example.com/base/api/v1/me/avatar"
        );
    }

    #[test]
    fn cloud_user_profile_url_uses_public_url_and_escapes_username() {
        let cloud = CloudSettings {
            host_url: "https://api.example.com/base".into(),
            public_url: Some("https://clips.example.com/cloud".into()),
            ..CloudSettings::default()
        };
        let url = cloud_user_profile_url(&cloud, "Dain 98").expect("profile URL");
        assert_eq!(url.as_str(), "https://clips.example.com/cloud/u/Dain%2098");
    }

    #[test]
    fn cloud_connection_status_includes_display_name() {
        let cloud = CloudSettings {
            connected_display_name: Some("Dain".into()),
            connected_username: Some("dain98".into()),
            ..CloudSettings::default()
        };

        let status = connection_status(&cloud);

        assert_eq!(status.display_name.as_deref(), Some("Dain"));
        assert_eq!(status.username.as_deref(), Some("dain98"));
    }

    #[test]
    fn cloud_user_avatar_data_url_requires_image_content_type() {
        assert_eq!(
            cloud_user_avatar_data_url(Some("image/png"), b"\x01\x02\x03").unwrap(),
            "data:image/png;base64,AQID"
        );
        assert!(
            cloud_user_avatar_data_url(Some("text/html"), b"<script>").is_err(),
            "avatar data URLs must only accept image responses"
        );
        assert!(
            cloud_user_avatar_data_url(Some("image/png"), b"").is_err(),
            "empty avatar bodies should not render as broken images"
        );
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
        let metadata = clip.with_extension("clipline.json");
        let pending_osu = clip.with_extension("osu-enrichment.json");
        let poster = crate::poster::poster_path(&clip);
        std::fs::write(&clip, b"mp4").unwrap();
        std::fs::write(&markers, b"{}").unwrap();
        std::fs::write(&metadata, b"{}").unwrap();
        std::fs::write(&pending_osu, b"{}").unwrap();
        std::fs::write(&poster, b"jpg").unwrap();

        delete_uploaded_local_files(&clip).unwrap();

        assert!(!clip.exists());
        assert!(!markers.exists());
        assert!(!metadata.exists());
        assert!(!pending_osu.exists());
        assert!(!poster.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_cleanup_preserves_sidecars_when_primary_deletion_fails() {
        let dir = TestDir::new("clipline-cloud", "delete-primary-first");
        let clip = dir.path().join("clip.mp4");
        let markers = clip.with_extension("markers.json");
        std::fs::create_dir(&clip).unwrap();
        std::fs::write(&markers, b"{}").unwrap();

        delete_uploaded_local_files(&clip).expect_err("a directory is not a removable MP4 file");

        assert!(clip.exists());
        assert!(markers.exists());
    }

    #[test]
    fn local_cleanup_reports_sidecar_failure_after_primary_deletion() {
        let dir = TestDir::new("clipline-cloud", "delete-sidecar-error");
        let clip = dir.path().join("clip.mp4");
        let markers = clip.with_extension("markers.json");
        std::fs::write(&clip, b"mp4").unwrap();
        std::fs::create_dir(&markers).unwrap();

        let error = delete_uploaded_local_files(&clip).expect_err("sidecar directory must fail");

        assert!(!clip.exists(), "primary deletion happens before sidecars");
        assert!(markers.exists());
        assert!(error.to_string().contains("sidecar"), "{error}");
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
            plays: Vec::new(),
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
            description: None,
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
            view_count: 0,
            markers: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_cloud_client(server: &MockServer) -> CloudClient {
        CloudClient::with_device_token(server.base_url().parse().unwrap(), "token")
    }

    fn clip_summary(
        id: &str,
        client_clip_id: Option<&str>,
        title: &str,
        visibility: &str,
        status: &str,
        public_url: Option<&str>,
    ) -> ClipSummaryResponse {
        let now = Utc::now();
        ClipSummaryResponse {
            id: id.into(),
            client_clip_id: client_clip_id.map(str::to_string),
            title: title.into(),
            description: None,
            game_name: Some("League of Legends".into()),
            game_id: Some("league_of_legends".into()),
            source_type: Some("replay".into()),
            recorded_at: Some(now),
            uploaded_at: Some(now),
            duration_ms: Some(30_000),
            file_size_bytes: Some(12_345),
            width: Some(1920),
            height: Some(1080),
            fps: Some(60.0),
            visibility: visibility.into(),
            status: status.into(),
            public_url: public_url.map(str::to_string),
            view_count: 0,
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
