use crate::trigram::TrigramIndex;
use anyhow::Result;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A repo's working set: files pushed one at a time, plus the trigram index built from them
/// (rebuilt wholesale on each `build` call — see HANDOFF.md for the incremental-index
/// follow-up this deliberately defers). Never touches disk; `files` is the only copy of the
/// content that exists anywhere in this process.
#[derive(Default)]
pub struct RepoData {
    pub name: String,
    pub version: String,
    /// Opaque caller-supplied grouping label (GitHub owner, ADO project, etc).
    pub org: String,
    /// Opaque caller-supplied branch name.
    pub branch: String,
    pub files: HashMap<String, Bytes>,
    pub file_order: Vec<String>,
    pub index: Option<Arc<TrigramIndex>>,
}

pub type RepoMap = Arc<RwLock<HashMap<String, RepoData>>>;

/// The whole indexing engine: an in-memory, per-repo file store plus an on-demand trigram
/// index. Built from scratch for xgrep-server — not a wrapper around an external search
/// library or CLI tool. Storage is pure heap memory (Bytes/HashMap); nothing here reads or
/// writes the filesystem.
pub struct Store {
    pub repos: RepoMap,
}

impl Store {
    pub fn new() -> Self {
        Self { repos: Arc::new(RwLock::new(HashMap::new())) }
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

        let mut repos = self.repos.write().await;
        if let Some(repo) = repos.get_mut(&repo_id) {
            repo.name = name;
            repo.version = version;
            repo.org = org;
            repo.branch = branch;
            repo.file_order = file_order;
            repo.index = Some(Arc::new(index));
        }
        Ok(())
    }

    pub async fn unregister(&self, repo_id: &str) -> bool {
        self.repos.write().await.remove(repo_id).is_some()
    }

    /// Read back a single file's raw bytes, as pushed via `put_file` — used by the
    /// `xgrep-server-local` UI variant to render file content (no ADO/disk round-trip needed,
    /// the bytes are already sitting in memory here).
    pub async fn get_file(&self, repo_id: &str, path: &str) -> Option<Bytes> {
        self.repos.read().await.get(repo_id)?.files.get(path).cloned()
    }

    /// Flat list of every path currently pushed for a repo, in the order last built — used by
    /// the `xgrep-server-local` UI variant to populate a file tree client-side.
    pub async fn list_paths(&self, repo_id: &str) -> Option<Vec<String>> {
        let repos = self.repos.read().await;
        let repo = repos.get(repo_id)?;
        if repo.file_order.is_empty() && !repo.files.is_empty() {
            return Some(repo.files.keys().cloned().collect());
        }
        Some(repo.file_order.clone())
    }
}
