//! Vertical integration through public seams (#3118 phase 7).
//!
//! Composes:
//!
//!   reborn-native repository
//!     -> ChunkingMemoryDocumentIndexer
//!     -> RepositoryMemoryBackend
//!     -> MemoryBackendFilesystemAdapter
//!     -> CompositeRootFilesystem mounted at /memory
//!
//! The contract this file locks in is the issue's Phase 7 list:
//! - Authorized memory read/write/list/search succeeds.
//! - Denied / unsupported memory operations fail closed before
//!   reaching the repository (no DB side effects).
//! - The scoped virtual path cannot escape its
//!   tenant/user/agent/project — search returns only same-scope results.
//! - Errors do not expose raw DB / provider internals to the caller.

#![cfg(any(feature = "libsql", feature = "postgres"))]

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    FilesystemError, IndexPolicy, MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::{
    EmbeddingError, EmbeddingProvider, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentPath, MemoryDocumentRepository,
    MemoryDocumentScope, MemorySearchRequest, RepositoryMemoryBackend,
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_memory::{ChunkConfig, ChunkingMemoryDocumentIndexer};

#[cfg(feature = "libsql")]
use ironclaw_memory::RebornLibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
use ironclaw_memory::RebornPostgresMemoryDocumentRepository;

// --- shared scaffolding ---------------------------------------------------

fn memory_mount_descriptor() -> MountDescriptor {
    MountDescriptor {
        virtual_root: VirtualPath::new("/memory").unwrap(),
        backend_id: BackendId::new("reborn-memory".to_string()).unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::FileContent,
        content_kind: ContentKind::MemoryDocument,
        index_policy: IndexPolicy::FullTextAndVector,
        capabilities: BackendCapabilities {
            read: true,
            write: true,
            list: true,
            stat: true,
            indexed: true,
            embedded: true,
            ..BackendCapabilities::default()
        },
    }
}

fn vpath(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!("/memory{suffix}")).unwrap()
}

#[derive(Default)]
struct DeterministicProvider;

#[async_trait]
impl EmbeddingProvider for DeterministicProvider {
    fn dimension(&self) -> usize {
        3
    }

    fn model_name(&self) -> &str {
        "deterministic-test-embedding"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        if text.contains("scope-token") || text.contains("hybrid-vector") {
            Ok(vec![1.0, 0.0, 0.0])
        } else if text.contains("unrelated") {
            Ok(vec![0.0, 1.0, 0.0])
        } else {
            Ok(vec![0.0, 0.0, 1.0])
        }
    }
}

// --- libSQL ---------------------------------------------------------------

#[cfg(feature = "libsql")]
async fn libsql_compose() -> (
    CompositeRootFilesystem,
    Arc<RebornLibSqlMemoryDocumentRepository>,
    Arc<RepositoryMemoryBackend<RebornLibSqlMemoryDocumentRepository>>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn_memory.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("libsql build"),
    );
    let repository = Arc::new(RebornLibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.expect("run_migrations");
    let provider = Arc::new(DeterministicProvider);
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repository.clone())
            .with_chunk_config(ChunkConfig {
                chunk_size: 4,
                overlap_percent: 0.0,
                min_chunk_size: 1,
            })
            .with_embedding_provider(provider.clone()),
    );
    let backend = Arc::new(
        RepositoryMemoryBackend::new(repository.clone())
            .with_indexer(indexer)
            .with_embedding_provider(provider)
            .with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                full_text_search: true,
                vector_search: true,
                embeddings: true,
                ..MemoryBackendCapabilities::default()
            }),
    );
    let adapter = Arc::new(MemoryBackendFilesystemAdapter::new(backend.clone()));
    let mut composite = CompositeRootFilesystem::new();
    composite
        .mount(memory_mount_descriptor(), adapter)
        .expect("mount /memory");
    (composite, repository, backend, dir)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_round_trips_authorized_read_write_through_composite_mount() {
    let (composite, _repo, _backend, _dir) = libsql_compose().await;
    let path = vpath("/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/welcome.md");

    composite
        .write_file(&path, b"first welcome note")
        .await
        .expect("write must succeed for authorized scope");
    let stored = composite.read_file(&path).await.expect("read");
    assert_eq!(stored, b"first welcome note");

    let stat = composite.stat(&path).await.expect("stat");
    assert_eq!(stat.len, "first welcome note".len() as u64);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_lists_direct_children_through_composite_mount() {
    let (composite, _repo, _backend, _dir) = libsql_compose().await;
    composite
        .write_file(
            &vpath("/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/a.md"),
            b"a",
        )
        .await
        .unwrap();
    composite
        .write_file(
            &vpath("/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes/sub/b.md"),
            b"b",
        )
        .await
        .unwrap();

    let entries = composite
        .list_dir(&vpath(
            "/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    let mut sorted = entries.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a.md".to_string(), "sub".to_string()]);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_search_through_composite_mount_returns_only_same_scope_results() {
    let (composite, _repo, backend, _dir) = libsql_compose().await;

    for (path, body) in [
        (
            "/tenants/tenant-a/users/alice/agents/_none/projects/proj-1/visible.md",
            "scope-token visible body",
        ),
        (
            "/tenants/tenant-a/users/alice/agents/_none/projects/proj-2/hidden-project.md",
            "scope-token hidden project",
        ),
        (
            "/tenants/tenant-a/users/bob/agents/_none/projects/proj-1/hidden-user.md",
            "scope-token hidden user",
        ),
        (
            "/tenants/tenant-b/users/alice/agents/_none/projects/proj-1/hidden-tenant.md",
            "scope-token hidden tenant",
        ),
    ] {
        composite
            .write_file(&vpath(path), body.as_bytes())
            .await
            .unwrap();
    }

    let scope = MemoryDocumentScope::new("tenant-a", "alice", Some("proj-1")).unwrap();
    let context = MemoryContext::new(scope);
    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("scope-token")
                .unwrap()
                .with_vector(false)
                .with_limit(10),
        )
        .await
        .unwrap();

    let result_paths = results
        .iter()
        .map(|result| result.path.relative_path().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        result_paths,
        vec!["visible.md".to_string()],
        "composite mount must not leak rows from other tenants/users/projects"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_capability_denied_search_fails_closed_before_repository_is_called() {
    let spy = Arc::new(SearchSpyRepository::default());
    let backend =
        RepositoryMemoryBackend::new(spy.clone()).with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: false,
            vector_search: false,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    let err = backend
        .search(&context, MemorySearchRequest::new("needle").unwrap())
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("memory backend does not support search")
    );
    assert_eq!(
        spy.search_calls(),
        0,
        "capability fail-closed must short-circuit before the repository"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_composite_mount_rejects_invalid_memory_path_with_clean_error() {
    let (composite, _repo, _backend, _dir) = libsql_compose().await;

    // No relative file path after the project id => not a memory document path.
    let bad = vpath("/tenants/tenant-a/users/alice/agents/_none/projects/_none");
    let err = composite.read_file(&bad).await.unwrap_err();
    let msg = err.to_string().to_lowercase();
    // Must not surface raw SQL/provider internals.
    assert!(
        !msg.contains("select")
            && !msg.contains("sql")
            && !msg.contains("postgres")
            && !msg.contains("libsql"),
        "error must not expose DB internals: {err}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_composite_mount_rejects_duplicate_root_at_registration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn_memory.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("libsql build"),
    );
    let repository = Arc::new(RebornLibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let backend = Arc::new(RepositoryMemoryBackend::new(repository));
    let adapter = Arc::new(MemoryBackendFilesystemAdapter::new(backend));
    let mut composite = CompositeRootFilesystem::new();
    composite
        .mount(memory_mount_descriptor(), adapter.clone())
        .expect("first /memory mount");
    let err = composite
        .mount(memory_mount_descriptor(), adapter)
        .expect_err("second mount at same root must conflict");
    assert!(matches!(err, FilesystemError::MountConflict { .. }));
}

// --- Postgres -------------------------------------------------------------

#[cfg(feature = "postgres")]
fn pg_pool() -> deadpool_postgres::Pool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("DATABASE_URL must be a valid Postgres URL");
    let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(mgr)
        .max_size(4)
        .build()
        .expect("build deadpool")
}

/// Explicit opt-in to skip the Postgres vertical tests. Without this set,
/// a connection failure must fail loud — the previous "silent skip + green
/// pass" pattern violated the `ironclaw_memory` guardrail that Postgres
/// behavioral coverage must be real.
#[cfg(feature = "postgres")]
const POSTGRES_SKIP_ENV: &str = "IRONCLAW_SKIP_POSTGRES_TESTS";

#[cfg(feature = "postgres")]
fn pg_skip_requested() -> bool {
    std::env::var(POSTGRES_SKIP_ENV).is_ok_and(|value| value == "1" || value == "true")
}

#[cfg(feature = "postgres")]
async fn pg_require_connection(pool: &deadpool_postgres::Pool) -> Option<()> {
    match pool.get().await {
        Ok(_) => Some(()),
        Err(error) => {
            if pg_skip_requested() {
                eprintln!(
                    "skipping reborn-postgres vertical test ({POSTGRES_SKIP_ENV}=1): {error}"
                );
                None
            } else {
                panic!(
                    "reborn-postgres vertical test could not reach Postgres ({error}); \
                     set DATABASE_URL to a reachable Postgres+pgvector instance, or set \
                     {POSTGRES_SKIP_ENV}=1 to explicitly skip."
                );
            }
        }
    }
}

#[cfg(feature = "postgres")]
async fn pg_cleanup_tenant(pool: &deadpool_postgres::Pool, tenant_id: &str) {
    let Ok(client) = pool.get().await else { return };
    let _ = client
        .execute(
            "DELETE FROM reborn_memory_documents WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await;
}

#[cfg(feature = "postgres")]
async fn pg_compose(
    tenant_id: &str,
) -> Option<(
    CompositeRootFilesystem,
    Arc<RebornPostgresMemoryDocumentRepository>,
    Arc<RepositoryMemoryBackend<RebornPostgresMemoryDocumentRepository>>,
)> {
    let pool = pg_pool();
    pg_require_connection(&pool).await?;
    let repository = Arc::new(RebornPostgresMemoryDocumentRepository::new(pool.clone()));
    repository.run_migrations().await.expect("run_migrations");
    pg_cleanup_tenant(&pool, tenant_id).await;
    // Wire the same chunking indexer the libSQL vertical stack uses so writes
    // through `CompositeRootFilesystem` populate `reborn_memory_chunks`.
    // Without it, FTS search has nothing to match and the search assertion
    // would be vacuously true on an empty result set.
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repository.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = Arc::new(
        RepositoryMemoryBackend::new(repository.clone())
            .with_indexer(indexer)
            .with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                full_text_search: true,
                vector_search: false,
                embeddings: false,
                ..MemoryBackendCapabilities::default()
            }),
    );
    let adapter = Arc::new(MemoryBackendFilesystemAdapter::new(backend.clone()));
    let mut composite = CompositeRootFilesystem::new();
    composite
        .mount(memory_mount_descriptor(), adapter)
        .expect("mount /memory");
    Some((composite, repository, backend))
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_round_trips_authorized_read_write_through_composite_mount() {
    let tenant = "reborn-pg-vert-roundtrip";
    let Some((composite, _repo, _backend)) = pg_compose(tenant).await else {
        return;
    };
    let path = vpath(&format!(
        "/tenants/{tenant}/users/alice/agents/_none/projects/_none/notes/welcome.md"
    ));

    composite
        .write_file(&path, b"first welcome note")
        .await
        .expect("write must succeed");
    let stored = composite.read_file(&path).await.expect("read");
    assert_eq!(stored, b"first welcome note");
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_search_through_composite_mount_returns_only_same_scope_results() {
    let tenant = "reborn-pg-vert-scope";
    let Some((composite, _repo, backend)) = pg_compose(tenant).await else {
        return;
    };
    for (path, body) in [
        (
            format!("/tenants/{tenant}/users/alice/agents/_none/projects/proj-1/visible.md"),
            "scope-token visible body",
        ),
        (
            format!("/tenants/{tenant}/users/bob/agents/_none/projects/proj-1/hidden-user.md"),
            "scope-token hidden user",
        ),
    ] {
        composite
            .write_file(&vpath(&path), body.as_bytes())
            .await
            .unwrap();
    }

    let scope = MemoryDocumentScope::new(tenant, "alice", Some("proj-1")).unwrap();
    let context = MemoryContext::new(scope);
    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("scope-token")
                .unwrap()
                .with_vector(false)
                .with_limit(10),
        )
        .await
        .unwrap();
    let paths = results
        .iter()
        .map(|r| r.path.relative_path().to_string())
        .collect::<Vec<_>>();
    // Require non-empty results so a missing indexer wiring (the previous
    // failure mode where writes never populated `reborn_memory_chunks` and
    // search returned []) is caught — `iter().all(...)` was vacuously true on
    // an empty Vec.
    assert!(
        !paths.is_empty(),
        "expected at least one search hit; empty results would mean writes did \
         not flow through the indexer to populate reborn_memory_chunks"
    );
    assert!(
        paths.iter().all(|p| p == "visible.md"),
        "scope leak: got paths = {paths:?}"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

// --- shared spy -----------------------------------------------------------

#[derive(Default)]
struct SearchSpyRepository {
    search_calls: std::sync::Mutex<usize>,
}

impl SearchSpyRepository {
    fn search_calls(&self) -> usize {
        *self.search_calls.lock().unwrap()
    }
}

#[async_trait]
impl MemoryDocumentRepository for SearchSpyRepository {
    async fn read_document(
        &self,
        _path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        Ok(None)
    }

    async fn write_document(
        &self,
        _path: &MemoryDocumentPath,
        _bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        Ok(())
    }

    async fn list_documents(
        &self,
        _scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        Ok(Vec::new())
    }

    async fn search_documents(
        &self,
        _scope: &MemoryDocumentScope,
        _request: &MemorySearchRequest,
    ) -> Result<Vec<ironclaw_memory::MemorySearchResult>, FilesystemError> {
        *self.search_calls.lock().unwrap() += 1;
        Ok(Vec::new())
    }
}
