use crate::models::LinkTemplate;
use crate::persist;
use crate::shard::{self, Shard};
use crate::trigram::TrigramIndex;
use anyhow::Result;
use bytes::Bytes;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A file's content, resolved from either a repo's push-phase buffer or its built shard. Cheap
/// to clone in both cases: `Bytes` is refcounted, and the shard case just clones an `Arc` plus
/// a path string, deferring the actual byte lookup (a slice into the shard's `mmap`) until
/// [`FileContent::as_bytes`] is called.
#[derive(Clone)]
pub enum FileContent {
    Pending(Bytes),
    Shard(Arc<Shard>, String),
}

impl FileContent {
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            FileContent::Pending(bytes) => Some(bytes.as_ref()),
            FileContent::Shard(shard, path) => shard.get(path),
        }
    }
}

/// A repo's working set: files pushed one at a time, plus the trigram index built from them
/// (rebuilt wholesale on each `build` call — an incremental index is a possible future
/// optimization, not implemented).
///
/// File content lives in two places depending on lifecycle stage:
/// - `pending`: files pushed via `put_file` since the last `build`, buffered in the Rust heap
///   (transient, bounded by one push burst).
/// - `shard`: the on-disk, `mmap`-backed blob written by the last `build` call — the actual
///   resident-memory win, since the OS page cache (not this process's heap) decides what stays
///   resident. `pending` always takes priority over `shard` for a given path, so a file pushed
///   again after a build is served fresh until the next build folds it back into the shard.
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
    pub pending: HashMap<String, Bytes>,
    pub shard: Option<Arc<Shard>>,
    /// Paths deleted via `delete_file` since the last build, masking a same-path entry still
    /// present in `shard` (the shard itself isn't rewritten until the next `build`).
    pub shard_deleted: HashSet<String>,
    pub file_order: Vec<String>,
    pub index: Option<Arc<TrigramIndex>>,
}

impl RepoData {
    /// Resolves a path to its current content, respecting `pending` overrides and
    /// `shard_deleted` masking. `None` means the path isn't part of this repo's working set.
    pub fn get_content(&self, path: &str) -> Option<FileContent> {
        if let Some(bytes) = self.pending.get(path) {
            return Some(FileContent::Pending(bytes.clone()));
        }
        if self.shard_deleted.contains(path) {
            return None;
        }
        let shard = self.shard.as_ref()?;
        if shard.contains(path) {
            Some(FileContent::Shard(shard.clone(), path.to_string()))
        } else {
            None
        }
    }
}

pub type RepoMap = Arc<RwLock<HashMap<String, RepoData>>>;

/// The whole indexing engine: an in-memory, per-repo file store plus an on-demand trigram
/// index. Built from scratch for muck — not a wrapper around an external search
/// library or CLI tool. File content is stored in `mmap`-backed on-disk shards (see
/// `crate::shard`) rather than fully resident in the Rust heap; the trigram index itself
/// (`crate::trigram::TrigramIndex`) stays heap-resident but uses a compact flat-buffer
/// encoding, not a naive `HashMap<_, Vec<_>>` — see that module's doc comment for why the
/// naive form was a real memory problem at scale, not just a hypothetical one. The only other
/// thing that touches disk is the optional backup file described on `persist_path`.
pub struct Store {
    pub repos: RepoMap,
    /// Directory shard files are written to. Defaults to `MUCK_SHARD_DIR` if set, otherwise a
    /// `muck-shards` subdirectory of the OS temp dir (see `Store::new`/`new_with_persistence`).
    shard_dir: PathBuf,
    /// Per-repo build locks, created on first use. `build()` holds a repo's lock for its
    /// entire duration (read pending state → write shard → commit) so two overlapping `POST
    /// .../build` requests for the *same* repo serialize instead of racing to decide the
    /// repo's final `file_order`/`index`/`shard` — without this, the loser's work (including,
    /// in the old pre-shard code, its file content) could silently vanish, clobbered by
    /// whichever request's final write landed last. Different repos still build fully
    /// concurrently — this only serializes same-repo builds against each other.
    build_locks: Arc<RwLock<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
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

/// `MUCK_SHARD_DIR` if set, otherwise a `muck-shards` subdirectory of the OS temp dir. Shard
/// files always live on local disk — see `HANDOFF_STORAGE_OPTIMIZATION.md` for why muck's
/// "purely in-memory" framing no longer applies to file content specifically (the trigram
/// index and push-phase buffering are still pure heap memory).
fn shard_dir_from_env() -> PathBuf {
    match std::env::var("MUCK_SHARD_DIR") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => std::env::temp_dir().join("muck-shards"),
    }
}

impl Store {
    pub fn new() -> Self {
        let shard_dir = shard_dir_from_env();
        // No persistence, so nothing will ever be loaded into `repos` — any shard file
        // already in `shard_dir` (left behind by a previous process run, since nothing
        // cleans these up on the way out) is orphaned as of this instant. Fire-and-forget:
        // doesn't block startup or readiness, just reclaims disk in the background.
        tokio::spawn(shard::purge_orphaned_shards(shard_dir.clone(), Vec::new()));
        Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
            shard_dir,
            build_locks: Arc::new(RwLock::new(HashMap::new())),
            persist_path: None,
            ready: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Enables disk backup/restore at `path`. If `path` already exists, the returned store
    /// starts with `ready == false` and a background task is spawned to load it; `ready`
    /// flips to `true` once that load attempt finishes (success or failure — a corrupt
    /// backup file is logged, not fatal, so the process doesn't stay unhealthy forever over
    /// a bad snapshot). If `path` doesn't exist yet (first run), `ready` starts `true`. Either
    /// way, a background task also purges any shard file under the shard directory that
    /// doesn't belong to a just-loaded repo — see `shard::purge_orphaned_shards`.
    pub fn new_with_persistence(path: PathBuf) -> Arc<Self> {
        let needs_load = path.exists();
        let shard_dir = shard_dir_from_env();
        let store = Arc::new(Self {
            repos: Arc::new(RwLock::new(HashMap::new())),
            shard_dir,
            build_locks: Arc::new(RwLock::new(HashMap::new())),
            persist_path: Some(path.clone()),
            ready: Arc::new(AtomicBool::new(!needs_load)),
        });

        let store_bg = store.clone();
        tokio::spawn(async move {
            if needs_load {
                match persist::load(&store_bg.repos, &store_bg.shard_dir, &path).await {
                    Ok(count) => tracing::info!(repos = count, ?path, "loaded persisted store"),
                    Err(error) => tracing::error!(?path, "failed to load persisted store: {error:#}"),
                }
            }
            let repo_ids: Vec<String> = store_bg.repos.read().await.keys().cloned().collect();
            shard::purge_orphaned_shards(store_bg.shard_dir.clone(), repo_ids).await;
            store_bg.ready.store(true, Ordering::SeqCst);
        });

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
        repo.pending.insert(path.to_string(), content);
    }

    pub async fn delete_file(&self, repo_id: &str, path: &str) {
        if let Some(repo) = self.repos.write().await.get_mut(repo_id) {
            repo.pending.remove(path);
            repo.shard_deleted.insert(path.to_string());
        }
    }

    /// (Re)builds the trigram index and on-disk shard from whatever files are currently
    /// pushed for this repo (shard content still present from the last build, minus anything
    /// in `shard_deleted`, plus/overridden-by anything in `pending`), and updates its
    /// name/version/org/branch. Building runs off the async runtime (spawn_blocking) since
    /// it's a CPU-bound scan over every file's bytes plus a disk write.
    pub async fn build(
        &self,
        repo_id: String,
        name: String,
        version: String,
        org: String,
        branch: String,
        links: Vec<LinkTemplate>,
    ) -> Result<()> {
        // Held for the whole method: two overlapping builds for the same repo must not
        // interleave their read-current-state/write-shard/commit sequences against each
        // other (see the `build_locks` field doc for what goes wrong if they do). Builds for
        // *different* repos don't share a lock and still run fully concurrently.
        let repo_lock = {
            let mut locks = self.build_locks.write().await;
            locks.entry(repo_id.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
        };
        let _build_guard = repo_lock.lock().await;

        let (order, contents, old_shard) = {
            let repos = self.repos.read().await;
            let repo = repos
                .get(&repo_id)
                .ok_or_else(|| anyhow::anyhow!("no files pushed yet for repo '{repo_id}'"))?;

            let mut order: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            if let Some(shard) = &repo.shard {
                for path in &repo.file_order {
                    if repo.shard_deleted.contains(path) || repo.pending.contains_key(path) {
                        continue;
                    }
                    if shard.contains(path) && seen.insert(path.clone()) {
                        order.push(path.clone());
                    }
                }
            }
            for path in repo.pending.keys() {
                if seen.insert(path.clone()) {
                    order.push(path.clone());
                }
            }

            let contents: Vec<FileContent> = order
                .iter()
                .map(|path| repo.get_content(path).expect("path just collected from repo's own state"))
                .collect();

            (order, contents, repo.shard.clone())
        };

        let shard_dir = self.shard_dir.clone();
        let build_repo_id = repo_id.clone();
        let (order, index, new_shard) =
            tokio::task::spawn_blocking(move || -> Result<(Vec<String>, TrigramIndex, Shard)> {
                let refs: Vec<&[u8]> = contents.iter().map(|c| c.as_bytes().unwrap_or(&[])).collect();
                let index = TrigramIndex::build(&refs);
                let new_shard = shard::write_shard(&shard_dir, &build_repo_id, &order, &refs)?;
                Ok((order, index, new_shard))
            })
            .await??;

        {
            let mut repos = self.repos.write().await;
            if let Some(repo) = repos.get_mut(&repo_id) {
                repo.name = name;
                repo.version = version;
                repo.org = org;
                repo.branch = branch;
                repo.links = links;
                repo.file_order = order;
                repo.shard = Some(Arc::new(new_shard));
                repo.pending.clear();
                repo.shard_deleted.clear();
                repo.index = Some(Arc::new(index));
            }
        }
        // The old shard is superseded by the one just written above; drop its file now that
        // nothing references it. Best-effort, matches `Shard::remove_file`'s own doc comment.
        if let Some(old_shard) = old_shard {
            old_shard.remove_file();
        }
        self.persist().await;
        Ok(())
    }

    pub async fn unregister(&self, repo_id: &str) -> bool {
        let removed = self.repos.write().await.remove(repo_id);
        let existed = removed.is_some();
        if let Some(repo) = removed {
            if let Some(shard) = repo.shard {
                shard.remove_file();
            }
        }
        // Best-effort hygiene, not correctness-critical: drops this repo's entry from
        // `build_locks` so a permanently-deleted repo doesn't leak an `Arc<Mutex<()>>`
        // forever. If a build for this exact repo id is racing this call, that build already
        // holds its own clone of the `Arc` and is unaffected — it just won't be found by a
        // *future* `build()` call for the same id, which will allocate a fresh lock instead.
        self.build_locks.write().await.remove(repo_id);
        if existed {
            self.persist().await;
        }
        existed
    }

    /// Read back a single file's raw bytes, as pushed via `put_file` — used by the
    /// `muck-local` UI variant to render file content (no ADO/disk round-trip needed).
    pub async fn get_file(&self, repo_id: &str, path: &str) -> Option<Bytes> {
        let repos = self.repos.read().await;
        let content = repos.get(repo_id)?.get_content(path)?;
        match content {
            FileContent::Pending(bytes) => Some(bytes),
            FileContent::Shard(shard, path) => shard.get(&path).map(Bytes::copy_from_slice),
        }
    }

    /// Flat list of every path currently pushed for a repo, in the order last built — used by
    /// the `muck-local` UI variant to populate a file tree client-side.
    pub async fn list_paths(&self, repo_id: &str) -> Option<Vec<String>> {
        let repos = self.repos.read().await;
        let repo = repos.get(repo_id)?;
        if repo.file_order.is_empty() && !repo.pending.is_empty() {
            return Some(repo.pending.keys().cloned().collect());
        }
        Some(repo.file_order.clone())
    }
}
