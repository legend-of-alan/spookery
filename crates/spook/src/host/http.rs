use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use super::gm_seat;
use super::limits::GM_LINE_CAP;
use super::world_loop::Msg;

// The HTTP body cap matches a GM TCP line. The seat tape has the same cap.
pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Msg>,
    next_conn: Arc<AtomicU64>,
) {
    let service = StreamableHttpService::new(
        move || {
            let conn = next_conn.fetch_add(1, Ordering::Relaxed);
            Ok(gm_seat::http_host(tx.clone(), conn))
        },
        Arc::new(LocalSessionManager::default()),
        http_config(),
    );
    let app = axum::Router::new().route_service("/mcp", service);
    let _ = axum::serve(listener, app).await;
}

fn http_config() -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1"])
        .with_allowed_origins(["http://127.0.0.1:*"]);
    config.max_request_body_bytes = GM_LINE_CAP;
    config
}
