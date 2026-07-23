//! `muck-local` — same search/index engine as `muck`, plus a couple of
//! read-only endpoints and an embedded React SPA (built from `ui/`) for a
//! GitHub-code-search-style local dev experience. No auth of any kind — this binary is a
//! local/dev artifact, not meant to be exposed publicly.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use std::env;
use muck::handlers::{self, AppState};

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct Assets;

async fn serve_ui(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return ([(header::CONTENT_TYPE, mime.as_ref().to_string())], file.data).into_response();
    }

    // SPA fallback: any unmatched path (client-side route) gets index.html.
    match Assets::get("index.html") {
        Some(file) => {
            ([(header::CONTENT_TYPE, "text/html".to_string())], file.data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = env::var("PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(7777);
    let state = AppState { store: muck::store_from_env() };

    let extra_routes = axum::Router::new()
        .route("/v1/repos/:repo_id/file", get(handlers::get_file))
        .route("/v1/repos/:repo_id/tree", get(handlers::get_tree))
        .with_state(state.clone());

    let app = muck::base_router(state).merge(extra_routes).fallback(serve_ui);

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("muck-local (embedded UI) listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
