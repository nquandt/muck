use bytes::Bytes;
use muck::search::{scan_repo, snapshot_candidates};
use muck::store::Store;

#[tokio::test]
async fn push_build_search_delete_rebuild_roundtrip() {
    let store = Store::new();

    store.put_file("repo1", "a.rs", Bytes::from_static(b"fn hello_world() {}")).await;
    store.put_file("repo1", "b.rs", Bytes::from_static(b"fn other() {}")).await;

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

    // File content is served from the mmap'd shard now, not an in-heap map.
    let content = store.get_file("repo1", "a.rs").await.unwrap();
    assert_eq!(content.as_ref(), b"fn hello_world() {}");

    let paths = store.list_paths("repo1").await.unwrap();
    assert_eq!(paths.len(), 2);

    let snapshots = snapshot_candidates(&store.repos, None, None, None, None, b"hello", false).await;
    assert_eq!(snapshots.len(), 1);
    let (results, _) = scan_repo(&snapshots[0], "hello", false, false, None, None, None);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, "a.rs");

    // A push after build should be visible immediately via get_file, without a rebuild.
    store.put_file("repo1", "a.rs", Bytes::from_static(b"fn hello_moon() {}")).await;
    let updated = store.get_file("repo1", "a.rs").await.unwrap();
    assert_eq!(updated.as_ref(), b"fn hello_moon() {}");

    // Delete masks the shard entry even before a rebuild.
    store.delete_file("repo1", "b.rs").await;
    assert!(store.get_file("repo1", "b.rs").await.is_none());

    // Rebuilding folds pending + deletions into a fresh shard.
    store
        .build(
            "repo1".to_string(),
            "Repo One".to_string(),
            "v2".to_string(),
            "acme".to_string(),
            "main".to_string(),
            vec![],
        )
        .await
        .unwrap();

    let paths = store.list_paths("repo1").await.unwrap();
    assert_eq!(paths, vec!["a.rs".to_string()]);

    let snapshots = snapshot_candidates(&store.repos, None, None, None, None, b"moon", false).await;
    let (results, _) = scan_repo(&snapshots[0], "moon", false, false, None, None, None);
    assert_eq!(results.len(), 1);

    assert!(store.unregister("repo1").await);
    assert!(store.list_paths("repo1").await.is_none());
}
