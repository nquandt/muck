use std::env;
use muck::base_router;
use muck::handlers::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(7777);

    // In-memory by default: no filesystem, no external search library/CLI. Callers push
    // files one at a time via PUT /v1/repos/{repoId}/files, then trigger a build via
    // POST /v1/repos/{repoId}/build. muck has no idea where content came from.
    // Set MUCK_PERSIST_PATH to back this instance up to (and restore from) local disk —
    // see muck::store_from_env.
    let state = AppState { store: muck::store_from_env() };

    let app = base_router(state);

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("muck listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
