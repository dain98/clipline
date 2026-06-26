use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;

use super::security::FallbackToken;

#[derive(Clone)]
struct FallbackServerState {
    token: String,
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
    if token != state.token {
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
    if token != state.token {
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
    let server_token = token_string.clone();
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
