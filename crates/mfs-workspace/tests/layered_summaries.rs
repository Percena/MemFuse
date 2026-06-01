use std::fs;

use mfs_types::IdentityContext;
use mfs_workspace::{AdaptiveBudget, Materializer, WorkspaceFs};

#[test]
fn computes_larger_l1_budget_for_large_documents() {
    let small = AdaptiveBudget::for_tokens(400, "markdown");
    let large = AdaptiveBudget::for_tokens(24_000, "pdf");

    assert!(small.l1_target < large.l1_target);
    assert!(small.l0_target <= 150);
}

#[tokio::test]
async fn materialization_writes_layered_summaries_for_localfs_resources() {
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let result = Materializer::for_tests()
        .materialize_localfs(
            &identity,
            "tests/fixtures/localfs_docs",
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();

    let abstract_text = fs::read_to_string(result.target_path.join(".abstract.md")).unwrap();
    let overview_text = fs::read_to_string(result.target_path.join(".overview.md")).unwrap();

    assert!(abstract_text.contains("api.md"));
    assert!(abstract_text.contains("guide.md"));

    assert!(overview_text.contains("mfs://resources/localfs/docs"));
    assert!(overview_text.contains("api.md"));
    assert!(overview_text.contains("guide.md"));
    assert!(overview_text.contains("Authentication"));
}

#[tokio::test]
async fn materialization_writes_file_level_layered_summaries() {
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let result = Materializer::for_tests()
        .materialize_localfs(
            &identity,
            "tests/fixtures/localfs_docs",
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();
    let fs = WorkspaceFs::from_fixture(
        "acme",
        "alice",
        "coding-agent",
        "tests/fixtures/localfs_docs",
    )
    .await
    .unwrap();

    let abstract_text = fs
        .abstract_text("mfs://resources/localfs/docs/api.md")
        .await
        .unwrap();
    let overview_text = fs
        .overview_text("mfs://resources/localfs/docs/api.md")
        .await
        .unwrap();

    assert!(result.target_path.join("api.md.abstract.md").exists());
    assert!(result.target_path.join("api.md.overview.md").exists());
    assert!(abstract_text.contains("api.md"));
    assert!(overview_text.contains("mfs://resources/localfs/docs/api.md"));
    assert!(!overview_text.contains("Resource: `mfs://resources/localfs/docs`"));
    assert!(overview_text.contains("Authentication"));
}
