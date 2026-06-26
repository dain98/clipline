use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::Request;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};

use super::security::{FallbackToken, token_guard};

const MAX_INVOKE_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct FallbackServerState {
    token: FallbackToken,
    ui_dir: PathBuf,
    base_url: String,
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
    (
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({
            "ok": false,
            "error": format!("fallback command not wired yet: {command}")
        })),
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
        });
        let app = axum::Router::new()
            .route("/{token}", get(index))
            .route("/{token}/", get(index))
            .route("/{token}/ui/{*asset}", get(ui_asset))
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
        })
    }

    fn invoke_request(body: &str) -> Request {
        Request::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(body.to_string()))
            .expect("build invoke request")
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
}
