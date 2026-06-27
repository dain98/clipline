use std::borrow::Cow;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::Request;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

use super::media::{parse_range_header, ByteRange, MediaKind, MediaRegistry};
use super::security::{token_guard, FallbackToken};

const MAX_INVOKE_BODY_BYTES: usize = 1024 * 1024;
#[cfg(test)]
const SSE_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);
#[cfg(not(test))]
const SSE_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
const SSE_HEARTBEAT_COMMENT: &str = ": heartbeat\n\n";

#[derive(Clone)]
struct FallbackServerState {
    token: FallbackToken,
    ui_assets: FallbackUiAssets,
    base_url: String,
    media: Arc<MediaRegistry>,
    host: Arc<crate::host::runtime::FallbackHostContext>,
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
#[derive(Clone)]
pub struct FallbackServerInfo {
    pub addr: SocketAddr,
    pub token: String,
    pub base_url: String,
}

#[derive(serde::Deserialize)]
struct SaveSettingsArgs {
    settings: crate::settings::AppSettings,
}

#[derive(serde::Deserialize)]
struct ChooseFolderArgs {
    current: Option<String>,
}

#[derive(serde::Deserialize)]
struct RevealClipArgs {
    path: String,
}

#[derive(serde::Deserialize)]
struct CopyClipToClipboardArgs {
    request: crate::library::CopyClipToClipboardRequest,
}

#[derive(serde::Deserialize)]
struct ClipPathArgs {
    path: String,
}

#[derive(serde::Deserialize)]
struct RenameClipArgs {
    path: String,
    name: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportClipArgs {
    path: String,
    start_s: f64,
    end_s: f64,
}

#[derive(serde::Deserialize)]
struct AudioPreviewArgs {
    request: crate::library::AudioPreviewRequest,
}

#[derive(serde::Deserialize)]
struct OpenCloudClipUrlArgs {
    url: String,
}

#[derive(serde::Deserialize)]
struct CloudConnectArgs {
    request: crate::cloud::CloudConnectRequest,
}

#[derive(serde::Deserialize)]
struct CloudClipAssetArgs {
    request: crate::cloud::CloudClipAssetRequest,
}

#[derive(serde::Deserialize)]
struct SyncCloudClipStatusArgs {
    request: crate::cloud::SyncCloudClipStatusRequest,
}

#[derive(serde::Deserialize)]
struct UploadClipToCloudArgs {
    request: crate::cloud::UploadClipCommandRequest,
}

#[derive(serde::Deserialize)]
struct ExtractWindowIconArgs {
    #[serde(rename = "exePath")]
    exe_path: String,
}

#[derive(serde::Deserialize)]
struct ReportDecodeSupportArgs {
    codecs: Vec<String>,
}

#[derive(serde::Deserialize)]
struct StartMicrophoneTestArgs {
    #[serde(rename = "deviceId")]
    device_id: Option<String>,
    volume: f64,
    mono: bool,
}

#[derive(serde::Deserialize)]
struct StopMicrophoneTestArgs {}

#[derive(Clone)]
enum FallbackUiAssets {
    Directory(PathBuf),
    Embedded,
}

impl FallbackUiAssets {
    fn index_html(&self) -> Result<String, String> {
        let bytes = self.asset_bytes("index.html")?;
        match bytes {
            Cow::Borrowed(bytes) => std::str::from_utf8(bytes)
                .map(str::to_string)
                .map_err(|e| format!("fallback UI index.html is not UTF-8: {e}")),
            Cow::Owned(bytes) => String::from_utf8(bytes)
                .map_err(|e| format!("fallback UI index.html is not UTF-8: {e}")),
        }
    }

    fn asset_bytes(&self, asset: &str) -> Result<Cow<'static, [u8]>, String> {
        match self {
            Self::Directory(ui_dir) => std::fs::read(ui_dir.join(asset))
                .map(Cow::Owned)
                .map_err(|e| format!("read fallback UI asset {asset:?}: {e}")),
            Self::Embedded => embedded_ui_asset(asset)
                .map(Cow::Borrowed)
                .ok_or_else(|| format!("fallback UI asset {asset:?} is not embedded")),
        }
    }
}

const EMBEDDED_UI_ASSETS: &[(&str, &[u8])] = &[
    (
        "index.html",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/index.html")),
    ),
    (
        "client-bridge.js",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/client-bridge.js")),
    ),
    (
        "main.js",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/main.js")),
    ),
    (
        "player-core.js",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/player-core.js")),
    ),
    (
        "styles.css",
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/styles.css")),
    ),
    (
        "assets/clipline-icon.svg",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/clipline-icon.svg"
        )),
    ),
    (
        "assets/games/league-of-legends.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/games/league-of-legends.png"
        )),
    ),
    (
        "assets/markers/README.md",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/README.md"
        )),
    ),
    (
        "assets/markers/baron.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/baron.png"
        )),
    ),
    (
        "assets/markers/death.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/death.png"
        )),
    ),
    (
        "assets/markers/dragon.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/dragon.png"
        )),
    ),
    (
        "assets/markers/kill.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/kill.png"
        )),
    ),
    (
        "assets/markers/turret.png",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/ui/assets/markers/turret.png"
        )),
    ),
];

fn embedded_ui_asset(asset: &str) -> Option<&'static [u8]> {
    EMBEDDED_UI_ASSETS
        .iter()
        .find_map(|(path, bytes)| (*path == asset).then_some(*bytes))
}

fn fallback_ui_assets() -> FallbackUiAssets {
    fallback_ui_assets_from_candidates(fallback_ui_asset_candidates())
}

fn fallback_ui_asset_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("ui"));
            candidates.push(parent.join("resources").join("ui"));
        }
    }
    candidates.push(dev_ui_dir());
    candidates
}

fn fallback_ui_assets_from_candidates<I>(candidates: I) -> FallbackUiAssets
where
    I: IntoIterator<Item = PathBuf>,
{
    candidates
        .into_iter()
        .find(|ui_dir| ui_dir.join("index.html").is_file())
        .map(FallbackUiAssets::Directory)
        .unwrap_or(FallbackUiAssets::Embedded)
}

fn dev_ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

async fn index(
    State(state): State<Arc<FallbackServerState>>,
    Path(token): Path<String>,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(mut html) = state.ui_assets.index_html() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "read index.html").into_response();
    };
    if !html.contains("client-bridge.js") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "index.html missing client-bridge.js",
        )
            .into_response();
    }
    let base_url =
        serde_json::to_string(&state.base_url).expect("fallback base URL serializes as JSON");
    let config = format!(
        r#"<base href="{}/ui/">
  <script>window.__CLIPLINE_FALLBACK__ = {{ baseUrl: {base_url} }};</script>"#,
        state.base_url
    );
    html = html.replace("<head>", &format!("<head>\n  {config}"));
    Html(html).into_response()
}

async fn ui_asset(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, asset)): Path<(String, String)>,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !is_safe_ui_asset_path(&asset) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(bytes) = state.ui_assets.asset_bytes(&asset) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match FsPath::new(&asset).extension().and_then(|ext| ext.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };
    let mut response = bytes.into_owned().into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn media_path(
    State(state): State<Arc<FallbackServerState>>,
    Path(token): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(path) = media_path_query(query.as_deref()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let settings = match storage_settings_from_app(&state.host.settings()) {
        Ok(settings) => settings,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let validated = match crate::host::library::validate_media_path(&settings, &path) {
        Ok(validated) => validated,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Ok(id) = state
        .media
        .register(validated.path, fallback_media_kind(validated.kind))
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let location = format!("{}/media/{id}", state.base_url);
    let Ok(location) = HeaderValue::from_str(&location) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = StatusCode::TEMPORARY_REDIRECT.into_response();
    response.headers_mut().insert(header::LOCATION, location);
    response
}

fn fallback_media_kind(kind: crate::host::library::HostMediaKind) -> MediaKind {
    match kind {
        crate::host::library::HostMediaKind::Clip => MediaKind::Clip,
        crate::host::library::HostMediaKind::Poster => MediaKind::Poster,
        crate::host::library::HostMediaKind::AudioPreview => MediaKind::AudioPreview,
        crate::host::library::HostMediaKind::CloudCache => MediaKind::CloudCache,
    }
}

async fn media(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, id)): Path<(String, String)>,
    request: Request,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let entry = match state.media.lookup(&id) {
        Ok(Some(entry)) => entry,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let range_header = request
        .headers()
        .get(header::RANGE)
        .map(|value| value.to_str().unwrap_or_default().to_string());
    let content_type = media_content_type(&entry.path);
    let path = entry.path;
    match read_media_file(path.clone(), range_header).await {
        Ok(MediaRead::Full { file, file_len }) => {
            media_file_response(file, StatusCode::OK, content_type, file_len, None)
        }
        Ok(MediaRead::Partial {
            file,
            range,
            file_len,
        }) => media_file_response(
            file,
            StatusCode::PARTIAL_CONTENT,
            content_type,
            range.len(),
            Some(format!("bytes {}-{}/{}", range.start, range.end, file_len)),
        ),
        Err(MediaReadError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(MediaReadError::RangeNotSatisfiable { file_len }) => {
            range_not_satisfiable_response(file_len)
        }
        Err(MediaReadError::Read) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

enum MediaRead {
    Full {
        file: File,
        file_len: u64,
    },
    Partial {
        file: File,
        range: ByteRange,
        file_len: u64,
    },
}

enum MediaReadError {
    NotFound,
    RangeNotSatisfiable { file_len: u64 },
    Read,
}

async fn read_media_file(
    path: PathBuf,
    range_header: Option<String>,
) -> Result<MediaRead, MediaReadError> {
    tokio::task::spawn_blocking(move || read_media_file_blocking(path, range_header.as_deref()))
        .await
        .map_err(|_| MediaReadError::Read)?
}

fn read_media_file_blocking(
    path: PathBuf,
    range_header: Option<&str>,
) -> Result<MediaRead, MediaReadError> {
    let mut file = File::open(&path).map_err(|_| MediaReadError::NotFound)?;
    let file_len = file.metadata().map_err(|_| MediaReadError::NotFound)?.len();
    if let Some(range_header) = range_header {
        let Some(range) = parse_range_header(range_header, file_len) else {
            return Err(MediaReadError::RangeNotSatisfiable { file_len });
        };
        file.seek(SeekFrom::Start(range.start))
            .map_err(|_| MediaReadError::Read)?;
        return Ok(MediaRead::Partial {
            file,
            range,
            file_len,
        });
    }

    Ok(MediaRead::Full { file, file_len })
}

fn media_file_response(
    file: File,
    status: StatusCode,
    content_type: &'static str,
    content_length: u64,
    content_range: Option<String>,
) -> Response {
    let file = tokio::fs::File::from_std(file);
    let body = axum::body::Body::from_stream(ReaderStream::new(file.take(content_length)));
    let mut response = body.into_response();
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string()).expect("content length is valid"),
    );
    if let Some(content_range) = content_range {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&content_range).expect("content range is valid"),
        );
    }
    response
}

fn range_not_satisfiable_response(file_len: u64) -> Response {
    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    let headers = response.headers_mut();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes */{file_len}")).expect("content range is valid"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    response
}

fn command_response<T: serde::Serialize>(result: Result<T, String>) -> Response {
    match result {
        Ok(value) => axum::Json(crate::host::runtime::FallbackCommandResult::ok(
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
        ))
        .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(crate::host::runtime::FallbackCommandResult::err(error)),
        )
            .into_response(),
    }
}

fn ok_response<T: serde::Serialize>(value: T) -> Response {
    axum::Json(crate::host::runtime::FallbackCommandResult::ok(
        serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
    ))
    .into_response()
}

fn parse_command_args<T: serde::de::DeserializeOwned>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("{command} arguments are invalid: {e}"))
}

fn storage_settings_from_app(
    settings: &crate::settings::AppSettings,
) -> Result<crate::library::StorageSettings, String> {
    Ok(crate::library::StorageSettings::new(
        crate::settings::quota_bytes_from_gb(settings.disk_quota_gb)?,
        settings.media_dir_path()?,
    ))
}

pub fn fallback_dispatches_command(command: &str) -> bool {
    matches!(
        command,
        "frontend_ready"
            | "minimize_main_window"
            | "save_replay"
            | "set_recording"
            | "get_settings"
            | "save_settings"
            | "choose_media_folder"
            | "choose_replay_cache_folder"
            | "get_autostart_status"
            | "check_for_updates"
            | "install_update"
            | "list_displays"
            | "list_audio_devices"
            | "probe_encoders"
            | "list_game_plugins"
            | "list_game_windows"
            | "extract_window_icon"
            | "memory_status"
            | "list_clips"
            | "clip_poster"
            | "preview_clip_audio_tracks"
            | "delete_clip"
            | "rename_clip"
            | "export_clip"
            | "storage_status"
            | "cloud_status"
            | "cloud_connect"
            | "cloud_disconnect"
            | "list_cloud_clips"
            | "cloud_clip_thumbnail"
            | "cache_cloud_clip_media"
            | "cloud_user_profile"
            | "cloud_user_avatar"
            | "reveal_clip"
            | "copy_clip_to_clipboard"
            | "open_cloud_user_profile"
            | "open_cloud_clip_url"
            | "sync_cloud_clip_status"
            | "upload_clip_to_cloud"
            | "report_decode_support"
            | "start_microphone_test"
            | "stop_microphone_test"
    )
}

async fn invoke(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, command)): Path<(String, String)>,
    request: Request,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": "invalid fallback token"
            })),
        )
            .into_response();
    }
    if !super::manifest::is_fallback_command(&command) {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": format!("unknown command: {command}")
            })),
        )
            .into_response();
    }
    let Ok(body) = to_bytes(request.into_body(), MAX_INVOKE_BODY_BYTES).await else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": "invalid fallback command payload"
            })),
        )
            .into_response();
    };
    let args = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(args) => args,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "ok": false,
                    "error": "invalid fallback command payload"
                })),
            )
                .into_response();
        }
    };
    if !fallback_dispatches_command(&command) {
        return (
            StatusCode::NOT_IMPLEMENTED,
            axum::Json(crate::host::runtime::FallbackCommandResult::err(format!(
                "fallback command not wired yet: {command}"
            ))),
        )
            .into_response();
    }
    match command.as_str() {
        "get_settings" => return ok_response(state.host.settings()),
        // The fallback UI runs in the user's browser, not a Clipline-owned native window.
        // Treat native minimize as a successful no-op so shared frontend controls can stay wired.
        "minimize_main_window" => return ok_response(serde_json::Value::Null),
        "frontend_ready" => return ok_response(serde_json::Value::Null),
        "save_replay" => {
            let _ = state.host.save_replay();
            return ok_response(serde_json::Value::Null);
        }
        "set_recording" => {
            let Some(recording) = args.get("recording").and_then(serde_json::Value::as_bool) else {
                return command_response::<bool>(Err(
                    "set_recording requires boolean recording".to_string()
                ));
            };
            return command_response(state.host.set_recording(recording));
        }
        "save_settings" => {
            let args = match parse_command_args::<SaveSettingsArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<crate::settings::AppSettings>(Err(e)),
            };
            return command_response(state.host.save_settings(args.settings));
        }
        "choose_media_folder" => {
            let args = match parse_command_args::<ChooseFolderArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<Option<String>>(Err(e)),
            };
            let settings = state.host.settings();
            return command_response(
                crate::app::host_choose_media_folder(&settings, args.current).await,
            );
        }
        "choose_replay_cache_folder" => {
            let args = match parse_command_args::<ChooseFolderArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<Option<String>>(Err(e)),
            };
            let settings = state.host.settings();
            return command_response(
                crate::app::host_choose_replay_cache_folder(&settings, args.current).await,
            );
        }
        "get_autostart_status" => {
            return command_response(state.host.get_autostart_status());
        }
        "check_for_updates" => {
            return command_response(state.host.check_for_updates().await);
        }
        "install_update" => {
            return command_response(state.host.install_update().await);
        }
        "list_displays" => return command_response(state.host.list_displays()),
        "list_audio_devices" => return command_response(state.host.list_audio_devices()),
        "probe_encoders" => return ok_response(state.host.probe_encoders()),
        "list_game_plugins" => return ok_response(crate::app::host_list_game_plugins()),
        "list_game_windows" => return ok_response(crate::app::host_list_game_windows()),
        "extract_window_icon" => {
            let args = match parse_command_args::<ExtractWindowIconArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<Option<String>>(Err(e)),
            };
            return ok_response(crate::app::host_extract_window_icon(args.exe_path));
        }
        "memory_status" => return command_response(crate::app::host_memory_status()),
        "list_clips" => {
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<Vec<crate::library::ClipInfo>>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                let dir = settings.clips_dir()?;
                crate::host::library::list_clips_from_dir(dir)
            })
            .await
            .map_err(|e| format!("list clips task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "clip_poster" => {
            let args = match parse_command_args::<ClipPathArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<String>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<String>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::clip_poster_for_host(args.path, &settings)
            })
            .await
            .map_err(|e| format!("clip poster task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "preview_clip_audio_tracks" => {
            let args = match parse_command_args::<AudioPreviewArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<String>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<String>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::preview_clip_audio_tracks_for_host(args.request, &settings)
            })
            .await
            .map_err(|e| format!("audio preview task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "delete_clip" => {
            let args = match parse_command_args::<ClipPathArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::delete_clip_for_host(args.path, &settings)
            })
            .await
            .map_err(|e| format!("delete clip task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "rename_clip" => {
            let args = match parse_command_args::<RenameClipArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<crate::library::RenamedClipInfo>(Err(e)),
            };
            let old_path = args.path.clone();
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<crate::library::RenamedClipInfo>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::rename_clip_for_host(args.path, args.name, &settings)
            })
            .await
            .map_err(|e| format!("rename clip task: {e}"))
            .and_then(|result| result);
            if let Ok(renamed) = &result {
                if let Err(error) = state
                    .host
                    .update_cloud_record_paths(&old_path, &renamed.path)
                {
                    eprintln!("update fallback cloud records after rename: {error}");
                }
            }
            return command_response(result);
        }
        "export_clip" => {
            let args = match parse_command_args::<ExportClipArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<crate::library::ExportedClipInfo>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<crate::library::ExportedClipInfo>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::export_clip_for_host(args.path, args.start_s, args.end_s, &settings)
            })
            .await
            .map_err(|e| format!("export clip task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "storage_status" => {
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<crate::library::StorageInfo>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                let dir = settings.clips_dir()?;
                crate::host::library::storage_status_for_dir(dir, settings.quota_bytes())
            })
            .await
            .map_err(|e| format!("storage status task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "cloud_status" => return ok_response(crate::cloud::host_cloud_status(&state.host)),
        "cloud_connect" => {
            let args = match parse_command_args::<CloudConnectArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => {
                    return command_response::<crate::cloud::CloudConnectionStatus>(Err(e));
                }
            };
            return command_response(
                crate::cloud::host_cloud_connect(&state.host, args.request).await,
            );
        }
        "cloud_disconnect" => {
            return command_response(crate::cloud::host_cloud_disconnect(&state.host));
        }
        "list_cloud_clips" => {
            return command_response(crate::cloud::host_list_cloud_clips(&state.host).await);
        }
        "cloud_clip_thumbnail" => {
            let args = match parse_command_args::<CloudClipAssetArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<Option<String>>(Err(e)),
            };
            return command_response(
                crate::cloud::host_cloud_clip_thumbnail(&state.host, args.request).await,
            );
        }
        "cache_cloud_clip_media" => {
            let args = match parse_command_args::<CloudClipAssetArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<crate::cloud::CachedCloudClip>(Err(e)),
            };
            return command_response(
                crate::cloud::host_cache_cloud_clip_media(&state.host, args.request).await,
            );
        }
        "cloud_user_profile" => {
            return command_response(crate::cloud::host_cloud_user_profile(&state.host).await);
        }
        "cloud_user_avatar" => {
            return command_response(crate::cloud::host_cloud_user_avatar(&state.host).await);
        }
        "reveal_clip" => {
            let args = match parse_command_args::<RevealClipArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let target = match crate::library::validate_clip_path(&settings, &args.path) {
                Ok(target) => target,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let Some(dir) = target.parent() else {
                return command_response::<()>(Err("clip has no containing folder".into()));
            };
            return command_response(crate::host::native::open_folder(dir));
        }
        "copy_clip_to_clipboard" => {
            let args = match parse_command_args::<CopyClipToClipboardArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let settings = match storage_settings_from_app(&state.host.settings()) {
                Ok(settings) => settings,
                Err(e) => return command_response::<()>(Err(e)),
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::library::copy_clip_to_clipboard_for_host(args.request, &settings)
            })
            .await
            .map_err(|e| format!("copy clip task: {e}"))
            .and_then(|result| result);
            return command_response(result);
        }
        "open_cloud_user_profile" => {
            return command_response(crate::cloud::host_open_cloud_user_profile(&state.host));
        }
        "open_cloud_clip_url" => {
            let args = match parse_command_args::<OpenCloudClipUrlArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<()>(Err(e)),
            };
            return command_response(crate::cloud::host_open_cloud_clip_url(args.url));
        }
        "sync_cloud_clip_status" => {
            let args = match parse_command_args::<SyncCloudClipStatusArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => {
                    return command_response::<crate::cloud::CloudClipStatusSyncResult>(Err(e));
                }
            };
            return command_response(
                crate::cloud::host_sync_cloud_clip_status(&state.host, args.request).await,
            );
        }
        "upload_clip_to_cloud" => {
            let args = match parse_command_args::<UploadClipToCloudArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<crate::cloud::CloudUploadResult>(Err(e)),
            };
            return command_response(
                crate::cloud::host_upload_clip_to_cloud(&state.host, args.request).await,
            );
        }
        "report_decode_support" => {
            let args = match parse_command_args::<ReportDecodeSupportArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<serde_json::Value>(Err(e)),
            };
            state.host.report_decode_support(&args.codecs);
            return ok_response(serde_json::Value::Null);
        }
        "start_microphone_test" => {
            let args = match parse_command_args::<StartMicrophoneTestArgs>(&command, args) {
                Ok(args) => args,
                Err(e) => return command_response::<serde_json::Value>(Err(e)),
            };
            return command_response(state.host.start_microphone_test(
                args.device_id,
                args.volume,
                args.mono,
            ));
        }
        "stop_microphone_test" => {
            if let Err(e) = parse_command_args::<StopMicrophoneTestArgs>(&command, args) {
                return command_response::<serde_json::Value>(Err(e));
            }
            state.host.stop_microphone_test();
            return ok_response(serde_json::Value::Null);
        }
        _ => {}
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(crate::host::runtime::FallbackCommandResult::err(format!(
            "fallback command not wired yet: {command}"
        ))),
    )
        .into_response()
}

async fn events(
    State(state): State<Arc<FallbackServerState>>,
    Path(token): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    if token_guard(&state.token, &token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(name) = event_name_query(query.as_deref()) {
        if !super::manifest::is_fallback_event(name) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    let filter = event_name_query(query.as_deref()).map(str::to_string);
    let hub = state.host.events();
    let (subscriber_id, event_rx) = hub.subscribe_with_id();
    let (reader, mut writer) = tokio::io::duplex(16 * 1024);
    let runtime = tokio::runtime::Handle::current();
    let thread_hub = hub.clone();
    let stream = std::thread::Builder::new()
        .name("clipline-fallback-sse".into())
        .spawn(move || {
            let mut next_heartbeat = std::time::Instant::now() + SSE_HEARTBEAT_INTERVAL;
            loop {
                let timeout = next_heartbeat.saturating_duration_since(std::time::Instant::now());
                match event_rx.recv_timeout(timeout) {
                    Ok(event) => {
                        if filter.as_deref().is_some_and(|name| event.name != name) {
                            // Keep checking the heartbeat deadline below so filtered streams
                            // still notice closed clients.
                        } else if let Some(chunk) = sse_event_chunk(&event) {
                            if write_sse_chunk(&runtime, &mut writer, &chunk).is_err() {
                                break;
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if std::time::Instant::now() >= next_heartbeat {
                    if write_sse_chunk(&runtime, &mut writer, SSE_HEARTBEAT_COMMENT).is_err() {
                        break;
                    }
                    next_heartbeat = std::time::Instant::now() + SSE_HEARTBEAT_INTERVAL;
                }
            }
            thread_hub.unsubscribe(subscriber_id);
        });
    if let Err(e) = stream {
        hub.unsubscribe(subscriber_id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn fallback event stream: {e}"),
        )
            .into_response();
    }

    let body = axum::body::Body::from_stream(ReaderStream::new(reader));
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn write_sse_chunk(
    runtime: &tokio::runtime::Handle,
    writer: &mut tokio::io::DuplexStream,
    chunk: &str,
) -> std::io::Result<()> {
    runtime.block_on(writer.write_all(chunk.as_bytes()))?;
    runtime.block_on(writer.flush())
}

fn sse_event_chunk(event: &crate::host::events::ClientEvent) -> Option<String> {
    let payload = serde_json::to_string(&event.payload).ok()?;
    Some(format!("event: {}\ndata: {payload}\n\n", event.name))
}

fn event_name_query(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == "name").then_some(value)
    })
}

fn media_path_query(query: Option<&str>) -> Result<PathBuf, ()> {
    let query = query.ok_or(())?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == "path" {
            let decoded = percent_decode_query_value(value)?;
            if decoded.is_empty() {
                return Err(());
            }
            return Ok(PathBuf::from(decoded));
        }
    }
    Err(())
}

fn percent_decode_query_value(value: &str) -> Result<String, ()> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn hex_value(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

fn media_content_type(path: &std::path::Path) -> &'static str {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return "application/octet-stream";
    };
    if extension.eq_ignore_ascii_case("mp4") {
        "video/mp4"
    } else if extension.eq_ignore_ascii_case("webm") {
        "video/webm"
    } else if extension.eq_ignore_ascii_case("png") {
        "image/png"
    } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        "image/jpeg"
    } else if extension.eq_ignore_ascii_case("wav") {
        "audio/wav"
    } else if extension.eq_ignore_ascii_case("mp3") {
        "audio/mpeg"
    } else {
        "application/octet-stream"
    }
}

fn is_safe_ui_asset_path(asset: &str) -> bool {
    if asset.is_empty() || asset.contains('\\') {
        return false;
    }
    std::path::Path::new(asset)
        .components()
        .all(|component| match component {
            Component::Normal(_) => true,
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => false,
        })
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
pub async fn start_fallback_server(
    port: Option<u16>,
    host: Arc<crate::host::runtime::FallbackHostContext>,
) -> Result<FallbackServerInfo, String> {
    let token = FallbackToken::generate()?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind fallback server: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("read fallback server address: {e}"))?;

    let token_string = token.as_str().to_string();
    let base_url = format!("http://{addr}/{token_string}");
    let server_token = token.clone();
    let server_base_url = base_url.clone();

    tokio::spawn(async move {
        let state = Arc::new(FallbackServerState {
            token: server_token,
            ui_assets: fallback_ui_assets(),
            base_url: server_base_url,
            media: Arc::new(MediaRegistry::default()),
            host,
        });
        let app = axum::Router::new()
            .route("/{token}", get(index))
            .route("/{token}/", get(index))
            .route("/{token}/ui/{*asset}", get(ui_asset))
            .route("/{token}/media-path", get(media_path))
            .route("/{token}/media/{id}", get(media))
            .route("/{token}/invoke/{command}", post(invoke))
            .route("/{token}/events", get(events))
            .route(
                "/health",
                get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
            )
            .with_state(state);
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("fallback server stopped: {e}");
        }
    });

    Ok(FallbackServerInfo {
        addr,
        token: token_string,
        base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body, HttpBody};
    use axum::extract::Request;
    use std::future::poll_fn;
    use std::time::{Duration, Instant};

    async fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        condition()
    }

    async fn read_sse_body_until(body: Body, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut body = std::pin::pin!(body);
        let mut text = String::new();
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let frame = tokio::time::timeout(
                remaining.min(Duration::from_millis(200)),
                poll_fn(|cx| body.as_mut().poll_frame(cx)),
            )
            .await;
            let frame = match frame {
                Ok(Some(frame)) => frame.expect("SSE body frame is ok"),
                Ok(None) => break,
                Err(_) => continue,
            };
            if let Ok(data) = frame.into_data() {
                text.push_str(std::str::from_utf8(&data).expect("SSE body is UTF-8"));
                if text.contains(needle) {
                    break;
                }
            }
        }
        text
    }

    fn test_state(token: FallbackToken) -> Arc<FallbackServerState> {
        test_state_with_settings(token, crate::settings::AppSettings::default())
    }

    fn test_state_with_settings(
        token: FallbackToken,
        settings: crate::settings::AppSettings,
    ) -> Arc<FallbackServerState> {
        Arc::new(FallbackServerState {
            token,
            ui_assets: FallbackUiAssets::Directory(dev_ui_dir()),
            base_url: "http://127.0.0.1/fallback-token".to_string(),
            media: Arc::new(super::super::media::MediaRegistry::default()),
            host: Arc::new(crate::host::runtime::FallbackHostContext::new(
                settings,
                Arc::new(crate::host::events::ClientEventHub::default()),
            )),
        })
    }

    fn invoke_request(body: &str) -> Request {
        Request::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(body.to_string()))
            .expect("build invoke request")
    }

    fn media_request(range: Option<&str>) -> Request {
        let mut builder = Request::builder();
        if let Some(range) = range {
            builder = builder.header(header::RANGE, range);
        }
        builder.body(Body::empty()).expect("build media request")
    }

    fn media_fixture(contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "clipline-media-test-{}-{}.mp4",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after epoch")
                .as_nanos()
        ));
        std::fs::write(&path, contents).expect("write media fixture");
        path
    }

    fn test_temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "clipline-fallback-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("create test temp dir");
        path
    }

    fn settings_with_media_dir(media_dir: &std::path::Path) -> crate::settings::AppSettings {
        crate::settings::AppSettings {
            media_dir: media_dir.display().to_string(),
            ..crate::settings::AppSettings::default()
        }
    }

    fn media_path_raw_query(path: &std::path::Path) -> String {
        format!("path={}", path.display())
    }

    async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let json = serde_json::from_slice(&body).expect("response body is JSON");
        (status, json)
    }

    async fn invoke_json(
        state: Arc<FallbackServerState>,
        command: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let token_string = state.token.as_str().to_string();
        let response = invoke(
            State(state),
            Path((token_string, command.to_string())),
            invoke_request(body),
        )
        .await;
        response_json(response).await
    }

    #[test]
    fn dispatch_table_contains_settings_and_probe_commands() {
        for command in [
            "get_settings",
            "save_settings",
            "list_displays",
            "list_audio_devices",
            "probe_encoders",
            "choose_media_folder",
            "choose_replay_cache_folder",
            "get_autostart_status",
            "check_for_updates",
            "install_update",
            "list_game_plugins",
            "list_game_windows",
            "extract_window_icon",
            "memory_status",
            "list_clips",
            "clip_poster",
            "preview_clip_audio_tracks",
            "delete_clip",
            "rename_clip",
            "export_clip",
            "storage_status",
            "reveal_clip",
            "copy_clip_to_clipboard",
            "open_cloud_user_profile",
            "open_cloud_clip_url",
            "report_decode_support",
            "start_microphone_test",
            "stop_microphone_test",
        ] {
            assert!(
                fallback_dispatches_command(command),
                "missing dispatch for {command}"
            );
        }
    }

    #[test]
    fn dispatch_table_contains_cloud_commands() {
        for command in [
            "cloud_status",
            "cloud_connect",
            "cloud_disconnect",
            "list_cloud_clips",
            "cloud_clip_thumbnail",
            "cache_cloud_clip_media",
            "cloud_user_profile",
            "cloud_user_avatar",
            "open_cloud_user_profile",
            "open_cloud_clip_url",
            "sync_cloud_clip_status",
            "upload_clip_to_cloud",
        ] {
            assert!(
                fallback_dispatches_command(command),
                "missing dispatch for {command}"
            );
        }
    }

    #[tokio::test]
    async fn invoke_sync_cloud_clip_status_accepts_request_envelope() {
        let clip_path = "D:\\Videos\\Clipline\\missing.mp4";
        let body = serde_json::json!({
            "request": {
                "path": clip_path,
            }
        })
        .to_string();

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(31)),
            "sync_cloud_clip_status",
            &body,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!({
                "ok": true,
                "value": {
                    "path": clip_path,
                    "record": null,
                    "removed": false,
                }
            })
        );
    }

    #[tokio::test]
    async fn invoke_cloud_request_envelopes_reach_host_validation() {
        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(32)),
            "cloud_connect",
            r#"{"request":{"host_url":"http://127.0.0.1:9","username":"u","password":"p","plain_http_confirmed":true}}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_error_is_not_parse_failure(&body, "host_url");

        for (token, command) in [(33, "cloud_clip_thumbnail"), (34, "cache_cloud_clip_media")] {
            let (status, body) = invoke_json(
                test_state(FallbackToken::generate_for_tests(token)),
                command,
                r#"{"request":{"remote_clip_id":"remote-1"}}"#,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["error"], "Clipline Cloud is not connected");
        }

        let media_dir = test_temp_dir("cloud-upload-envelope");
        let clip = media_dir.join("upload.mp4");
        std::fs::write(&clip, b"not a real mp4").expect("write upload fixture");
        let settings = settings_with_media_dir(&media_dir);
        let body = serde_json::json!({
            "request": {
                "path": clip.display().to_string(),
            }
        })
        .to_string();
        let (status, body) = invoke_json(
            test_state_with_settings(FallbackToken::generate_for_tests(35), settings),
            "upload_clip_to_cloud",
            &body,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "connect to Clipline Cloud first");
    }

    fn assert_error_is_not_parse_failure(body: &serde_json::Value, field: &str) {
        let error = body["error"].as_str().expect("error is a string");
        assert!(
            !error.contains("arguments are invalid") && !error.contains("missing field"),
            "expected host validation error, got parse error: {error}"
        );
        assert!(
            !error.contains(field),
            "expected error not to complain about field {field}: {error}"
        );
    }

    #[tokio::test]
    async fn invoke_routes_low_risk_task14_commands() {
        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(7)),
            "list_game_plugins",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert!(body["value"].is_array());

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(8)),
            "extract_window_icon",
            r#"{"exePath":"C:\\definitely-missing\\clipline-task14-nope.exe"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true, "value": null}));

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(9)),
            "memory_status",
            "{}",
        )
        .await;
        assert!(matches!(status, StatusCode::OK | StatusCode::BAD_REQUEST));
        assert_ne!(status, StatusCode::NOT_IMPLEMENTED);
        assert_ne!(
            body.get("error").and_then(serde_json::Value::as_str),
            Some("fallback command not wired yet: memory_status")
        );

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(10)),
            "report_decode_support",
            r#"{"codecs":["hevc","av1"]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true, "value": null}));

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(11)),
            "stop_microphone_test",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true, "value": null}));
    }

    #[tokio::test]
    async fn invoke_task15_commands_without_native_side_effects() {
        let settings = crate::settings::AppSettings {
            open_on_startup: true,
            ..crate::settings::AppSettings::default()
        };
        let (status, body) = invoke_json(
            test_state_with_settings(FallbackToken::generate_for_tests(13), settings),
            "get_autostart_status",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true, "value": true}));

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(14)),
            "open_cloud_clip_url",
            r#"{"url":"file:///C:/secret.txt"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], false);
        assert_eq!(
            body["error"],
            "cloud clip URL scheme is not supported: file"
        );
    }

    #[tokio::test]
    async fn invoke_task14_typed_commands_reject_invalid_args_before_side_effects() {
        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(7)),
            "save_settings",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], false);
        assert!(
            body["error"]
                .as_str()
                .expect("error message")
                .contains("save_settings arguments are invalid"),
            "unexpected body: {body}"
        );

        let (status, body) = invoke_json(
            test_state(FallbackToken::generate_for_tests(8)),
            "start_microphone_test",
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["ok"], false);
        assert!(
            body["error"]
                .as_str()
                .expect("error message")
                .contains("start_microphone_test arguments are invalid"),
            "unexpected body: {body}"
        );
    }

    #[test]
    fn fallback_ui_assets_fall_back_to_embedded_assets_when_directories_are_missing() {
        let missing = std::env::temp_dir()
            .join(format!("clipline-missing-ui-{}", std::process::id()))
            .join("ui");

        let assets = fallback_ui_assets_from_candidates([missing]);

        assert!(matches!(assets, FallbackUiAssets::Embedded));
        let html = assets.index_html().expect("embedded index is readable");
        assert!(html.contains("client-bridge.js"));
        let bridge = assets
            .asset_bytes("client-bridge.js")
            .expect("embedded bridge is readable");
        assert!(std::str::from_utf8(bridge.as_ref())
            .expect("bridge is UTF-8")
            .contains("window.cliplineHost"));
    }

    #[tokio::test]
    async fn invoke_rejects_invalid_token_before_parsing_body() {
        let token = FallbackToken::generate_for_tests(7);
        let response = invoke(
            State(test_state(token)),
            Path(("wrong".to_string(), "get_settings".to_string())),
            invoke_request("not json"),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body,
            serde_json::json!({"ok": false, "error": "invalid fallback token"})
        );
    }

    #[tokio::test]
    async fn invoke_returns_controlled_error_for_valid_token_malformed_json() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let response = invoke(
            State(test_state(token)),
            Path((token_string, "get_settings".to_string())),
            invoke_request("not json"),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            serde_json::json!({"ok": false, "error": "invalid fallback command payload"})
        );
    }

    #[tokio::test]
    async fn invoke_get_settings_returns_host_settings_without_reflecting_args() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let settings = crate::settings::AppSettings {
            hotkey: "Ctrl+F8".to_string(),
            ..crate::settings::AppSettings::default()
        };
        let response = invoke(
            State(test_state_with_settings(token, settings)),
            Path((token_string, "get_settings".to_string())),
            invoke_request(r#"{"secret":"cloud-shaped-value"}"#),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(body["value"]["hotkey"], "Ctrl+F8");
        assert!(body["value"].get("secret").is_none());
    }

    #[tokio::test]
    async fn invoke_set_recording_rejects_missing_boolean_recording_arg() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let response = invoke(
            State(test_state(token)),
            Path((token_string, "set_recording".to_string())),
            invoke_request("{}"),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            serde_json::json!({"ok": false, "error": "set_recording requires boolean recording"})
        );
    }

    #[tokio::test]
    async fn invoke_set_recording_false_returns_recording_state() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let response = invoke(
            State(test_state(token)),
            Path((token_string, "set_recording".to_string())),
            invoke_request(r#"{"recording":false}"#),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"ok": true, "value": false}));
    }

    #[tokio::test]
    async fn events_streams_filtered_hub_events() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let hub = state.host.events();
        let response = events(
            State(state.clone()),
            Path(token_string),
            RawQuery(Some("name=saved".to_string())),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );

        hub.emit(crate::host::events::ClientEvent::new(
            "status",
            serde_json::json!({"recording": true}),
        ));
        hub.emit(crate::host::events::ClientEvent::new(
            "saved",
            serde_json::json!({"path": "clip.mp4"}),
        ));
        let body = read_sse_body_until(response.into_body(), "event: saved\n").await;

        assert!(body.contains("event: saved\n"));
        assert!(body.contains(r#"data: {"path":"clip.mp4"}"#));
        assert!(!body.contains("event: status\n"));
        assert!(wait_until(|| hub.subscriber_count() == 0).await);
        drop(state);
    }

    #[tokio::test]
    async fn events_drops_idle_subscriber_after_response_is_dropped() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let hub = state.host.events();
        let response = events(
            State(state),
            Path(token_string),
            RawQuery(Some("name=status".to_string())),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(hub.subscriber_count(), 1);
        drop(response);

        assert!(wait_until(|| hub.subscriber_count() == 0).await);
    }

    #[tokio::test]
    async fn media_path_rejects_invalid_token() {
        let token = FallbackToken::generate_for_tests(7);
        let response = media_path(
            State(test_state(token)),
            Path("wrong".to_string()),
            RawQuery(Some(
                "path=C%3A%5CVideos%5CClipline%5Csecret.mp4".to_string(),
            )),
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn media_path_redirects_registered_path_to_opaque_media_url() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let media_dir = test_temp_dir("media-path-valid");
        let clip = media_dir.join("secret.mp4");
        std::fs::write(&clip, b"clip").expect("write valid clip");
        let response = media_path(
            State(test_state_with_settings(
                token,
                settings_with_media_dir(&media_dir),
            )),
            Path(token_string),
            RawQuery(Some(media_path_raw_query(&clip))),
        )
        .await;
        let _ = std::fs::remove_dir_all(media_dir);

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        let location = response
            .headers()
            .get(header::LOCATION)
            .expect("redirect includes location")
            .to_str()
            .expect("location is valid ASCII");
        assert!(location.starts_with("http://127.0.0.1/fallback-token/media/m"));
        assert!(!location.contains("Videos"));
        assert!(!location.contains("secret"));
    }

    #[tokio::test]
    async fn media_path_rejects_paths_outside_configured_library() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let media_dir = test_temp_dir("media-path-library");
        let outside_dir = test_temp_dir("media-path-outside");
        let outside = outside_dir.join("secret.mp4");
        std::fs::write(&outside, b"clip").expect("write outside clip");

        let response = media_path(
            State(test_state_with_settings(
                token,
                settings_with_media_dir(&media_dir),
            )),
            Path(token_string),
            RawQuery(Some(media_path_raw_query(&outside))),
        )
        .await;
        let _ = std::fs::remove_dir_all(media_dir);
        let _ = std::fs::remove_dir_all(outside_dir);

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers().get(header::LOCATION).is_none());
    }

    #[tokio::test]
    async fn invoke_task16_library_commands_without_native_side_effects() {
        let media_dir = test_temp_dir("library-commands");
        let outside_dir = test_temp_dir("library-commands-outside");
        let outside = outside_dir.join("outside.mp4");
        std::fs::write(&outside, b"clip").expect("write outside clip");
        let state = test_state_with_settings(
            FallbackToken::generate_for_tests(16),
            settings_with_media_dir(&media_dir),
        );

        let (status, body) = invoke_json(state.clone(), "list_clips", "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert!(body["value"].as_array().is_some_and(Vec::is_empty));

        let (status, body) = invoke_json(state.clone(), "storage_status", "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);

        for (command, body) in [
            (
                "clip_poster",
                serde_json::json!({ "path": outside.display().to_string() }).to_string(),
            ),
            (
                "preview_clip_audio_tracks",
                serde_json::json!({
                    "request": {
                        "path": outside.display().to_string(),
                        "audioTrackIds": []
                    }
                })
                .to_string(),
            ),
            (
                "delete_clip",
                serde_json::json!({ "path": outside.display().to_string() }).to_string(),
            ),
            (
                "rename_clip",
                serde_json::json!({
                    "path": outside.display().to_string(),
                    "name": "renamed.mp4"
                })
                .to_string(),
            ),
            (
                "export_clip",
                serde_json::json!({
                    "path": outside.display().to_string(),
                    "startS": 0.0,
                    "endS": 1.0
                })
                .to_string(),
            ),
        ] {
            let (status, body) = invoke_json(state.clone(), command, &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{command}: {body}");
            assert_eq!(body["ok"], false, "{command}: {body}");
            assert_ne!(
                body.get("error").and_then(serde_json::Value::as_str),
                Some(format!("fallback command not wired yet: {command}").as_str()),
                "{command}: {body}"
            );
        }

        let _ = std::fs::remove_dir_all(media_dir);
        let _ = std::fs::remove_dir_all(outside_dir);
    }

    #[tokio::test]
    async fn invoke_rename_clip_updates_cloud_record_path() {
        let media_dir = test_temp_dir("library-rename-cloud-record");
        let clip = media_dir.join("tracked.mp4");
        std::fs::write(&clip, b"clip").expect("write tracked clip");
        let mut settings = settings_with_media_dir(&media_dir);
        settings.cloud.uploads.insert(
            "tracked".into(),
            crate::settings::CloudUploadRecord {
                local_clip_id: "tracked".into(),
                path: clip.display().to_string(),
                remote_clip_id: Some("remote-1".into()),
                remote_url: None,
                visibility: "private".into(),
                upload_status: "uploaded".into(),
                error: None,
                updated_at_unix: 1,
            },
        );
        let state = test_state_with_settings(FallbackToken::generate_for_tests(17), settings);
        let body = serde_json::json!({
            "path": clip.display().to_string(),
            "name": "renamed.mp4"
        })
        .to_string();
        let renamed = media_dir.join("renamed.mp4");

        let (status, body) = invoke_json(state.clone(), "rename_clip", &body).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        assert_eq!(
            body["value"]["path"],
            serde_json::Value::String(renamed.display().to_string())
        );
        assert!(!clip.exists());
        assert!(renamed.exists());
        assert_eq!(
            state
                .host
                .settings()
                .cloud
                .uploads
                .get("tracked")
                .expect("cloud upload record remains present")
                .path,
            renamed.display().to_string()
        );
        let _ = std::fs::remove_dir_all(media_dir);
    }

    #[test]
    fn parse_range_header_accepts_closed_range() {
        assert_eq!(
            parse_range_header("bytes=0-99", 1000),
            Some(ByteRange { start: 0, end: 99 })
        );
    }

    #[test]
    fn parse_range_header_accepts_open_ended_range() {
        assert_eq!(
            parse_range_header("bytes=100-", 1000),
            Some(ByteRange {
                start: 100,
                end: 999
            })
        );
    }

    #[test]
    fn parse_range_header_accepts_suffix_range() {
        assert_eq!(
            parse_range_header("bytes=-500", 1000),
            Some(ByteRange {
                start: 500,
                end: 999
            })
        );
    }

    #[test]
    fn parse_range_header_rejects_invalid_or_multi_range_inputs() {
        assert_eq!(parse_range_header("bytes=99-0", 1000), None);
        assert_eq!(parse_range_header("bytes=0-99,200-299", 1000), None);
        assert_eq!(parse_range_header("items=0-99", 1000), None);
        assert_eq!(parse_range_header("bytes=-0", 1000), None);
        assert_eq!(parse_range_header("bytes=1000-", 1000), None);
    }

    #[tokio::test]
    async fn media_serves_registered_file_bytes() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let file_path = media_fixture(b"clipline media");
        let id = state
            .media
            .register(file_path.clone(), super::super::media::MediaKind::Clip)
            .expect("register media fixture");

        let response = media(State(state), Path((token_string, id)), media_request(None)).await;
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("media response includes content type")
            .to_str()
            .expect("content type is valid ASCII")
            .to_string();
        let accept_ranges = response
            .headers()
            .get(header::ACCEPT_RANGES)
            .expect("media response includes accept-ranges")
            .to_str()
            .expect("accept-ranges is valid ASCII")
            .to_string();
        let content_length = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .expect("media response includes content length")
            .to_str()
            .expect("content length is valid ASCII")
            .to_string();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read media body");
        let _ = std::fs::remove_file(file_path);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "video/mp4");
        assert_eq!(accept_ranges, "bytes");
        assert_eq!(content_length, "14");
        assert_eq!(&body[..], b"clipline media");
    }

    #[tokio::test]
    async fn media_serves_partial_range_response() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let contents: Vec<u8> = (0..=255).collect();
        let file_path = media_fixture(&contents);
        let id = state
            .media
            .register(file_path.clone(), super::super::media::MediaKind::Clip)
            .expect("register media fixture");

        let response = media(
            State(state),
            Path((token_string, id)),
            media_request(Some("bytes=10-19")),
        )
        .await;
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read media body");
        let _ = std::fs::remove_file(file_path);

        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            headers.get(header::CONTENT_RANGE).unwrap(),
            "bytes 10-19/256"
        );
        assert_eq!(headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert_eq!(headers.get(header::CONTENT_LENGTH).unwrap(), "10");
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "video/mp4");
        assert_eq!(&body[..], &contents[10..20]);
    }

    #[tokio::test]
    async fn media_serves_oversized_bounded_range_response() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let contents = b"small media";
        let file_path = media_fixture(contents);
        let id = state
            .media
            .register(file_path.clone(), super::super::media::MediaKind::Clip)
            .expect("register media fixture");

        let response = media(
            State(state),
            Path((token_string, id)),
            media_request(Some("bytes=0-65535")),
        )
        .await;
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read media body");
        let _ = std::fs::remove_file(file_path);

        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(headers.get(header::CONTENT_RANGE).unwrap(), "bytes 0-10/11");
        assert_eq!(headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
        assert_eq!(headers.get(header::CONTENT_LENGTH).unwrap(), "11");
        assert_eq!(&body[..], contents);
    }

    #[tokio::test]
    async fn media_returns_416_for_unsatisfiable_range() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let state = test_state(token);
        let file_path = media_fixture(b"clipline media");
        let id = state
            .media
            .register(file_path.clone(), super::super::media::MediaKind::Clip)
            .expect("register media fixture");

        let response = media(
            State(state),
            Path((token_string, id)),
            media_request(Some("bytes=100-199")),
        )
        .await;
        let status = response.status();
        let content_range = response
            .headers()
            .get(header::CONTENT_RANGE)
            .expect("416 includes content range")
            .to_str()
            .expect("content range is valid ASCII")
            .to_string();
        let _ = std::fs::remove_file(file_path);

        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(content_range, "bytes */14");
    }
}
