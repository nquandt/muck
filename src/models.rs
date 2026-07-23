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
    /// to muck, surfaced for facet filtering/display only.
    pub org: String,
    /// Caller-supplied branch name indexed — opaque to muck, same as `org`.
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

/// A named "open this file elsewhere" link, supplied by the indexing caller at build time.
/// `url_template` is a token-substitution string (not a URL itself), substituted client-side
/// against `{org}`, `{repoName}`, `{branch}`, `{version}`, `{path}`, `{line}` — muck itself
/// treats it as opaque and echoes it back verbatim via `/v1/index/status`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LinkTemplate {
    pub name: String,
    #[serde(rename = "urlTemplate")]
    pub url_template: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct IndexedRepo {
    #[serde(rename = "repoId")]
    pub repo_id: String,
    #[serde(rename = "repoName")]
    pub repo_name: String,
    /// Caller-supplied version/revision tag (e.g. a git commit sha) — opaque to muck.
    pub version: String,
    pub org: String,
    pub branch: String,
    pub status: String,
    #[serde(default)]
    pub links: Vec<LinkTemplate>,
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
/// clone/checkout is never needed on the muck side.
#[derive(Debug, Deserialize)]
pub struct FilePathQuery {
    pub path: String,
}

/// Response for `GET /v1/repos/{repoId}/file?path=...` — used only by the `muck-local`
/// UI variant. `content` is omitted (left `None`) when the file looks binary.
#[derive(Debug, Serialize)]
pub struct FileContentResponse {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(rename = "isBinary")]
    pub is_binary: bool,
}

/// Response for `GET /v1/repos/{repoId}/tree` — used only by the `muck-local` UI
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
    /// A JSON-encoded `Vec<LinkTemplate>` (query strings don't carry nested arrays cleanly,
    /// so it travels as a single serialized string param rather than a JSON body) — e.g.
    /// `links=%5B%7B%22name%22%3A%22GitHub%22%2C%22urlTemplate%22%3A%22https%3A%2F%2Fgithub.com%2F%7Borg%7D%2F%7BrepoName%7D%2Fblob%2F%7Bbranch%7D%2F%7Bpath%7D%23L%7Bline%7D%22%7D%5D`.
    /// Invalid/absent JSON is treated as no links, not an error.
    #[serde(default)]
    pub links: Option<String>,
}
