//! On-disk, `mmap`-backed storage for a repo's file content, replacing the old
//! `HashMap<String, Bytes>` fully-resident-in-heap approach. A shard is a single file: every
//! pushed file's bytes concatenated back to back, plus an in-memory offset/length table
//! (`(u64, u32)` per path) recording where each file lives in that blob. The blob itself is
//! opened via `mmap` rather than read into an owned buffer, so the OS page cache — not the
//! Rust heap — decides what's actually resident at any given moment; it shrinks under memory
//! pressure and re-faults from disk on the next query that touches a cold file.
//!
//! Deliberately minimal, unlike Zoekt's real shard format: no compression, no symbol index,
//! no compound shards. Just enough to get file bytes out from behind a path.

use anyhow::{Context, Result};
use memmap2::Mmap;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A single repo's file content, backed by one `mmap`'d shard file on disk. Rewritten wholesale
/// on every `Store::build` call, consistent with the trigram index's own "rebuilt wholesale"
/// behavior — there is no incremental update path.
pub struct Shard {
    mmap: Mmap,
    offsets: HashMap<String, (u64, u32)>,
    /// The shard file's path on disk, kept so [`Shard::remove_file`] can clean it up when a
    /// repo is unregistered or superseded by a fresh build.
    path: PathBuf,
}

impl Shard {
    /// Returns a file's raw bytes as a slice into the mmap'd region — no copy, no allocation.
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        let (offset, len) = *self.offsets.get(path)?;
        let start = offset as usize;
        let end = start + len as usize;
        self.mmap.get(start..end)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.offsets.contains_key(path)
    }

    /// Best-effort delete of the underlying shard file. Errors are logged, not propagated: on
    /// Windows in particular, deleting a file that's still `mmap`'d can fail depending on how
    /// the mapping was opened, and this is always called during teardown (unregister/rebuild)
    /// where there's no meaningful way to react to failure beyond leaking the old shard file.
    pub fn remove_file(&self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            tracing::warn!(path = ?self.path, "failed to remove old shard file: {error:#}");
        }
    }
}

/// Turns a repo id into a filesystem-safe file name — repo ids are opaque caller-supplied
/// strings and may contain characters (`/`, `:`) that aren't safe as a bare file name, or be
/// long enough to blow past filesystem path-length limits. The sanitized prefix is kept only
/// for human-readability when eyeballing the shard directory; the hash suffix (of the
/// *original*, unsanitized id) is what actually guarantees uniqueness — two different repo
/// ids that sanitize to the same string (e.g. `"a/b"` and `"a:b"`, both becoming `"a_b"`)
/// still get different filenames, since the hash is computed before sanitizing/truncating.
/// `DefaultHasher::new()` uses fixed (non-randomized) keys, so this is deterministic across
/// calls within the same process and across restarts of the same build — unlike `HashMap`'s
/// usual `RandomState`, which reseeds every process.
fn shard_file_name(repo_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    repo_id.hash(&mut hasher);
    let hash = hasher.finish();

    let sanitized: String = repo_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(64)
        .collect();
    format!("{sanitized}-{hash:016x}.shard")
}

/// Distinguishes concurrent `write_shard` calls' tmp files from each other. Two `build` calls
/// for the same repo can legitimately race (nothing in `Store::build` serializes them per
/// repo) — without a unique tmp name they'd both write through the same path and could
/// interleave, corrupting whichever one loses the final rename. A per-process atomic counter
/// is enough: uniqueness only needs to hold within one process's lifetime, since the tmp file
/// never survives past its own rename.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes a fresh shard file for `repo_id` under `dir` containing `contents` (in `order`,
/// zero-indexed to match the trigram index's doc ids), then `mmap`s it and returns the result.
/// Writes to a uniquely-named tmp sibling first and renames into place, matching
/// `persist.rs`'s crash-safety pattern — a process killed mid-write leaves no partial shard
/// file behind, and concurrent builds for the same repo can't corrupt each other's tmp file
/// (see `TMP_COUNTER`); the final rename still picks one winner if they race, consistent with
/// `build`'s existing "rebuilt wholesale, last write wins" semantics.
pub fn write_shard(dir: &Path, repo_id: &str, order: &[String], contents: &[&[u8]]) -> Result<Shard> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating shard dir {dir:?}"))?;

    let final_path = dir.join(shard_file_name(repo_id));
    let unique = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = dir.join(format!("{}.tmp-{}-{unique}", shard_file_name(repo_id), std::process::id()));

    let mut offsets = HashMap::with_capacity(order.len());
    {
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("creating shard tmp file {tmp_path:?}"))?;
        let mut cursor: u64 = 0;
        for (path, content) in order.iter().zip(contents.iter()) {
            file.write_all(content)?;
            offsets.insert(path.clone(), (cursor, content.len() as u32));
            cursor += content.len() as u64;
        }
        if cursor == 0 {
            // `Mmap::map` refuses to map a zero-length file; pad with a single unused byte so
            // an empty repo (or a repo of only empty files) still gets a valid mapping. Not
            // part of any file's offset range, so it's never observed by `Shard::get`.
            file.write_all(&[0u8])?;
        }
        file.flush()?;
    }
    std::fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("renaming {tmp_path:?} to {final_path:?}"))?;

    let file = File::open(&final_path).with_context(|| format!("opening shard {final_path:?}"))?;
    // Safety: the shard file is exclusively owned by this process (unique per-repo path,
    // written atomically above via tmp+rename) and never mutated in place after this point —
    // a rebuild always writes a brand new file rather than modifying an existing mapping.
    let mmap = unsafe { Mmap::map(&file) }.with_context(|| format!("mmap {final_path:?}"))?;

    Ok(Shard { mmap, offsets, path: final_path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_file_content_through_offsets() {
        let dir = std::env::temp_dir().join(format!("muck-shard-test-{}", std::process::id()));
        let order = vec!["a.txt".to_string(), "b.txt".to_string()];
        let contents: Vec<&[u8]> = vec![b"hello", b"world!"];
        let shard = write_shard(&dir, "repo/with:colons", &order, &contents).unwrap();

        assert_eq!(shard.get("a.txt"), Some(&b"hello"[..]));
        assert_eq!(shard.get("b.txt"), Some(&b"world!"[..]));
        assert_eq!(shard.get("missing.txt"), None);
        assert!(shard.contains("a.txt"));
        assert!(!shard.contains("missing.txt"));

        shard.remove_file();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_repo_still_maps() {
        let dir = std::env::temp_dir().join(format!("muck-shard-test-empty-{}", std::process::id()));
        let shard = write_shard(&dir, "empty-repo", &[], &[]).unwrap();
        assert_eq!(shard.get("anything"), None);
        shard.remove_file();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn purge_removes_only_unreferenced_shards() {
        let dir = std::env::temp_dir().join(format!("muck-shard-test-purge-{}", std::process::id()));
        let keep_shard = write_shard(&dir, "keep-me", &["a".to_string()], &[b"x"]).unwrap();
        let drop_shard = write_shard(&dir, "drop-me", &["a".to_string()], &[b"x"]).unwrap();
        let keep_path = keep_shard.path.clone();
        let drop_path = drop_shard.path.clone();
        assert!(keep_path.exists());
        assert!(drop_path.exists());

        purge_orphaned_shards(dir.clone(), vec!["keep-me".to_string()]).await;

        assert!(keep_path.exists(), "kept repo's shard should survive purge");
        assert!(!drop_path.exists(), "unreferenced repo's shard should be purged");

        drop(keep_shard);
        drop(drop_shard);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Deletes any `*.shard` file in `dir` that doesn't belong to one of `keep_repo_ids` — run
/// once at `Store` startup (see `store.rs`) to clean up shard files left behind by a previous
/// process run that a fresh `Store` has no `Shard` handle for and will therefore never call
/// `Shard::remove_file` on otherwise: a crash mid-build (old shard never got superseded so
/// never got removed), or restarting with persistence disabled (nothing ever reloads those
/// repos, so their shards are permanently orphaned without this). Best-effort — a missing or
/// unreadable `dir` (e.g. first run, nothing written yet) is not an error.
pub async fn purge_orphaned_shards(dir: PathBuf, keep_repo_ids: Vec<String>) {
    let keep: HashSet<String> = keep_repo_ids.iter().map(|id| shard_file_name(id)).collect();
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(_) => return,
    };
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(_) => break,
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Only ever touch files this module itself could have written (`*.shard` and its
        // `*.shard.tmp-*` staging files) — never delete anything else that might be sharing
        // the directory, e.g. if `MUCK_SHARD_DIR` is pointed at a non-dedicated path.
        let is_ours = name.ends_with(".shard") || name.contains(".shard.tmp-");
        if is_ours && !keep.contains(name.as_ref()) {
            if let Err(error) = tokio::fs::remove_file(entry.path()).await {
                tracing::warn!(path = ?entry.path(), "failed to remove orphaned shard file: {error:#}");
            }
        }
    }
}
