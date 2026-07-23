use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Default)]
pub struct SearchFilters {
    #[serde(rename = "repoIds")]
    pub repo_ids: Option<Vec<String>>,
    #[serde(rename = "fileTypes")]
    pub file_types: Option<Vec<String>>,
    #[serde(rename = "pathPrefix")]
    pub path_prefix: Option<String>,
    pub orgs: Option<Vec<String>>,
    pub branches: Option<Vec<String>>,
    /// Ripgrep-style include/exclude globs (`-g "*.rs"`, `-g "!*_test.rs"`) — see
    /// [`crate::globfilter`].
    pub globs: Option<Vec<String>>,
}

/// The body of a POST to /v1/search.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(rename = "allowedRepoIds", default)]
    pub allowed_repo_ids: Option<Vec<String>>,
    #[serde(default)]
    pub filters: Option<SearchFilters>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(rename = "pageSize", default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    25
}

/// Includes `contextBefore`/`contextAfter` — a couple of surrounding lines so a UI can render
/// a GitHub-style result card without a second round-trip to fetch the whole file.
#[derive(Debug, Serialize, Clone)]
pub struct SearchResult {
    #[serde(rename = "repoId")]
    pub repo_id: String,
    #[serde(rename = "repoName")]
    pub repo_name: String,
    pub path: String,
    pub line: u64,
    pub column: u64,
    pub snippet: String,
    #[serde(rename = "contextBefore")]
    pub context_before: Vec<String>,
    #[serde(rename = "contextAfter")]
    pub context_after: Vec<String>,
    pub score: f64,
    #[serde(rename = "blobSha")]
    pub blob_sha: String,
    /// Caller-supplied grouping label (e.g. a GitHub owner or Azure DevOps project) — opaque
    /// to xgrep-server, surfaced for facet filtering/display only.
    pub org: String,
    /// Caller-supplied branch name indexed — opaque to xgrep-server, same as `org`.
    pub branch: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchFacet {
    pub name: String,
    #[serde(rename = "type")]
    pub facet_type: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    pub facets: Vec<SearchFacet>,
}

#[derive(Debug, Serialize, Clone)]
pub struct IndexedRepo {
    #[serde(rename = "repoId")]
    pub repo_id: String,
    #[serde(rename = "repoName")]
    pub repo_name: String,
    /// Caller-supplied version/revision tag (e.g. a git commit sha) — opaque to xgrep-server.
    pub version: String,
    pub org: String,
    pub branch: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct IndexStatusResponse {
    pub repositories: Vec<IndexedRepo>,
    #[serde(rename = "totalRepos")]
    pub total_repos: usize,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Query params for `PUT|DELETE /v1/repos/{repoId}/files?path=...` — the request body of a
/// PUT is the file's raw bytes. Pushed one file at a time by the caller, so a full
/// clone/checkout is never needed on the xgrep-server side.
#[derive(Debug, Deserialize)]
pub struct FilePathQuery {
    pub path: String,
}

/// Response for `GET /v1/repos/{repoId}/file?path=...` — used only by the `xgrep-server-local`
/// UI variant. `content` is omitted (left `None`) when the file looks binary.
#[derive(Debug, Serialize)]
pub struct FileContentResponse {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(rename = "isBinary")]
    pub is_binary: bool,
}

/// Response for `GET /v1/repos/{repoId}/tree` — used only by the `xgrep-server-local` UI
/// variant. Flat path list; the client builds tree structure from it.
#[derive(Debug, Serialize)]
pub struct TreeResponse {
    pub paths: Vec<String>,
}

/// Query params for `POST /v1/repos/{repoId}/build?name=...&version=...&org=...&branch=...` —
/// (re)builds the index from whatever files have been pushed so far for this repo.
#[derive(Debug, Deserialize)]
pub struct BuildRepoQuery {
    pub name: String,
    /// Opaque version/revision tag, surfaced back via `/v1/index/status` and as `blobSha`
    /// in search results.
    #[serde(default)]
    pub version: String,
    /// Opaque grouping label (e.g. GitHub owner or Azure DevOps project name), surfaced back via
    /// `/v1/index/status` and search results/facets for filtering.
    #[serde(default)]
    pub org: String,
    /// Opaque branch name indexed, surfaced the same way as `org`.
    #[serde(default)]
    pub branch: String,
}
