use crate::models::LinkTemplate;
use crate::persist;
use crate::trigram::TrigramIndex;
use anyhow::Result;
use bytes::Bytes;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A repo's working set: files pushed one at a time, plus the trigram index built from them
/// (rebuilt wholesale on each `build` call — an incremental index is a possible future
/// optimization, not implemented). `files` is the only copy of the content that exists
/// anywhere in this process, unless optional disk persistence is enabled (see `persist`).
#[derive(Default)]
pub struct RepoData {
    pub name: String,
    pub version: String,
    /// Opaque caller-supplied grouping label (GitHub owner, ADO project, etc).
    pub org: String,
    /// Opaque caller-supplied branch name.
    pub branch: String,
    /// Caller-supplied "open this file elsewhere" link templates, set at build time.
    pub links: Vec<LinkTemplate>,
    pub files: HashMap<String, Bytes>,
    pub file_order: Vec<String>,
    pub index: Option<Arc<TrigramIndex>>,
}

pub type RepoMap = Arc<RwLock<HashMap<String, RepoData>>>;

/// The whole indexing engine: an in-memory, per-repo file store plus an on-demand trigram
/// index. Built from scratch for muck — not a wrapper around an external search
/// library or CLI tool. Storage is pure heap memory (Bytes/HashMap); the only thing that
/// ever touches disk is the optional backup file described on `persist_path`.
pub struct Store {
    pub repos: RepoMap,
    /// Set from the `MUCK_PERSIST_PATH` env var. When set, the store's full contents are
    /// written to this path (via [`persist::save`]) after every `build`/`unregister` call,
    /// and loaded back from it (via [`persist::load`]) on startup if the file already
    /// exists. Local filesystem only — deliberately not designed to be shared across
    /// horizontally-scaled instances; each instance backs up to (and restores from) its own
    /// local disk.
    persist_path: Option<PathBuf>,
    /// `false` while an on-disk snapshot is still being loaded at startup — see
    /// [`Store::new_with_persistence`] and `handlers::health`, which reports `503` while
    /// this is `false` instead of `200`. Always `true` when persistence is disabled.
    pub ready: Arc<AtomicBool>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
            persist_path: None,
            ready: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Enables disk backup/restore at `path`. If `path` already exists, the returned store
    /// starts with `ready == false` and a background task is spawned to load it; `ready`
    /// flips to `true` once that load attempt finishes (success or failure — a corrupt
    /// backup file is logged, not fatal, so the process doesn't stay unhealthy forever over
    /// a bad snapshot). If `path` doesn't exist yet (first run), `ready` starts `true`.
    pub fn new_with_persistence(path: PathBuf) -> Arc<Self> {
        let needs_load = path.exists();
        let store = Arc::new(Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
            persist_path: Some(path.clone()),
            ready: Arc::new(AtomicBool::new(!needs_load)),
        });

        if needs_load {
            let store = store.clone();
            tokio::spawn(async move {
                match persist::load(&store.repos, &path).await {
                    Ok(count) => tracing::info!(repos = count, ?path, "loaded persisted store"),
                    Err(error) => tracing::error!(?path, "failed to load persisted store: {error:#}"),
                }
                store.ready.store(true, Ordering::SeqCst);
            });
        }

        store
    }

    /// Best-effort backup after a mutation that changes committed (built/unregistered)
    /// state. Errors are logged, not propagated — persistence is a backup, not the source
    /// of truth, so a write failure here shouldn't fail the request that triggered it.
    async fn persist(&self) {
        if let Some(path) = &self.persist_path {
            if let Err(error) = persist::save(&self.repos, path).await {
                tracing::error!(?path, "failed to persist store: {error:#}");
            }
        }
    }

    pub async fn put_file(&self, repo_id: &str, path: &str, content: Bytes) {
        let mut repos = self.repos.write().await;
        let repo = repos.entry(repo_id.to_string()).or_default();
        repo.files.insert(path.to_string(), content);
    }

    pub async fn delete_file(&self, repo_id: &str, path: &str) {
        if let Some(repo) = self.repos.write().await.get_mut(repo_id) {
            repo.files.remove(path);
        }
    }

    /// (Re)builds the trigram index from whatever files are currently pushed for this repo,
    /// and updates its name/version/org/branch. Building runs off the async runtime
    /// (spawn_blocking) since it's a CPU-bound scan over every file's bytes.
    pub async fn build(
        &self,
        repo_id: String,
        name: String,
        version: String,
        org: String,
        branch: String,
        links: Vec<LinkTemplate>,
    ) -> Result<()> {
        let (file_order, docs) = {
            let repos = self.repos.read().await;
            let repo = repos
                .get(&repo_id)
                .ok_or_else(|| anyhow::anyhow!("no files pushed yet for repo '{repo_id}'"))?;
            let file_order: Vec<String> = repo.files.keys().cloned().collect();
            let docs: Vec<Bytes> = file_order.iter().map(|p| repo.files[p].clone()).collect();
            (file_order, docs)
        };

        let index = tokio::task::spawn_blocking(move || TrigramIndex::build(&docs)).await?;

        {
            let mut repos = self.repos.write().await;
            if let Some(repo) = repos.get_mut(&repo_id) {
                repo.name = name;
                repo.version = version;
                repo.org = org;
                repo.branch = branch;
                repo.links = links;
                repo.file_order = file_order;
                repo.index = Some(Arc::new(index));
            }
        }
        self.persist().await;
        Ok(())
    }

    pub async fn unregister(&self, repo_id: &str) -> bool {
        let removed = self.repos.write().await.remove(repo_id).is_some();
        if removed {
            self.persist().await;
        }
        removed
    }

    /// Read back a single file's raw bytes, as pushed via `put_file` — used by the
    /// `muck-local` UI variant to render file content (no ADO/disk round-trip needed,
    /// the bytes are already sitting in memory here).
    pub async fn get_file(&self, repo_id: &str, path: &str) -> Option<Bytes> {
        self.repos.read().await.get(repo_id)?.files.get(path).cloned()
    }

    /// Flat list of every path currently pushed for a repo, in the order last built — used by
    /// the `muck-local` UI variant to populate a file tree client-side.
    pub async fn list_paths(&self, repo_id: &str) -> Option<Vec<String>> {
        let repos = self.repos.read().await;
        let repo = repos.get(repo_id)?;
        if repo.file_order.is_empty() && !repo.files.is_empty() {
            return Some(repo.files.keys().cloned().collect());
        }
        Some(repo.file_order.clone())
    }
}
