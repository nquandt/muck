use std::env;
use std::sync::Arc;
use xgrep_server::handlers::AppState;
use xgrep_server::store::Store;
use xgrep_server::base_router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(7777);

    // Purely in-memory: no filesystem, no external search library/CLI. Callers push files
    // one at a time via PUT /v1/repos/{repoId}/files, then trigger a build via
    // POST /v1/repos/{repoId}/build. xgrep-server has no idea where content came from.
    let state = AppState { store: Arc::new(Store::new()) };

    let app = base_router(state);

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("xgrep-server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
