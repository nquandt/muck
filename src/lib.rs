//! Originally forked/ported from momokun7/xgrep (https://github.com/momokun7/xgrep).

pub mod globfilter;
pub mod handlers;
pub mod models;
pub mod search;
pub mod store;
pub mod trigram;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::Router;
use handlers::AppState;

/// Individual files are pushed as a single request body — generous enough for real-world
/// source files, small enough to bound worst-case memory use per upload.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// The route set shipped in the deployed `xgrep-server` binary. Shared with the
/// `xgrep-server-local` binary (see `src/bin/local.rs`), which layers additional read-only
/// endpoints and static UI serving on top of this same router.
pub fn base_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/search", post(handlers::search_repos))
        .route("/v1/index/status", get(handlers::index_status))
        .route(
            "/v1/repos/:repo_id/files",
            put(handlers::put_file)
                .delete(handlers::delete_file)
                .layer(DefaultBodyLimit::max(MAX_FILE_BYTES)),
        )
        .route("/v1/repos/:repo_id/build", post(handlers::build_repo))
        .route("/v1/repos/:repo_id", delete(handlers::unregister_repo))
        .with_state(state)
}
