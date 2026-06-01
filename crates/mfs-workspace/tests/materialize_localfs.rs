use mfs_types::IdentityContext;
use mfs_workspace::Materializer;

#[tokio::test]
async fn materializes_localfs_source_into_user_isolated_projection_view() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let result = Materializer::for_tests()
        .materialize_localfs(
            &ctx,
            "tests/fixtures/localfs_docs",
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();

    assert!(
        result
            .workspace_root
            .join("tenants/acme/alice/resources/localfs/docs")
            .exists()
    );
    assert!(result.provenance.source_kind == "localfs");
    assert_ne!(result.provenance.source_snapshot_id, "pending");
}

#[tokio::test]
async fn materializes_single_localfs_file_into_projection_view() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let source = tempfile::tempdir().unwrap();
    let source_file = source.path().join("notes.md");
    std::fs::write(&source_file, "# Notes\nsingle file resource\n").unwrap();

    let result = Materializer::for_tests()
        .materialize_localfs(&ctx, source_file.to_str().unwrap(), "mfs://resources/notes")
        .await
        .unwrap();

    let projection_root = result
        .workspace_root
        .join("tenants/acme/alice/resources/notes");
    assert!(projection_root.join("notes.md").exists());
    assert!(projection_root.join(".abstract.md").exists());
    assert!(projection_root.join(".overview.md").exists());
}
