use std::process::Command;

use mfs_connectors::{GitConnector, ResourceConnector, SourceRef};

#[tokio::test]
async fn git_connector_enumerates_head_tree_and_reads_blob_bytes() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join("docs")).unwrap();
    std::fs::write(repo.path().join("docs/guide.md"), "# Git Fixture\n").unwrap();

    run_git(repo.path(), &["init", "--initial-branch", "main"]);
    run_git(repo.path(), &["config", "user.email", "ci@example.com"]);
    run_git(repo.path(), &["config", "user.name", "CI"]);
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "initial"]);

    let connector = GitConnector::new();
    let source = SourceRef::new("git", repo.path().to_string_lossy().to_string());
    let nodes = connector.enumerate(&source).await.unwrap();

    assert!(
        nodes
            .iter()
            .any(|node| node.relative_path().to_string_lossy() == "docs")
    );
    let guide = nodes
        .iter()
        .find(|node| node.relative_path().to_string_lossy() == "docs/guide.md")
        .unwrap()
        .clone();
    let body = connector.read_bytes(&source, &guide).await.unwrap();
    assert!(String::from_utf8(body).unwrap().contains("Git Fixture"));
}

#[test]
fn git_connector_reports_snapshot_capability() {
    let capability = GitConnector::new().capabilities();

    assert!(capability.can_enumerate);
    assert!(capability.can_read_bytes);
    assert!(capability.can_read_metadata);
    assert!(capability.supports_snapshot_materialization);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {:?}", args);
}
