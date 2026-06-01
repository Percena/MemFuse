use mfs_workspace::WorkspaceFs;

#[tokio::test]
async fn supports_ls_tree_stat_and_read_for_materialized_resources() {
    let fs = WorkspaceFs::from_fixture(
        "acme",
        "alice",
        "coding-agent",
        "tests/fixtures/localfs_docs",
    )
    .await
    .unwrap();

    let listing = fs.ls("mfs://resources/localfs/docs").await.unwrap();
    assert!(listing.iter().any(|entry| entry.name == "api.md"));

    let tree = fs.tree("mfs://resources/localfs/docs", 2).await.unwrap();
    assert!(tree.is_dir);
    assert!(tree.children.iter().any(|child| child.name == "api.md"));

    let stat = fs
        .stat("mfs://resources/localfs/docs/api.md")
        .await
        .unwrap();
    assert!(!stat.is_dir);

    let body = fs
        .read("mfs://resources/localfs/docs/api.md")
        .await
        .unwrap();
    assert!(body.contains("Authentication"));
}

#[tokio::test]
async fn supports_reading_directory_abstract_and_overview() {
    let fs = WorkspaceFs::from_fixture(
        "acme",
        "alice",
        "coding-agent",
        "tests/fixtures/localfs_docs",
    )
    .await
    .unwrap();

    let abstract_text = fs
        .abstract_text("mfs://resources/localfs/docs")
        .await
        .unwrap();
    let overview_text = fs
        .overview_text("mfs://resources/localfs/docs")
        .await
        .unwrap();

    assert!(abstract_text.contains("Deterministic summary"));
    assert!(overview_text.contains("# Overview"));
    assert!(overview_text.contains("mfs://resources/localfs/docs"));
}

#[tokio::test]
async fn supports_glob_patterns_for_materialized_resources() {
    let fs = WorkspaceFs::from_fixture(
        "acme",
        "alice",
        "coding-agent",
        "tests/fixtures/localfs_docs",
    )
    .await
    .unwrap();

    let matches = fs
        .glob("mfs://resources/localfs/docs", "guides/**/*.md")
        .await
        .unwrap();

    assert!(matches.contains(&"mfs://resources/localfs/docs/guides/oauth.md".to_owned()));
    assert!(matches.contains(&"mfs://resources/localfs/docs/guides/tokens/refresh.md".to_owned()));
}

#[tokio::test]
async fn supports_owned_write_plane_for_user_paths() {
    let workspace = tempfile::tempdir().unwrap();
    let fs = WorkspaceFs::open_existing_for_uri(
        workspace.path(),
        "acme",
        "alice",
        "coding-agent",
        Some("mfs://user/notes"),
    )
    .unwrap();

    fs.mkdir("mfs://user/notes").await.unwrap();
    fs.write_text(
        "mfs://user/notes/profile.md",
        "OAuth token rotation playbook",
    )
    .await
    .unwrap();
    assert_eq!(
        fs.read("mfs://user/notes/profile.md").await.unwrap(),
        "OAuth token rotation playbook"
    );

    fs.move_path("mfs://user/notes/profile.md", "mfs://user/notes/renamed.md")
        .await
        .unwrap();
    assert_eq!(
        fs.read("mfs://user/notes/renamed.md").await.unwrap(),
        "OAuth token rotation playbook"
    );

    fs.remove_path("mfs://user/notes/renamed.md").await.unwrap();
    assert!(fs.read("mfs://user/notes/renamed.md").await.is_err());
}
