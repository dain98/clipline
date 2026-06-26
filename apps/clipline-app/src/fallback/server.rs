use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::Request;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use super::media::{MediaKind, MediaRegistry};
use super::security::{FallbackToken, token_guard};

const MAX_INVOKE_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct FallbackServerState {
    token: FallbackToken,
    ui_dir: PathBuf,
    base_url: String,
    media: Arc<MediaRegistry>,
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

fn ui_dir() -> Result<PathBuf, String> {
    let ui_dir = std::env::current_exe()
        .map_err(|e| format!("read current exe path: {e}"))?
        .parent()
        .ok_or_else(|| "current exe has no parent directory".to_string())?
        .join("ui");
    if ui_dir.join("index.html").is_file() {
        Ok(ui_dir)
    } else {
        Err(format!("fallback UI not found at {}", ui_dir.display()))
    }
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
    let path = state.ui_dir.join("index.html");
    let Ok(mut html) = std::fs::read_to_string(&path) else {
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
    let path = state.ui_dir.join(&asset);
    let Ok(bytes) = std::fs::read(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };
    let mut response = bytes.into_response();
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
    let Ok(id) = state.media.register(path, MediaKind::Clip) else {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    fn len(self) -> u64 {
        self.end - self.start + 1
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
        let Some(range) = parse_byte_range(range_header, file_len) else {
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

fn parse_byte_range(header_value: &str, file_len: u64) -> Option<ByteRange> {
    if file_len == 0 {
        return None;
    }
    let range = header_value.strip_prefix("bytes=")?;
    if range.contains(',') {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    if start.is_empty() {
        let suffix_len = end.parse::<u64>().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let len = suffix_len.min(file_len);
        return Some(ByteRange {
            start: file_len - len,
            end: file_len - 1,
        });
    }

    let start = start.parse::<u64>().ok()?;
    if start >= file_len {
        return None;
    }
    if end.is_empty() {
        return Some(ByteRange {
            start,
            end: file_len - 1,
        });
    }
    let end = end.parse::<u64>().ok()?.min(file_len - 1);
    (end >= start).then_some(ByteRange { start, end })
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
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "ok": false,
                "error": "invalid fallback command payload"
            })),
        )
            .into_response();
    }
    match command.as_str() {
        "frontend_ready" | "save_replay" => {
            return axum::Json(crate::host::runtime::FallbackCommandResult::ok(
                serde_json::Value::Null,
            ))
            .into_response();
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
    let body = "event: status\ndata: {\"recording\":false}\n\n";
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
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
pub async fn start_fallback_server(port: Option<u16>) -> Result<FallbackServerInfo, String> {
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
        let ui_dir = ui_dir().unwrap_or_else(|_| dev_ui_dir());
        let state = Arc::new(FallbackServerState {
            token: server_token,
            ui_dir,
            base_url: server_base_url,
            media: Arc::new(MediaRegistry::default()),
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
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;

    fn test_state(token: FallbackToken) -> Arc<FallbackServerState> {
        Arc::new(FallbackServerState {
            token,
            ui_dir: dev_ui_dir(),
            base_url: "http://127.0.0.1/fallback-token".to_string(),
            media: Arc::new(super::super::media::MediaRegistry::default()),
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

    async fn response_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let json = serde_json::from_slice(&body).expect("response body is JSON");
        (status, json)
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
    async fn invoke_placeholder_does_not_reflect_args() {
        let token = FallbackToken::generate_for_tests(7);
        let token_string = token.as_str().to_string();
        let response = invoke(
            State(test_state(token)),
            Path((token_string, "get_settings".to_string())),
            invoke_request(r#"{"secret":"cloud-shaped-value"}"#),
        )
        .await;

        let (status, body) = response_json(response).await;

        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body,
            serde_json::json!({
                "ok": false,
                "error": "fallback command not wired yet: get_settings"
            })
        );
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
        let response = media_path(
            State(test_state(token)),
            Path(token_string),
            RawQuery(Some(
                "path=C%3A%5CVideos%5CClipline%5Csecret.mp4".to_string(),
            )),
        )
        .await;

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

    #[test]
    fn parse_byte_range_accepts_closed_range() {
        assert_eq!(
            parse_byte_range("bytes=0-99", 1000),
            Some(ByteRange { start: 0, end: 99 })
        );
    }

    #[test]
    fn parse_byte_range_accepts_open_ended_range() {
        assert_eq!(
            parse_byte_range("bytes=100-", 1000),
            Some(ByteRange {
                start: 100,
                end: 999
            })
        );
    }

    #[test]
    fn parse_byte_range_accepts_suffix_range() {
        assert_eq!(
            parse_byte_range("bytes=-500", 1000),
            Some(ByteRange {
                start: 500,
                end: 999
            })
        );
    }

    #[test]
    fn parse_byte_range_rejects_invalid_or_multi_range_inputs() {
        assert_eq!(parse_byte_range("bytes=99-0", 1000), None);
        assert_eq!(parse_byte_range("bytes=0-99,200-299", 1000), None);
        assert_eq!(parse_byte_range("items=0-99", 1000), None);
        assert_eq!(parse_byte_range("bytes=-0", 1000), None);
        assert_eq!(parse_byte_range("bytes=1000-", 1000), None);
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
