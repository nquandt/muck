//! Optional local-disk backup/restore for [`crate::store::Store`], enabled by pointing the
//! `MUCK_PERSIST_PATH` env var at a file path (see `main.rs`/`bin/local.rs`).
//!
//! muck's normal operating mode is purely in-memory — this exists only so a single
//! instance can survive a restart without callers having to re-push and rebuild every repo.
//! It is deliberately single-instance, local-filesystem only: there is no locking, no
//! multi-writer coordination, and no attempt to make the file shareable across
//! horizontally-scaled instances. If that's ever needed, it's a different feature (a shared
//! store, not a local backup file) and should be designed separately.

use crate::models::LinkTemplate;
use crate::shard;
use crate::store::{RepoData, RepoMap};
use crate::trigram::TrigramIndex;
use anyhow::Result;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// On-disk representation of a repo. Deliberately omits the trigram index — it's cheap to
/// rebuild from `files` on load and serializing a `HashMap<[u8; 3], HashSet<u32>>` would
/// bloat the file for no benefit. `files` is a `Vec` (not a `HashMap`) purely to preserve
/// insertion order, so a reloaded repo's `file_order` matches what it was before restart.
#[derive(Serialize, Deserialize)]
struct PersistedRepo {
    name: String,
    version: String,
    org: String,
    branch: String,
    #[serde(default)]
    links: Vec<LinkTemplate>,
    files: Vec<(String, Bytes)>,
}

type PersistedStore = HashMap<String, PersistedRepo>;

/// Snapshots `repos` and writes it to `path`. Writes to a `.tmp` sibling file first and
/// renames it into place, so a process killed mid-write leaves the previous snapshot intact
/// rather than a truncated/corrupt one.
pub async fn save(repos: &RepoMap, path: &Path) -> Result<()> {
    let snapshot: PersistedStore = {
        let repos = repos.read().await;
        repos
            .iter()
            .map(|(id, repo)| {
                let files = repo
                    .file_order
                    .iter()
                    .filter_map(|p| {
                        repo.get_content(p)
                            .and_then(|c| c.as_bytes().map(|b| (p.clone(), Bytes::copy_from_slice(b))))
                    })
                    .collect();
                (
                    id.clone(),
                    PersistedRepo {
                        name: repo.name.clone(),
                        version: repo.version.clone(),
                        org: repo.org.clone(),
                        branch: repo.branch.clone(),
                        links: repo.links.clone(),
                        files,
                    },
                )
            })
            .collect()
    };

    let bytes = tokio::task::spawn_blocking(move || bincode::serialize(&snapshot)).await??;

    let tmp_path = path.with_extension("tmp");
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&tmp_path, &bytes).await?;
    tokio::fs::rename(&tmp_path, path).await?;
    Ok(())
}

/// Loads a snapshot written by [`save`] and populates `repos` with it, rebuilding each repo's
/// on-disk shard (under `shard_dir`) and trigram index along the way. Returns the number of
/// repos loaded.
pub async fn load(repos: &RepoMap, shard_dir: &Path, path: &Path) -> Result<usize> {
    let bytes = tokio::fs::read(path).await?;
    let snapshot: PersistedStore =
        tokio::task::spawn_blocking(move || bincode::deserialize(&bytes)).await??;

    let count = snapshot.len();
    for (id, persisted) in snapshot {
        let file_order: Vec<String> = persisted.files.iter().map(|(p, _)| p.clone()).collect();
        let docs: Vec<Bytes> = persisted.files.into_iter().map(|(_, b)| b).collect();
        let shard_dir = shard_dir.to_path_buf();
        let repo_id = id.clone();
        let build_order = file_order.clone();
        let (index, new_shard) = tokio::task::spawn_blocking(move || -> Result<_> {
            let refs: Vec<&[u8]> = docs.iter().map(|b| b.as_ref()).collect();
            let index = TrigramIndex::build(&refs);
            let new_shard = shard::write_shard(&shard_dir, &repo_id, &build_order, &refs)?;
            Ok((index, new_shard))
        })
        .await??;

        repos.write().await.insert(
            id,
            RepoData {
                name: persisted.name,
                version: persisted.version,
                org: persisted.org,
                branch: persisted.branch,
                links: persisted.links,
                pending: HashMap::new(),
                shard: Some(Arc::new(new_shard)),
                shard_deleted: HashSet::new(),
                file_order,
                index: Some(Arc::new(index)),
            },
        );
    }
    Ok(count)
}
