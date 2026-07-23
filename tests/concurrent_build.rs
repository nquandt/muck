use bytes::Bytes;
use muck::store::Store;
use std::sync::Arc;

/// Fires many overlapping `build()` calls at the same repo concurrently and confirms the
/// store ends up in a fully self-consistent state — not a torn mix of two builds' file sets
/// (e.g. `file_order` naming a file the final `shard`/`index` don't actually have, which is
/// exactly what unserialized concurrent builds risked before `Store::build_locks` was added).
/// Doesn't (and can't, from outside) prove *which* build's result won a given race — only
/// that whichever one did, it's internally coherent.
#[tokio::test]
async fn concurrent_builds_never_leave_a_torn_state() {
    let unique = std::process::id();
    let shard_dir = std::env::temp_dir().join(format!("muck-concurrent-build-test-{unique}"));
    std::fs::remove_dir_all(&shard_dir).ok();
    unsafe { std::env::set_var("MUCK_SHARD_DIR", &shard_dir) };

    let store = Arc::new(Store::new());
    store.put_file("repo1", "a.rs", Bytes::from_static(b"fn a() {}")).await;

    let mut handles = Vec::new();
    for i in 0..20 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            store.put_file("repo1", "a.rs", Bytes::from_static(b"fn a() {}")).await;
            store
                .build(
                    "repo1".to_string(),
                    "repo-one".to_string(),
                    format!("v{i}"),
                    "acme".to_string(),
                    "main".to_string(),
                    vec![],
                )
                .await
        }));
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    // Every path `list_paths` reports must actually be readable — if two builds' writes had
    // interleaved (the bug the per-repo lock prevents), this is where it would show up as a
    // path present in `file_order` but missing from the shard that got committed alongside it.
    let paths = store.list_paths("repo1").await.unwrap();
    assert!(!paths.is_empty());
    for path in &paths {
        let content = store.get_file("repo1", path).await;
        assert!(content.is_some(), "path {path:?} listed but not readable — torn build state");
    }

    unsafe { std::env::remove_var("MUCK_SHARD_DIR") };
    std::fs::remove_dir_all(&shard_dir).ok();
}
