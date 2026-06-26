use std::net::SocketAddr;

use super::security::FallbackToken;

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

    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
        );
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
