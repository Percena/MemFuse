use mfs_workspace::{ResourceCatalog, WorkspaceFs};

#[tokio::test]
async fn registers_localfs_resources_and_reads_from_existing_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("guide.md"),
        "# Guide\nsemantic retrieval\n",
    )
    .unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let resource = catalog
        .register_localfs(
            "acme",
            "alice",
            "coding-agent",
            source.path().to_str().unwrap(),
            Some("docs"),
        )
        .await
        .unwrap();

    assert_eq!(resource.logical_name, "docs");
    assert_eq!(resource.root_uri, "mfs://resources/localfs/docs");

    let fs = WorkspaceFs::open_existing(workspace.path(), "acme", "alice", "coding-agent").unwrap();
    let listing = fs.ls("mfs://resources/localfs/docs").await.unwrap();
    assert!(listing.iter().any(|entry| entry.name == "guide.md"));

    let body = fs
        .read("mfs://resources/localfs/docs/guide.md")
        .await
        .unwrap();
    assert!(body.contains("semantic retrieval"));
}

#[tokio::test]
async fn auto_generated_logical_names_are_unique_per_tenant() {
    let workspace = tempfile::tempdir().unwrap();
    let source_one = tempfile::tempdir().unwrap();
    let source_two = tempfile::tempdir().unwrap();
    std::fs::write(source_one.path().join("one.md"), "# One\n").unwrap();
    std::fs::write(source_two.path().join("two.md"), "# Two\n").unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let first = catalog
        .register_localfs(
            "acme",
            "alice",
            "coding-agent",
            source_one.path().to_str().unwrap(),
            None,
        )
        .await
        .unwrap();
    let second = catalog
        .register_localfs(
            "acme",
            "alice",
            "coding-agent",
            source_two.path().to_str().unwrap(),
            None,
        )
        .await
        .unwrap();

    assert_ne!(first.resource_id, second.resource_id);
    assert_ne!(first.logical_name, second.logical_name);
    assert!(second.root_uri.starts_with("mfs://resources/localfs/"));
}

#[tokio::test]
async fn registers_git_resources_under_source_aware_uri() {
    let workspace = tempfile::tempdir().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();

    let repo = git2::Repository::init(repo_dir.path()).unwrap();
    repo.remote("origin", "https://github.com/example-org/example-repo.git")
        .unwrap();
    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    std::fs::create_dir_all(repo_dir.path().join("docs")).unwrap();
    std::fs::write(repo_dir.path().join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    std::fs::write(repo_dir.path().join("docs/guide.md"), "# Guide\n").unwrap();
    let signature = git2::Signature::now("Tester", "tester@example.com").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
        .unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let resource = catalog
        .register_git(
            "acme",
            "alice",
            "coding-agent",
            repo_dir.path().to_str().unwrap(),
            None,
        )
        .await
        .unwrap();

    // Namespace is now flattened: "example-org" stays as single segment
    // URI depth fixed at 4: mfs://resources/git/{host}/{flat_ns}/{repo}
    assert_eq!(
        resource.root_uri,
        "mfs://resources/git/github.com/example-org/example-repo"
    );
    assert_eq!(resource.resource_kind, "code_repo");
    assert_eq!(resource.source_host.as_deref(), Some("github.com"));
    assert_eq!(resource.source_namespace.as_deref(), Some("example-org"));
    assert_eq!(resource.source_repo.as_deref(), Some("example-repo"));
    // source_ref should now be populated with "branch:commit_short"
    assert!(resource.source_ref.is_some());
    let sr = resource.source_ref.unwrap();
    assert!(
        sr.contains(':'),
        "source_ref should be 'branch:commit_short'"
    );
}

#[tokio::test]
async fn registers_local_only_git_resources_under_stable_local_namespace() {
    let workspace = tempfile::tempdir().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();

    let repo = git2::Repository::init(repo_dir.path()).unwrap();
    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    std::fs::write(
        repo_dir.path().join("src/lib.rs"),
        "pub fn local_only() {}\n",
    )
    .unwrap();
    let signature = git2::Signature::now("Tester", "tester@example.com").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
        .unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let resource = catalog
        .register_git(
            "acme",
            "alice",
            "coding-agent",
            repo_dir.path().to_str().unwrap(),
            None,
        )
        .await
        .unwrap();

    // No-origin git repos now use stable "local" namespace
    // URI: mfs://resources/git/local/local/{repo_name}
    // (no canonicalize(path) hash — stable even if directory is moved)
    assert!(
        resource
            .root_uri
            .starts_with("mfs://resources/git/local/local/")
    );
    assert!(!resource.root_uri.ends_with('/'));
    assert_eq!(resource.resource_kind, "code_repo");
    assert_eq!(resource.source_host.as_deref(), Some("local"));
    assert_eq!(resource.source_namespace.as_deref(), Some("local"));
    // source_ref should be populated for local git repos too
    assert!(resource.source_ref.is_some());
}

#[tokio::test]
async fn same_origin_repo_family_detection_refreshes_existing() {
    // Register the same GitHub repo twice — family detection should refresh
    // the existing resource instead of creating a duplicate.
    let workspace = tempfile::tempdir().unwrap();

    // First copy
    let copy_a = tempfile::tempdir().unwrap();
    let repo_a = git2::Repository::init(copy_a.path()).unwrap();
    repo_a
        .remote("origin", "https://github.com/example-org/example-repo.git")
        .unwrap();
    std::fs::create_dir_all(copy_a.path().join("src")).unwrap();
    std::fs::write(copy_a.path().join("src/lib.rs"), "pub fn v1() {}\n").unwrap();
    let sig = git2::Signature::now("Tester", "tester@example.com").unwrap();
    let mut idx = repo_a.index().unwrap();
    idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tid = idx.write_tree().unwrap();
    let tree = repo_a.find_tree(tid).unwrap();
    repo_a
        .commit(Some("HEAD"), &sig, &sig, "v1", &tree, &[])
        .unwrap();

    // Second copy (same origin, different content)
    let copy_b = tempfile::tempdir().unwrap();
    let repo_b = git2::Repository::init(copy_b.path()).unwrap();
    repo_b
        .remote("origin", "https://github.com/example-org/example-repo.git")
        .unwrap();
    std::fs::create_dir_all(copy_b.path().join("src")).unwrap();
    std::fs::write(copy_b.path().join("src/lib.rs"), "pub fn v2() {}\n").unwrap();
    let mut idx_b = repo_b.index().unwrap();
    idx_b
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tid_b = idx_b.write_tree().unwrap();
    let tree_b = repo_b.find_tree(tid_b).unwrap();
    repo_b
        .commit(Some("HEAD"), &sig, &sig, "v2", &tree_b, &[])
        .unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();

    let first = catalog
        .register_git(
            "acme",
            "alice",
            "coding-agent",
            copy_a.path().to_str().unwrap(),
            Some("my-project"),
        )
        .await
        .unwrap();

    let second = catalog
        .register_git(
            "acme",
            "alice",
            "coding-agent",
            copy_b.path().to_str().unwrap(),
            Some("my-project-v2"),
        )
        .await
        .unwrap();

    // Both should have the same canonical URI (family detection)
    assert_eq!(first.root_uri, second.root_uri);
    // Same resource_id (family key hash, not timestamp)
    assert_eq!(first.resource_id, second.resource_id);
    // Same logical_name (refreshed, not re-registered)
    assert_eq!(first.logical_name, second.logical_name);
    // source_ref should reflect the latest registration
    assert!(second.source_ref.is_some());
}

#[tokio::test]
async fn gitlab_subgroup_namespace_is_flattened() {
    // GitLab subgroup URLs should produce a flattened namespace in the URI,
    // keeping depth at exactly 4 layers: git/host/namespace/repo
    let workspace = tempfile::tempdir().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();

    let repo = git2::Repository::init(repo_dir.path()).unwrap();
    repo.remote(
        "origin",
        "https://gitlab.com/org/sub-group/deeper-group/repo.git",
    )
    .unwrap();
    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    std::fs::write(repo_dir.path().join("src/lib.rs"), "pub fn demo() {}\n").unwrap();
    let sig = git2::Signature::now("Tester", "tester@example.com").unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    let tid = idx.write_tree().unwrap();
    let tree = repo.find_tree(tid).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();

    let catalog = ResourceCatalog::open(workspace.path()).unwrap();
    let resource = catalog
        .register_git(
            "acme",
            "alice",
            "coding-agent",
            repo_dir.path().to_str().unwrap(),
            Some("deep-repo"),
        )
        .await
        .unwrap();

    // Namespace "org/sub-group/deeper-group" is flattened to "org-sub-group-deeper-group"
    // URI: mfs://resources/git/gitlab.com/org-sub-group-deeper-group/repo (4 layers)
    assert_eq!(
        resource.root_uri,
        "mfs://resources/git/gitlab.com/org-sub-group-deeper-group/repo"
    );
    assert_eq!(resource.source_host.as_deref(), Some("gitlab.com"));
    assert_eq!(
        resource.source_namespace.as_deref(),
        Some("org-sub-group-deeper-group")
    );
    assert_eq!(resource.source_repo.as_deref(), Some("repo"));
}
