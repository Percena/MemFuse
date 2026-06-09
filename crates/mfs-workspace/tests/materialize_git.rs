use std::process::Command;

use mfs_types::IdentityContext;
use mfs_workspace::Materializer;

#[tokio::test]
async fn materializes_git_source_into_user_isolated_projection_view() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join("docs")).unwrap();
    std::fs::write(repo.path().join("docs/guide.md"), "# Git Guide\n").unwrap();

    run_git(repo.path(), &["init", "--initial-branch", "main"]);
    run_git(repo.path(), &["config", "user.email", "ci@example.com"]);
    run_git(repo.path(), &["config", "user.name", "CI"]);
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "initial"]);

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let result = Materializer::for_tests()
        .materialize_git(
            &identity,
            repo.path().to_str().unwrap(),
            "mfs://resources/git/docs",
        )
        .await
        .unwrap();

    assert!(
        result
            .workspace_root
            .join("tenants/acme/alice/resources/git/docs")
            .exists()
    );
    assert_eq!(result.provenance.source_kind, "git");
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {:?}", args);
}
