use bytes::Bytes;
use muck::store::Store;

/// Simulates a process restart with `MUCK_PERSIST_PATH` set: build a repo, drop the `Store`
/// (as if the process exited), then construct a fresh one pointed at the same persist file and
/// confirm the repo — and its shard-backed content — comes back correctly. Also confirms a
/// shard file from a repo that's no longer in the persisted snapshot doesn't survive the
/// restart, since nothing else in the system would ever clean it up otherwise.
#[tokio::test]
async fn repo_survives_restart_and_orphan_shard_is_purged() {
    let unique = std::process::id();
    let shard_dir = std::env::temp_dir().join(format!("muck-restart-test-shards-{unique}"));
    let persist_path = std::env::temp_dir().join(format!("muck-restart-test-{unique}.bin"));
    std::fs::remove_dir_all(&shard_dir).ok();
    std::fs::remove_file(&persist_path).ok();
    // SAFETY: tests in this crate don't run this specific env var concurrently under the same
    // temp path, since it's namespaced by this process's pid.
    unsafe { std::env::set_var("MUCK_SHARD_DIR", &shard_dir) };

    {
        let store = Store::new_with_persistence(persist_path.clone());
        // No load needed (first run) — `ready` should already be true.
        assert!(store.ready.load(std::sync::atomic::Ordering::SeqCst));

        store.put_file("repo1", "a.rs", Bytes::from_static(b"fn hello() {}")).await;
        store
            .build(
                "repo1".to_string(),
                "Repo One".to_string(),
                "v1".to_string(),
                "acme".to_string(),
                "main".to_string(),
                vec![],
            )
            .await
            .unwrap();

        // An orphan: a stray `*.shard` file `build()` never touches, simulating one left
        // behind by a repo that existed in a prior run but was never registered again (or
        // crashed mid-build) — nothing references it going forward. Its exact name doesn't
        // matter (shard filenames include a hash of the repo id, so this test can't easily
        // predict a real one) — purge only needs it to look like a shard file it could have
        // written, i.e. end in `.shard`.
        std::fs::write(shard_dir.join("orphan-repo.shard"), b"stale").unwrap();

        // store dropped here — simulates process exit.
    }

    assert!(shard_dir.join("orphan-repo.shard").exists(), "test setup sanity check");

    let store = Store::new_with_persistence(persist_path.clone());
    // Needs to load — `ready` starts false and flips true once the background load finishes.
    for _ in 0..100 {
        if store.ready.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(store.ready.load(std::sync::atomic::Ordering::SeqCst), "store never became ready");

    let content = store.get_file("repo1", "a.rs").await.unwrap();
    assert_eq!(content.as_ref(), b"fn hello() {}");

    assert!(!shard_dir.join("orphan-repo.shard").exists(), "orphaned shard should be purged on restart");
    // repo1's shard filename includes a hash of the repo id (see shard::shard_file_name), so
    // this can't predict its exact name — just confirm exactly one real shard file remains,
    // i.e. the orphan was removed and repo1's own (re-written on load) was kept.
    let remaining_shards: Vec<_> = std::fs::read_dir(&shard_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".shard"))
        .collect();
    assert_eq!(remaining_shards.len(), 1, "expected exactly repo1's shard to remain: {remaining_shards:?}");

    unsafe { std::env::remove_var("MUCK_SHARD_DIR") };
    std::fs::remove_dir_all(&shard_dir).ok();
    std::fs::remove_file(&persist_path).ok();
}
