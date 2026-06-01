use mfs_index::{SearchHit, SemanticDocument, SqliteSemanticIndex};

#[test]
fn semantic_index_persists_documents_and_filters_by_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let index = SqliteSemanticIndex::open_at(dir.path().join("semantic.sqlite")).unwrap();

    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/docs/auth.md",
            "resource",
            2,
            "auth.md",
            "OAuth authentication login flow",
            vec![1.0, 0.0, 0.0],
        ))
        .unwrap();
    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/code/auth.rs",
            "resource",
            2,
            "auth.rs",
            "token validator implementation",
            vec![0.0, 1.0, 0.0],
        ))
        .unwrap();

    let hits = index
        .semantic_search(
            &[1.0, 0.0, 0.0],
            "",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            None,
            None,
            10,
        )
        .unwrap();

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].uri, "mfs://resources/docs/auth.md");
}

#[test]
fn semantic_index_retains_lexical_search_and_delete_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let index = SqliteSemanticIndex::open_at(dir.path().join("semantic.sqlite")).unwrap();

    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/docs",
            "resource",
            1,
            ".overview.md",
            "Authentication overview and login best practices",
            vec![0.5, 0.5],
        ))
        .unwrap();

    let lexical_hits = index
        .search_lexical(
            "authentication",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            None,
            None,
            10,
        )
        .unwrap();
    assert_eq!(lexical_hits.len(), 1);
    assert_eq!(lexical_hits[0].uri, "mfs://resources/docs");

    let deleted = index
        .delete_prefix_in_projection(
            Some("tenant:acme:alice:resources"),
            Some("mfs://resources/docs"),
        )
        .unwrap();
    assert_eq!(deleted, 1);

    let remaining = index
        .search_lexical(
            "authentication",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            None,
            None,
            10,
        )
        .unwrap();
    assert!(remaining.is_empty());
}

#[test]
fn semantic_search_sorts_by_cosine_similarity() {
    let dir = tempfile::tempdir().unwrap();
    let index = SqliteSemanticIndex::open_at(dir.path().join("semantic.sqlite")).unwrap();

    for document in [
        doc(
            "tenant:acme:alice:resources",
            "mfs://resources/a",
            "resource",
            2,
            "a",
            "alpha",
            vec![1.0, 0.0],
        ),
        doc(
            "tenant:acme:alice:resources",
            "mfs://resources/b",
            "resource",
            2,
            "b",
            "beta",
            vec![0.5, 0.5],
        ),
    ] {
        index.upsert_document(&document).unwrap();
    }

    let hits = index
        .semantic_search(
            &[1.0, 0.0],
            "",
            Some(&["tenant:acme:alice:resources"]),
            None,
            None,
            None,
            10,
        )
        .unwrap();

    assert_eq!(
        ordered_uris(&hits),
        vec!["mfs://resources/a", "mfs://resources/b"]
    );
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn semantic_index_preserves_multiple_levels_for_same_canonical_uri() {
    let dir = tempfile::tempdir().unwrap();
    let index = SqliteSemanticIndex::open_at(dir.path().join("semantic.sqlite")).unwrap();

    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/docs",
            "resource",
            0,
            ".abstract.md",
            "High level abstract",
            vec![1.0, 0.0],
        ))
        .unwrap();
    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/docs",
            "resource",
            1,
            ".overview.md",
            "Detailed overview",
            vec![0.0, 1.0],
        ))
        .unwrap();

    let abstract_hits = index
        .search_lexical(
            "high",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            Some(&[0]),
            None,
            10,
        )
        .unwrap();
    let overview_hits = index
        .search_lexical(
            "detailed",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            Some(&[1]),
            None,
            10,
        )
        .unwrap();

    assert_eq!(abstract_hits.len(), 1);
    assert_eq!(abstract_hits[0].level, 0);
    assert_eq!(overview_hits.len(), 1);
    assert_eq!(overview_hits[0].level, 1);
}

#[test]
fn semantic_index_scopes_same_uri_by_projection_view() {
    let dir = tempfile::tempdir().unwrap();
    let index = SqliteSemanticIndex::open_at(dir.path().join("semantic.sqlite")).unwrap();

    index
        .upsert_document(&doc(
            "tenant:acme:alice:resources",
            "mfs://resources/docs/auth.md",
            "resource",
            2,
            "auth.md",
            "Alice auth guide",
            vec![1.0, 0.0],
        ))
        .unwrap();
    index
        .upsert_document(&doc(
            "tenant:acme:bob:resources",
            "mfs://resources/docs/auth.md",
            "resource",
            2,
            "auth.md",
            "Bob auth guide",
            vec![0.0, 1.0],
        ))
        .unwrap();

    let alice_hits = index
        .search_lexical(
            "alice",
            Some(&["tenant:acme:alice:resources"]),
            Some("mfs://resources/docs"),
            None,
            None,
            10,
        )
        .unwrap();
    let bob_hits = index
        .search_lexical(
            "bob",
            Some(&["tenant:acme:bob:resources"]),
            Some("mfs://resources/docs"),
            None,
            None,
            10,
        )
        .unwrap();

    assert_eq!(alice_hits.len(), 1);
    assert_eq!(alice_hits[0].uri, "mfs://resources/docs/auth.md");
    assert_eq!(bob_hits.len(), 1);
    assert_eq!(bob_hits[0].uri, "mfs://resources/docs/auth.md");
}

fn ordered_uris(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.uri.as_str()).collect()
}

fn doc(
    projection_view_id: &str,
    uri: &str,
    context_type: &str,
    level: u8,
    title: &str,
    body: &str,
    embedding: Vec<f32>,
) -> SemanticDocument {
    SemanticDocument {
        projection_view_id: projection_view_id.to_owned(),
        uri: uri.to_owned(),
        context_type: context_type.to_owned(),
        resource_id: None,
        content_kind: None,
        language: None,
        level,
        title: title.to_owned(),
        body: body.to_owned(),
        embedding,
    }
}
