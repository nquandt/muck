use crate::globfilter::GlobFilter;
use crate::models::{
    BuildRepoQuery, FileContentResponse, FilePathQuery, HealthResponse, IndexStatusResponse,
    IndexedRepo, SearchFacet, SearchRequest, SearchResponse, TreeResponse,
};
use crate::search;
use crate::store::Store;
use axum::body::Bytes;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
}

/// Returns `503` while a persisted snapshot is still loading at startup (see
/// `Store::new_with_persistence`) — nothing is served as "ok" until that load attempt
/// finishes, so a load balancer/orchestrator won't route traffic to a half-populated
/// instance. Always `200` when disk persistence is disabled.
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    if state.store.ready.load(std::sync::atomic::Ordering::SeqCst) {
        (StatusCode::OK, Json(HealthResponse { status: "ok", version: env!("CARGO_PKG_VERSION") }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse { status: "loading", version: env!("CARGO_PKG_VERSION") }),
        )
    }
}

pub async fn index_status(State(state): State<AppState>) -> Json<IndexStatusResponse> {
    let repos = state.store.repos.read().await;
    let repositories: Vec<IndexedRepo> = repos
        .iter()
        .map(|(id, repo)| IndexedRepo {
            repo_id: id.clone(),
            repo_name: repo.name.clone(),
            version: repo.version.clone(),
            org: repo.org.clone(),
            branch: repo.branch.clone(),
            status: if repo.index.is_some() { "ready".to_string() } else { "pending".to_string() },
        })
        .collect();
    let total_repos = repositories.len();
    Json(IndexStatusResponse { repositories, total_repos })
}

/// `PUT /v1/repos/{repoId}/files?path=...` — writes (or overwrites) a single file in a
/// repo's in-memory working set. Body is the file's raw bytes. Pushed one file at a time by
/// the caller as it streams content off wherever it's sourced from (GitHub, Azure DevOps,
/// a local checkout) — no full clone, no shared volume, no filesystem at all on this side.
/// Does not rebuild the index; call `POST /v1/repos/{repoId}/build` once done pushing for a
/// sync cycle.
pub async fn put_file(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
    Query(query): Query<FilePathQuery>,
    content: Bytes,
) -> StatusCode {
    state.store.put_file(&repo_id, &query.path, content).await;
    StatusCode::NO_CONTENT
}

/// `DELETE /v1/repos/{repoId}/files?path=...` — removes a single file from a repo's working
/// set (e.g. deleted upstream). Does not rebuild the index.
pub async fn delete_file(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
    Query(query): Query<FilePathQuery>,
) -> StatusCode {
    state.store.delete_file(&repo_id, &query.path).await;
    StatusCode::NO_CONTENT
}

/// `POST /v1/repos/{repoId}/build?name=...&version=...` — (re)builds the trigram index from
/// whatever files have been pushed so far via `PUT .../files`.
pub async fn build_repo(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
    Query(query): Query<BuildRepoQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.store.build(repo_id.clone(), query.name, query.version, query.org, query.branch).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "repoId": repo_id }))),
        Err(error) => {
            tracing::error!(repo = %repo_id, "failed to build index: {error:#}");
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "status": "error", "message": error.to_string() })),
            )
        }
    }
}

/// `DELETE /v1/repos/{repoId}` — removes a repo's entire working set and drops it from the
/// index (e.g. the repo was deleted upstream).
pub async fn unregister_repo(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
) -> StatusCode {
    if state.store.unregister(&repo_id).await {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// `GET /v1/repos/{repoId}/file?path=...` — read back a file's content. Only mounted by the
/// `xgrep-server-local` binary (see `src/bin/local.rs`); the deployed `xgrep-server` binary
/// never routes this.
pub async fn get_file(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<FileContentResponse>, StatusCode> {
    let bytes = state.store.get_file(&repo_id, &query.path).await.ok_or(StatusCode::NOT_FOUND)?;
    match String::from_utf8(bytes.to_vec()) {
        Ok(content) => Ok(Json(FileContentResponse { path: query.path, content: Some(content), is_binary: false })),
        Err(_) => Ok(Json(FileContentResponse { path: query.path, content: None, is_binary: true })),
    }
}

/// `GET /v1/repos/{repoId}/tree` — flat list of every pushed path. Only mounted by the
/// `xgrep-server-local` binary.
pub async fn get_tree(
    State(state): State<AppState>,
    AxumPath(repo_id): AxumPath<String>,
) -> Result<Json<TreeResponse>, StatusCode> {
    let paths = state.store.list_paths(&repo_id).await.ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(TreeResponse { paths }))
}

pub async fn search_repos(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<serde_json::Value>)> {
    if req.query.trim().is_empty() {
        return Ok(Json(SearchResponse { results: vec![], next_cursor: None, facets: vec![] }));
    }

    // Smart-case, same convention as ripgrep/grep -i defaults: an all-lowercase pattern
    // searches case-insensitively.
    let case_insensitive = !req.query.chars().any(|c| c.is_ascii_uppercase());
    let query_lower_bytes = req.query.to_lowercase().into_bytes();

    let filter_repo_ids = req.filters.as_ref().and_then(|f| f.repo_ids.clone());
    let file_types = req.filters.as_ref().and_then(|f| f.file_types.clone());
    let path_prefix = req.filters.as_ref().and_then(|f| f.path_prefix.clone());
    let filter_orgs = req.filters.as_ref().and_then(|f| f.orgs.clone());
    let filter_branches = req.filters.as_ref().and_then(|f| f.branches.clone());
    let globs = req.filters.as_ref().and_then(|f| f.globs.clone());

    let glob_filter = match globs.as_deref().map(GlobFilter::new) {
        Some(Ok(filter)) => Some(filter),
        Some(Err(message)) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "status": "error", "message": message })),
            ))
        }
        None => None,
    };

    let snapshots = search::snapshot_candidates(
        &state.store.repos,
        req.allowed_repo_ids.as_deref(),
        filter_repo_ids.as_deref(),
        filter_orgs.as_deref(),
        filter_branches.as_deref(),
        &query_lower_bytes,
        req.regex,
    )
    .await;

    let query = req.query.clone();
    let regex = req.regex;

    let (mut results, facet_counts) = tokio::task::spawn_blocking(move || {
        let mut all_results = Vec::new();
        let mut all_facets = HashMap::new();
        for snapshot in &snapshots {
            let (repo_results, repo_facets) = search::scan_repo(
                snapshot,
                &query,
                regex,
                case_insensitive,
                file_types.as_deref(),
                path_prefix.as_deref(),
                glob_filter.as_ref(),
            );
            all_results.extend(repo_results);
            for (key, count) in repo_facets {
                *all_facets.entry(key).or_insert(0) += count;
            }
        }
        (all_results, all_facets)
    })
    .await
    .unwrap_or_default();

    // Tie-break on repo/path/line after score so repeated identical queries always come back
    // in the same order — score alone leaves ties, and their relative order otherwise depends
    // on repo/file iteration order, which isn't guaranteed stable across requests.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.repo_id.cmp(&b.repo_id))
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });

    let start = req.cursor.as_deref().and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
    let end = (start + req.page_size).min(results.len());
    let page = if start < results.len() { results[start..end].to_vec() } else { vec![] };
    let next_cursor = if end < results.len() { Some(end.to_string()) } else { None };

    let mut facets: Vec<SearchFacet> = facet_counts
        .into_iter()
        .map(|((name, facet_type), count)| SearchFacet { name, facet_type, count })
        .collect();
    facets.sort_by(|a, b| a.facet_type.cmp(&b.facet_type).then(b.count.cmp(&a.count)));

    Ok(Json(SearchResponse { results: page, next_cursor, facets }))
}
