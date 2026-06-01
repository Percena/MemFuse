use mfs_metadata::MetadataStore;
use mfs_server::{
    complete_prepared_resource_ingest, prepare_resource_ingest, rebuild_projection,
    refresh_projection, refresh_registered_resource,
};
use mfs_types::IdentityContext;
use mfs_workspace::{Materializer, WorkspaceFs};

#[tokio::test]
async fn rebuild_restores_metadata_and_search_index_from_workspace() {
    let fs = WorkspaceFs::from_fixture(
        "acme",
        "alice",
        "coding-agent",
        "tests/fixtures/localfs_docs",
    )
    .await
    .unwrap();
    let metadata =
        MetadataStore::open_at(fs.workspace_root().join("_system/metadata.sqlite"), false).unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");

    let engine = rebuild_projection(
        &metadata,
        &identity,
        fs.projection_root(),
        fs.projection_uri(),
    )
    .await
    .unwrap();

    let result = engine
        .find("authentication", Some(fs.projection_uri()))
        .await
        .unwrap();
    assert!(!result.resources.is_empty());
    assert!(metadata.count_path_entries().unwrap() > 0);
    assert!(
        metadata
            .get_path_entry(
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs/api.md"
            )
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn refresh_replaces_stale_projection_files_and_updates_metadata() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("current.md"), "# Current\n").unwrap();

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let initial = Materializer::new(workspace.path())
        .materialize_localfs(
            &identity,
            source.path().to_str().unwrap(),
            "mfs://resources/localfs/docs",
        )
        .await
        .unwrap();
    let metadata_path = workspace.path().join("_system/metadata.sqlite");
    let metadata = MetadataStore::open_at(&metadata_path, false).unwrap();
    let _ = rebuild_projection(
        &metadata,
        &identity,
        initial.target_path.as_path(),
        initial.provenance.target_uri.as_str(),
    )
    .await
    .unwrap();

    std::fs::remove_file(source.path().join("current.md")).unwrap();
    std::fs::write(source.path().join("next.md"), "# Next\n").unwrap();

    let result = refresh_projection(
        &metadata,
        workspace.path(),
        &identity,
        "localfs",
        source.path().to_str().unwrap(),
        "mfs://resources/localfs/docs",
    )
    .await
    .unwrap();

    assert!(!result.projection_root.join("current.md").exists());
    assert!(result.projection_root.join("next.md").exists());
    assert!(
        metadata
            .get_path_entry(
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs/current.md"
            )
            .unwrap()
            .is_none()
    );
    assert!(
        metadata
            .get_path_entry(
                "tenant:acme:alice:resources",
                "mfs://resources/localfs/docs/next.md"
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(
        metadata
            .list_snapshots("acme", "alice", Some("tenant:acme:alice:resources"), 10)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn prepared_ingest_persists_repo_classification_and_code_symbols() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("src")).unwrap();
    std::fs::create_dir_all(source.path().join("docs")).unwrap();
    std::fs::write(source.path().join("README.md"), "# Repo\n").unwrap();
    std::fs::write(source.path().join("docs/guide.md"), "# Guide\n").unwrap();
    std::fs::write(
        source.path().join("src/lib.rs"),
        "pub fn greet(name: &str) {}\n",
    )
    .unwrap();

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let managed = prepare_resource_ingest(
        &metadata,
        workspace.path(),
        &identity,
        "localfs",
        source.path().to_str().unwrap(),
        Some("sample-repo"),
        None,
        None,
    )
    .await
    .unwrap();
    complete_prepared_resource_ingest(&metadata, workspace.path(), &identity, &managed)
        .await
        .unwrap();

    let source_record = metadata
        .get_resource_source(&managed.resource_id)
        .unwrap()
        .unwrap();
    assert_eq!(source_record.resource_kind, "mixed_repo");
    assert_eq!(
        source_record.canonical_root_uri,
        "mfs://resources/localfs/sample-repo"
    );

    let readme = metadata
        .get_path_entry(
            "tenant:acme:alice:resources",
            "mfs://resources/localfs/sample-repo/README.md",
        )
        .unwrap()
        .unwrap();
    assert_eq!(readme.content_kind.as_deref(), Some("repo_doc"));

    let code = metadata
        .get_path_entry(
            "tenant:acme:alice:resources",
            "mfs://resources/localfs/sample-repo/src/lib.rs",
        )
        .unwrap()
        .unwrap();
    assert_eq!(code.content_kind.as_deref(), Some("code"));
    assert_eq!(code.language.as_deref(), Some("rust"));
    assert!(code.content_digest.is_some());

    let code_symbols = metadata
        .get_code_symbols(
            "tenant:acme:alice:resources",
            Some("mfs://resources/localfs/sample-repo/src/lib.rs"),
        )
        .unwrap();
    assert!(!code_symbols.is_empty());
    assert!(code_symbols.iter().any(|item| item.symbol_name == "greet"));
}

#[tokio::test]
async fn registered_refresh_records_resource_change_events() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("current.md"), "# Current\n").unwrap();

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let managed = prepare_resource_ingest(
        &metadata,
        workspace.path(),
        &identity,
        "localfs",
        source.path().to_str().unwrap(),
        Some("watch-repo"),
        None,
        None,
    )
    .await
    .unwrap();
    complete_prepared_resource_ingest(&metadata, workspace.path(), &identity, &managed)
        .await
        .unwrap();

    std::fs::remove_file(source.path().join("current.md")).unwrap();
    std::fs::write(source.path().join("next.md"), "# Next\n").unwrap();

    let result =
        refresh_registered_resource(&metadata, workspace.path(), &identity, &managed.resource_id)
            .await
            .unwrap();
    assert_eq!(result.resource_id, managed.resource_id);

    let events = metadata
        .list_change_events_by_resource(&managed.resource_id, 10)
        .unwrap();
    assert!(events.iter().any(|item| item.change_type == "deleted"));
    assert!(events.iter().any(|item| item.change_type == "added"));
}

#[tokio::test]
async fn prepared_url_ingest_preserves_original_url_identity() {
    let workspace = tempfile::tempdir().unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let (url, handle) =
        spawn_static_http_server("/docs/guide.md", "# Guide\nbody\n", "text/markdown");

    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let managed = prepare_resource_ingest(
        &metadata,
        workspace.path(),
        &identity,
        "url",
        &url,
        Some("remote-guide"),
        None,
        None,
    )
    .await
    .unwrap();
    handle.join().unwrap();

    assert_eq!(managed.source_kind, "url");
    assert_eq!(managed.source_identifier, url);
    // The host segment includes the dynamic port (slugified) since the mock
    // server binds to a random port.  Verify the root_uri starts with the
    // canonical prefix and ends with the repo slug.
    assert!(managed.root_uri.starts_with("mfs://resources/url/"));
    assert!(managed.root_uri.ends_with("/docs-guide-md"));
    assert!(managed.source_host.is_some());
    assert!(managed.source_repo.as_deref() == Some("docs-guide-md"));
}

#[tokio::test]
async fn refresh_handles_file_directory_type_flips() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("shape"), "file-v1\n").unwrap();

    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let managed = prepare_resource_ingest(
        &metadata,
        workspace.path(),
        &identity,
        "localfs",
        source.path().to_str().unwrap(),
        Some("shape-repo"),
        None,
        None,
    )
    .await
    .unwrap();
    complete_prepared_resource_ingest(&metadata, workspace.path(), &identity, &managed)
        .await
        .unwrap();

    std::fs::remove_file(source.path().join("shape")).unwrap();
    std::fs::create_dir_all(source.path().join("shape")).unwrap();
    std::fs::write(source.path().join("shape/nested.md"), "# nested\n").unwrap();

    refresh_registered_resource(&metadata, workspace.path(), &identity, &managed.resource_id)
        .await
        .unwrap();

    let root = workspace
        .path()
        .join("tenants/acme/alice/resources/localfs/shape-repo/shape");
    assert!(root.is_dir());
    assert!(root.join("nested.md").exists());
}

#[tokio::test]
async fn repeated_refresh_with_same_changed_uri_remains_idempotent() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("doc.md"), "# v1\n").unwrap();
    let identity = IdentityContext::new("acme", "alice", "coding-agent");
    let metadata =
        MetadataStore::open_at(workspace.path().join("_system/metadata.sqlite"), false).unwrap();
    let managed = prepare_resource_ingest(
        &metadata,
        workspace.path(),
        &identity,
        "localfs",
        source.path().to_str().unwrap(),
        Some("repeat-repo"),
        None,
        None,
    )
    .await
    .unwrap();
    complete_prepared_resource_ingest(&metadata, workspace.path(), &identity, &managed)
        .await
        .unwrap();

    for version in ["# v2\n", "# v3\n"] {
        std::fs::write(source.path().join("doc.md"), version).unwrap();
        refresh_registered_resource(&metadata, workspace.path(), &identity, &managed.resource_id)
            .await
            .unwrap();
    }

    let events = metadata
        .list_change_events_by_resource(&managed.resource_id, 10)
        .unwrap();
    assert!(
        events
            .iter()
            .filter(|item| item.change_type == "modified")
            .count()
            >= 2
    );
}

fn spawn_static_http_server(
    path: &str,
    body: &str,
    content_type: &str,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let path = path.to_owned();
    let body = body.to_owned();
    let content_type = content_type.to_owned();
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}{path}"), handle)
}
