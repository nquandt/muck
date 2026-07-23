//! Originally forked/ported from momokun7/xgrep (https://github.com/momokun7/xgrep).

pub mod globfilter;
pub mod handlers;
pub mod models;
pub mod persist;
pub mod search;
pub mod shard;
pub mod store;
pub mod trigram;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post, put};
use axum::Router;
use handlers::AppState;
use std::sync::Arc;
use store::Store;

/// Individual files are pushed as a single request body — generous enough for real-world
/// source files, small enough to bound worst-case memory use per upload.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Builds a `Store` per the `MUCK_PERSIST_PATH` env var: unset means no disk backup/restore
/// (a restart loses everything, though pushed file content still lives in on-disk shard
/// files for as long as the container/host survives — see `store::shard_dir_from_env`), set
/// means also back up full repo state to and restore it from that file path on this
/// instance's local disk (see `store::Store::new_with_persistence` and `persist`). Shared by
/// both `muck` and `muck-local` so the two binaries behave identically here.
pub fn store_from_env() -> Arc<Store> {
    match std::env::var("MUCK_PERSIST_PATH") {
        Ok(path) if !path.trim().is_empty() => Store::new_with_persistence(path.into()),
        _ => Arc::new(Store::new()),
    }
}

/// The route set shipped in the deployed `muck` binary. Shared with the
/// `muck-local` binary (see `src/bin/local.rs`), which layers additional read-only
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
