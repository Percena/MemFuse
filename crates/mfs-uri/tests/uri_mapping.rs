use mfs_types::IdentityContext;
use mfs_uri::{MfsUri, WorkspaceMapper};

#[test]
fn maps_resource_uri_into_account_scoped_workspace_path() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let uri = MfsUri::parse("mfs://resources/project-a/docs/api.md").unwrap();
    let path = WorkspaceMapper::new("~/.memfuse/data")
        .map(&ctx, &uri)
        .unwrap();

    assert_eq!(
        path.display().to_string(),
        "~/.memfuse/data/tenants/acme/alice/resources/project-a/docs/api.md"
    );
}

#[test]
fn maps_user_uri_into_user_scoped_workspace_path() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let uri = MfsUri::parse("mfs://user/memories/profile.md").unwrap();
    let path = WorkspaceMapper::new("~/.memfuse/data")
        .map(&ctx, &uri)
        .unwrap();

    assert_eq!(
        path.display().to_string(),
        "~/.memfuse/data/tenants/acme/alice/user/memories/profile.md"
    );
}

#[test]
fn maps_session_archive_uri_into_agent_scoped_workspace_path() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let uri = MfsUri::parse("mfs://session/coding-agent/session-123/history/archive_001").unwrap();
    let path = WorkspaceMapper::new("~/.memfuse/data")
        .map(&ctx, &uri)
        .unwrap();

    assert_eq!(
        path.display().to_string(),
        "~/.memfuse/data/tenants/acme/alice/session/coding-agent/session-123/history/archive_001"
    );
}

#[test]
fn normalizes_short_form_uri_into_root_and_canonical_path() {
    let uri = MfsUri::parse("/user/memories/profile.md").unwrap();

    assert_eq!(uri.root(), "user");
    assert_eq!(uri.canonical_path(), "memories/profile.md");
}

#[test]
fn rejects_root_only_uri() {
    let err = MfsUri::parse("mfs://").unwrap_err();

    assert_eq!(err.to_string(), "URI must include a logical root");
}

#[test]
fn normalizes_repeated_slashes_in_full_uri() {
    let uri = MfsUri::parse("mfs://user//memories///profile.md").unwrap();

    assert_eq!(uri.root(), "user");
    assert_eq!(uri.canonical_path(), "memories/profile.md");
}

#[test]
fn rejects_non_mfs_scheme() {
    let err = MfsUri::parse("http://resources/project-a/docs/api.md").unwrap_err();

    assert_eq!(
        err.to_string(),
        "URI must start with 'mfs://' or a short MFS path: http://resources/project-a/docs/api.md"
    );
}

#[test]
fn rejects_parent_path_traversal_segments() {
    let err = MfsUri::parse("mfs://resources/project-a/../secrets.txt").unwrap_err();

    assert_eq!(err.to_string(), "URI path segment '..' is not allowed");
}

#[test]
fn shortens_overlong_workspace_path_components_deterministically() {
    let ctx = IdentityContext::new("acme", "alice", "coding-agent");
    let long_segment = "verylongsegment".repeat(12);
    let uri = MfsUri::parse(&format!("mfs://resources/{long_segment}/file.md")).unwrap();
    let path = WorkspaceMapper::new("/tmp/workspace")
        .map(&ctx, &uri)
        .unwrap();
    let rendered = path.display().to_string();

    assert!(rendered.ends_with("/file.md"));
    assert!(!rendered.contains(&format!("/{long_segment}/")));
}
