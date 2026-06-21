use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_memory::{
    ChunkingMemoryDocumentIndexer, DocumentMetadata, FilesystemMemoryDocumentRepository,
    InMemoryMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendWriteOptions, MemoryContext, MemoryDocumentPath, MemoryDocumentRepository,
    MemoryDocumentScope, MemorySearchRequest, RepositoryMemoryBackend,
    redact_sensitive_memory_content, stable_learning_document_relative_path,
};

fn scope() -> MemoryDocumentScope {
    MemoryDocumentScope::new_with_agent("tenant-a", "alice", Some("agent-a"), None)
        .expect("valid memory scope")
}

fn context() -> MemoryContext {
    MemoryContext::new(scope())
}

fn doc_path(relative_path: &str) -> MemoryDocumentPath {
    let scope = scope();
    MemoryDocumentPath::new_with_agent(
        scope.tenant_id(),
        scope.user_id(),
        scope.agent_id(),
        scope.project_id(),
        relative_path,
    )
    .expect("valid memory path")
}

fn searchable_capabilities() -> MemoryBackendCapabilities {
    MemoryBackendCapabilities {
        file_documents: true,
        metadata: true,
        versioning: true,
        full_text_search: true,
        ..MemoryBackendCapabilities::default()
    }
}

fn in_memory_backend() -> (
    Arc<InMemoryMemoryDocumentRepository>,
    RepositoryMemoryBackend<InMemoryMemoryDocumentRepository>,
) {
    let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
    let backend = RepositoryMemoryBackend::new(Arc::clone(&repo))
        .without_prompt_write_safety_policy()
        .with_capabilities(searchable_capabilities());
    (repo, backend)
}

fn filesystem_backend() -> (
    Arc<FilesystemMemoryDocumentRepository<InMemoryBackend>>,
    RepositoryMemoryBackend<FilesystemMemoryDocumentRepository<InMemoryBackend>>,
) {
    let repo = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::new(
        InMemoryBackend::new(),
    )));
    let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(Arc::clone(&repo)));
    let backend = RepositoryMemoryBackend::new(Arc::clone(&repo))
        .without_prompt_write_safety_policy()
        .with_indexer(indexer)
        .with_capabilities(searchable_capabilities());
    (repo, backend)
}

fn learning_metadata(
    key: &str,
    category: &str,
    confidence: u8,
    created_at: String,
) -> DocumentMetadata {
    DocumentMetadata {
        confidence: Some(confidence),
        created_at: Some(created_at),
        category: Some(category.to_string()),
        key: Some(key.to_string()),
        source: Some("test".to_string()),
        ..DocumentMetadata::default()
    }
}

fn learning_path(category: &str, key: &str) -> String {
    stable_learning_document_relative_path(category, key).expect("valid stable learning path")
}

async fn write_learning<R>(
    backend: &RepositoryMemoryBackend<R>,
    relative_path: &str,
    content: &str,
    metadata: DocumentMetadata,
) where
    R: MemoryDocumentRepository + 'static,
{
    backend
        .write_document_with_backend_options(
            &context(),
            &doc_path(relative_path),
            content.as_bytes(),
            &MemoryBackendWriteOptions {
                metadata_overlay: Some(metadata),
            },
        )
        .await
        .expect("write learning");
}

async fn search<R>(
    backend: &RepositoryMemoryBackend<R>,
    query: &str,
) -> Vec<ironclaw_memory::MemorySearchResult>
where
    R: MemoryDocumentRepository + 'static,
{
    backend
        .search(
            &context(),
            MemorySearchRequest::new(query)
                .expect("valid query")
                .with_vector(false)
                .with_limit(10),
        )
        .await
        .expect("search succeeds")
}

async fn assert_overwrite_no_ghost<R>(backend: RepositoryMemoryBackend<R>)
where
    R: MemoryDocumentRepository + 'static,
{
    let now = Utc::now().to_rfc3339();
    let editor_preference_path = learning_path("preference", "editor");
    let theme_preference_path = learning_path("preference", "theme");
    let editor_fact_path = learning_path("fact", "editor");
    write_learning(
        &backend,
        &editor_preference_path,
        "editor preference old_unique_marker use nano",
        learning_metadata("editor", "preference", 8, now.clone()),
    )
    .await;
    write_learning(
        &backend,
        &editor_preference_path,
        "editor preference new_unique_marker use helix",
        learning_metadata("editor", "preference", 9, now.clone()),
    )
    .await;
    write_learning(
        &backend,
        &theme_preference_path,
        "theme preference other_key_marker use dark mode",
        learning_metadata("theme", "preference", 7, now.clone()),
    )
    .await;
    write_learning(
        &backend,
        &editor_fact_path,
        "editor fact category_marker is a binary name",
        learning_metadata("editor", "fact", 7, now),
    )
    .await;

    assert!(
        search(&backend, "old_unique_marker").await.is_empty(),
        "overwriting the same keyed document must remove the old value from search"
    );

    let new_hits = search(&backend, "new_unique_marker").await;
    assert_eq!(new_hits.len(), 1);
    assert_eq!(new_hits[0].path.relative_path(), editor_preference_path);

    let other_key_hits = search(&backend, "other_key_marker").await;
    assert_eq!(other_key_hits.len(), 1);
    assert_eq!(
        other_key_hits[0].path.relative_path(),
        theme_preference_path
    );

    let category_hits = search(&backend, "category_marker").await;
    assert_eq!(category_hits.len(), 1);
    assert_eq!(category_hits[0].path.relative_path(), editor_fact_path);
}

#[tokio::test]
async fn in_memory_overwrite_same_learning_path_has_no_search_ghost() {
    let (_repo, backend) = in_memory_backend();
    assert_overwrite_no_ghost(backend).await;
}

#[tokio::test]
async fn filesystem_overwrite_same_learning_path_has_no_search_ghost() {
    let (_repo, backend) = filesystem_backend();
    assert_overwrite_no_ghost(backend).await;
}

async fn assert_decay_at_read_ranks_flags_and_does_not_mutate<R>(
    repo: Arc<R>,
    backend: RepositoryMemoryBackend<R>,
) where
    R: MemoryDocumentRepository + 'static,
{
    let fresh_path = learning_path("preference", "fresh");
    let stale_path = learning_path("preference", "stale");
    let now = Utc::now();
    write_learning(
        &backend,
        &stale_path,
        "decay ranking marker stale answer",
        learning_metadata(
            "stale",
            "preference",
            1,
            (now - Duration::days(400)).to_rfc3339(),
        ),
    )
    .await;
    write_learning(
        &backend,
        &fresh_path,
        "decay ranking marker fresh answer",
        learning_metadata("fresh", "preference", 10, now.to_rfc3339()),
    )
    .await;

    let stale_doc = doc_path(&stale_path);
    let before_bytes = repo
        .read_document(&stale_doc)
        .await
        .expect("read stale document before search");
    let before_metadata = repo
        .read_document_metadata(&stale_doc)
        .await
        .expect("read stale metadata before search");

    let results = search(&backend, "decay ranking marker").await;
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].path.relative_path(),
        fresh_path,
        "fresh high-confidence learning should outrank stale low-confidence learning"
    );

    let stale = results
        .iter()
        .find(|result| result.path.relative_path() == stale_path)
        .expect("stale learning remains findable");
    let learning = stale.learning.as_ref().expect("learning metadata surfaced");
    assert_eq!(learning.metadata.confidence, Some(1));
    assert!(
        learning.is_stale,
        "aged low-confidence learning must be flagged"
    );

    let after_bytes = repo
        .read_document(&stale_doc)
        .await
        .expect("read stale document after search");
    let after_metadata = repo
        .read_document_metadata(&stale_doc)
        .await
        .expect("read stale metadata after search");
    assert_eq!(after_bytes, before_bytes, "search must not mutate content");
    assert_eq!(
        after_metadata, before_metadata,
        "decay is read-time ranking only; search must not mutate metadata"
    );
}

async fn assert_non_learning_metadata_does_not_decay_search_results<R>(
    backend: RepositoryMemoryBackend<R>,
) where
    R: MemoryDocumentRepository + 'static,
{
    let now = Utc::now();
    write_learning(
        &backend,
        "notes/a-ordinary.md",
        "ordinary ranking marker stale-looking metadata",
        DocumentMetadata {
            confidence: Some(1),
            created_at: Some((now - Duration::days(400)).to_rfc3339()),
            ..DocumentMetadata::default()
        },
    )
    .await;
    write_learning(
        &backend,
        "notes/z-ordinary.md",
        "ordinary ranking marker fresh-looking metadata",
        DocumentMetadata {
            confidence: Some(10),
            created_at: Some(now.to_rfc3339()),
            ..DocumentMetadata::default()
        },
    )
    .await;

    let results = search(&backend, "ordinary ranking marker").await;
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].path.relative_path(),
        "notes/a-ordinary.md",
        "ordinary non-keyed metadata must not trigger learning decay re-ranking"
    );
    assert!(
        results.iter().all(|result| result.learning.is_none()),
        "ordinary non-keyed metadata must not surface learning signals"
    );
}

#[tokio::test]
async fn in_memory_decay_at_read_ranks_flags_and_does_not_mutate() {
    let (repo, backend) = in_memory_backend();
    assert_decay_at_read_ranks_flags_and_does_not_mutate(repo, backend).await;
}

#[tokio::test]
async fn filesystem_decay_at_read_ranks_flags_and_does_not_mutate() {
    let (repo, backend) = filesystem_backend();
    assert_decay_at_read_ranks_flags_and_does_not_mutate(repo, backend).await;
}

#[tokio::test]
async fn in_memory_non_learning_metadata_does_not_decay_search_results() {
    let (_repo, backend) = in_memory_backend();
    assert_non_learning_metadata_does_not_decay_search_results(backend).await;
}

#[tokio::test]
async fn filesystem_non_learning_metadata_does_not_decay_search_results() {
    let (_repo, backend) = filesystem_backend();
    assert_non_learning_metadata_does_not_decay_search_results(backend).await;
}

#[test]
fn redacts_secret_patterns_preserving_non_secret_content() {
    let input = concat!(
        "database_url=postgres://app:swordfish@db.example/app\n",
        "password: hunter2\n",
        "OPENAI_API_KEY=sk-proj-test1234567890abcdefghij\n",
        "region=us-east-1\n"
    );

    let redacted = redact_sensitive_memory_content(input);

    assert!(!redacted.contains("swordfish"));
    assert!(!redacted.contains("hunter2"));
    assert!(!redacted.contains("sk-proj-test"));
    assert!(redacted.contains("postgres://app:[REDACTED - sensitive]@db.example/app"));
    assert!(redacted.contains("password: [REDACTED - sensitive]"));
    assert!(redacted.contains("OPENAI_API_KEY=[REDACTED - sensitive]"));
    assert!(redacted.contains("region=us-east-1"));
}
