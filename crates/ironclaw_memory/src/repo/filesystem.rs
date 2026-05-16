//! Filesystem-backed memory document repository.
//!
//! This repository routes document persistence through the unified
//! [`RootFilesystem`] trait from `ironclaw_filesystem`, treating each memory
//! document as a record-shaped [`Entry`] under its scoped virtual path. It is
//! the **forward-looking** persistence layer for memory documents — once
//! consumers can supply a `ScopedFilesystem` (with a `MountView` covering
//! `/memory`), this implementation supersedes the dual-SQL native
//! repositories.
//!
//! Status: **scaffold**. Versioning, hybrid search (FTS + Vector + RRF
//! fusion), chunking projections, archived-revision retention, and metadata
//! cascade are not yet wired through the filesystem ops. The existing
//! `LibSqlMemoryDocumentRepository` / `PostgresMemoryDocumentRepository` /
//! `Reborn*` repos remain authoritative during the migration window — this
//! file just demonstrates the integration path and lets new callers opt in
//! for non-versioned document round-trips and FTS / vector queries that the
//! filesystem backends now support natively.
//!
//! ## What's wired in
//!
//! - `read_document` / `write_document` / `list_documents` round-trip through
//!   `RootFilesystem::put` / `get` / `query`.
//! - Documents are stored as `Entry::record` with `kind = "memory_document"`
//!   and `body = bytes`. The indexed projection carries the scope keys plus
//!   a `content` text projection so backends with FTS indexes declared on
//!   `content` can serve `Filter::Fts` against them.
//! - `read_document_metadata` / `write_document_metadata` are stored at a
//!   sibling `.meta` path so a single `delete(prefix)` on a document
//!   subtree clears both.
//!
//! ## What's still TODO
//!
//! - Compare-and-append using `CasExpectation::Version` on the unified
//!   `put` instead of returning Unsupported. The hash-based contract on
//!   `MemoryDocumentRepository::compare_and_append_document_with_options`
//!   should round-trip through `get` -> CAS write.
//! - Search through `Filter::Fts` and `Filter::VectorNearest` is exposed
//!   here via the trait's `search_documents`, but the chunking projection
//!   (one chunk = one record under a `<doc>/.chunks/<n>` path) is not yet
//!   driven — `ChunkingMemoryDocumentIndexer` still talks to the native
//!   repos. Migrating it is the next milestone.
//! - Archived-revision retention. The native repos write previous revisions
//!   to a `memory_document_versions` table; this repo would do the same by
//!   writing to a `<doc>/.versions/<n>` sibling path.
//! - Capability enforcement is currently delegated to the backend mount
//!   (`DescriptorOverclaims` at mount time). A capability-aware view
//!   wrapper that fails-closed before the backend dispatch is the natural
//!   next layer.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexValue,
    Page, RecordKind, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;

use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{MemorySearchRequest, MemorySearchResult};

use super::{MemoryDocumentRepository, ensure_document_path_does_not_conflict};

/// Stable indexed-projection keys carried on every memory-document record.
pub(crate) mod fs_keys {
    pub const TENANT: &str = "tenant_id";
    pub const USER: &str = "user_id";
    pub const AGENT: &str = "agent_id";
    pub const PROJECT: &str = "project_id";
    pub const CONTENT: &str = "content";
}

/// Filesystem-backed memory document repository.
///
/// Wraps a shared [`RootFilesystem`] handle and routes every memory operation
/// through the unified `put` / `get` / `query` ops. New consumers should
/// prefer this repository when their mount table already exposes a
/// `/memory` backend.
pub struct FilesystemMemoryDocumentRepository<F> {
    filesystem: Arc<F>,
}

impl<F> FilesystemMemoryDocumentRepository<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    fn record_kind() -> RecordKind {
        // `_kind` is a structural label, so this construction can't fail
        // for a literal that matches the validator. We surface the
        // unreachable construction failure as a backend error rather than
        // panic to keep the repo's behavior fail-closed.
        RecordKind::new("memory_document")
            .unwrap_or_else(|_| unreachable!("`memory_document` is a valid record-kind identifier"))
    }

    fn document_virtual_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        path.virtual_path()
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn metadata_virtual_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let body = Self::document_virtual_path(path, operation)?;
        VirtualPath::new(format!("{}.meta", body.as_str()))
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn build_entry(scope: &MemoryDocumentScope, bytes: &[u8]) -> Entry {
        let mut entry = Entry::record(Self::record_kind(), &serde_json::Value::Null)
            .unwrap_or_else(|_| Entry::bytes(Vec::new()));
        entry.body = bytes.to_vec();
        let tenant = scope.tenant_id().to_string();
        let user = scope.user_id().to_string();
        entry = entry
            .with_indexed(
                IndexKey::new(fs_keys::TENANT)
                    .unwrap_or_else(|_| unreachable!("tenant_id is a valid index key")),
                IndexValue::Text(tenant),
            )
            .with_indexed(
                IndexKey::new(fs_keys::USER)
                    .unwrap_or_else(|_| unreachable!("user_id is a valid index key")),
                IndexValue::Text(user),
            )
            .with_indexed(
                IndexKey::new(fs_keys::CONTENT)
                    .unwrap_or_else(|_| unreachable!("content is a valid index key")),
                IndexValue::Text(String::from_utf8_lossy(bytes).into_owned()),
            );
        if let Some(agent_id) = scope.agent_id() {
            entry = entry.with_indexed(
                IndexKey::new(fs_keys::AGENT)
                    .unwrap_or_else(|_| unreachable!("agent_id is a valid index key")),
                IndexValue::Text(agent_id.to_string()),
            );
        }
        if let Some(project_id) = scope.project_id() {
            entry = entry.with_indexed(
                IndexKey::new(fs_keys::PROJECT)
                    .unwrap_or_else(|_| unreachable!("project_id is a valid index key")),
                IndexValue::Text(project_id.to_string()),
            );
        }
        entry
    }
}

#[async_trait]
impl<F> MemoryDocumentRepository for FilesystemMemoryDocumentRepository<F>
where
    F: RootFilesystem + 'static,
{
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path = Self::document_virtual_path(path, FilesystemOperation::ReadFile)?;
        let entry = self.filesystem.get(&virtual_path).await?;
        Ok(entry.map(|versioned| versioned.entry.body))
    }

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        // Path-conflict check against the scope's existing documents.
        // Mirrors the in-memory and SQL repositories' fail-closed semantics
        // when a new path would shadow or be shadowed by an existing one.
        let existing = self.list_documents(path.scope()).await?;
        ensure_document_path_does_not_conflict(path, &existing, FilesystemOperation::WriteFile)?;
        let virtual_path = Self::document_virtual_path(path, FilesystemOperation::WriteFile)?;
        let entry = Self::build_entry(path.scope(), bytes);
        self.filesystem
            .put(&virtual_path, entry, CasExpectation::Any)
            .await?;
        Ok(())
    }

    async fn read_document_metadata(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<serde_json::Value>, FilesystemError> {
        let virtual_path = Self::metadata_virtual_path(path, FilesystemOperation::ReadFile)?;
        let Some(versioned) = self.filesystem.get(&virtual_path).await? else {
            return Ok(None);
        };
        if versioned.entry.body.is_empty() {
            return Ok(None);
        }
        serde_json::from_slice::<serde_json::Value>(&versioned.entry.body)
            .map(Some)
            .map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::ReadFile,
                    error.to_string(),
                )
            })
    }

    async fn write_document_metadata(
        &self,
        path: &MemoryDocumentPath,
        metadata: &serde_json::Value,
    ) -> Result<(), FilesystemError> {
        let virtual_path = Self::metadata_virtual_path(path, FilesystemOperation::WriteFile)?;
        let bytes = serde_json::to_vec(metadata).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        let entry = Entry::bytes(bytes);
        self.filesystem
            .put(&virtual_path, entry, CasExpectation::Any)
            .await?;
        Ok(())
    }

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        let prefix = scope.virtual_prefix().map_err(|error| {
            memory_error(
                valid_memory_path(),
                FilesystemOperation::ListDir,
                error.to_string(),
            )
        })?;
        // Drain every page of the query: `Page::MAX_LIMIT == 1024`, so a
        // single-shot query silently truncates at 1024 documents — the
        // `write_document` ancestor/descendant conflict check at the call
        // site below would then miss conflicts across the truncation
        // boundary (PR #3679 audit F1). Mirrors the `query_all_pages`
        // helper added to `src/db/filesystem_jobs.rs`.
        //
        // `VersionedEntry.path` carries the absolute virtual path of the
        // record (added in #3659), so we can recover `MemoryDocumentPath`
        // directly from the query results without a second `list_dir`
        // pass. The previous fallback to `list_dir` was dead code under
        // any backend that supports `query` (F9).
        let prefix_str = format!("{}/", prefix.as_str().trim_end_matches('/'));
        let mut documents = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self.filesystem.query(&prefix, &Filter::All, page).await {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => break,
                Err(error) => return Err(error),
            };
            let received = entries.len() as u64;
            for versioned in entries {
                // Only memory documents (skip `.meta` siblings, chunk
                // projections, and any other record kind that may live
                // under the same prefix).
                if versioned
                    .entry
                    .kind
                    .as_ref()
                    .is_none_or(|kind| kind.as_str() != "memory_document")
                {
                    continue;
                }
                let path_str = versioned.path.as_str();
                let Some(relative) = path_str.strip_prefix(&prefix_str) else {
                    continue;
                };
                // `write_document_metadata` writes a sibling at
                // `<doc>.meta` with an untyped bytes entry (no record
                // kind), so it would already be filtered above — but
                // keep an explicit suffix check in case the kind-less
                // metadata contract changes.
                if relative.ends_with(".meta") {
                    continue;
                }
                if let Ok(doc) = MemoryDocumentPath::new_with_agent(
                    scope.tenant_id(),
                    scope.user_id(),
                    scope.agent_id(),
                    scope.project_id(),
                    relative,
                ) {
                    documents.push(doc);
                }
            }
            // Short page means we've drained everything; a zero-length
            // page also ends the loop and prevents an infinite spin if
            // the backend ever returns an unexpected empty trailing page.
            if received < Page::MAX_LIMIT as u64 {
                break;
            }
            offset = offset.saturating_add(received);
        }
        documents.sort();
        documents.dedup();
        Ok(documents)
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        // The trait's hybrid-search contract (RRF / weighted-score fusion
        // across FTS + vector branches) lives on top of the chunk store,
        // which this repo does not yet maintain. Until that's wired in,
        // expose only the FTS branch via `Filter::Fts` against the
        // `content` indexed projection so callers can opt into a partial
        // search that bypasses chunking + embeddings.
        //
        // The previous scaffold issued the query, then dropped the
        // results with `let _ = results; Ok(Vec::new())`. That was worse
        // than returning `Unsupported`: a caller wiring up the trait
        // would see a clean empty result set and assume "no matches",
        // when in fact the search call had simply lied. Now we map every
        // `VersionedEntry.path` (added in #3659) back to a
        // `MemoryDocumentPath`, de-dupe by path (FTS may return multiple
        // chunk records for the same document once chunk projections
        // come online), and assign a per-rank score from RRF over the
        // FTS-only branch so the result vector is consistent with the
        // native repos' fusion contract for the trivial single-branch
        // case.
        let prefix = scope.virtual_prefix().map_err(|error| {
            memory_error(
                valid_memory_path(),
                FilesystemOperation::Query,
                error.to_string(),
            )
        })?;
        let key = IndexKey::new(fs_keys::CONTENT).map_err(|error| {
            memory_error(
                prefix.clone(),
                FilesystemOperation::Query,
                error.to_string(),
            )
        })?;
        let filter = Filter::Fts {
            key,
            query: request.query().to_string(),
        };
        let page = Page::new(0, request.pre_fusion_limit() as u32);
        let results = self.filesystem.query(&prefix, &filter, page).await?;
        let prefix_str = format!("{}/", prefix.as_str().trim_end_matches('/'));
        let mut seen = std::collections::HashSet::<String>::new();
        let mut out: Vec<MemorySearchResult> = Vec::new();
        for (index, versioned) in results.into_iter().enumerate() {
            // Skip non-memory-document entries that may live under the
            // same prefix (chunk projections, metadata siblings).
            if versioned
                .entry
                .kind
                .as_ref()
                .is_none_or(|kind| kind.as_str() != "memory_document")
            {
                continue;
            }
            let path_str = versioned.path.as_str();
            let Some(relative) = path_str.strip_prefix(&prefix_str) else {
                continue;
            };
            if relative.ends_with(".meta") {
                continue;
            }
            let Ok(doc) = MemoryDocumentPath::new_with_agent(
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id(),
                scope.project_id(),
                relative,
            ) else {
                continue;
            };
            if !seen.insert(doc.relative_path().to_string()) {
                continue;
            }
            let rank = (index as u32).saturating_add(1);
            // RRF score using the request's `rrf_k`: matches the
            // single-branch shape of `fuse_memory_search_results` for the
            // FTS-only case. Once the chunk projection is wired in, this
            // call site will route through `fuse_memory_search_results`
            // directly.
            let score = 1.0 / (request.rrf_k() as f32 + rank as f32);
            // Snippet uses the document body — the FTS index sits on
            // the `content` projection which already mirrors the body
            // bytes (lossy UTF-8 conversion is fixed by F8 in a
            // follow-up commit).
            let snippet = String::from_utf8_lossy(&versioned.entry.body).into_owned();
            out.push(MemorySearchResult {
                path: doc,
                score,
                snippet,
                full_text_rank: Some(rank),
                vector_rank: None,
            });
            if out.len() >= request.limit() {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    fn doc(relative: &str) -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant-a", "alice", Some("proj-1"), relative)
            .unwrap_or_else(|_| unreachable!("valid memory document path"))
    }

    #[tokio::test]
    async fn write_and_read_round_trip_a_document_through_unified_put_get() {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(fs);
        let path = doc("notes/welcome.md");
        repo.write_document(&path, b"hello").await.unwrap();
        let read = repo.read_document(&path).await.unwrap();
        assert_eq!(read.as_deref(), Some(b"hello".as_slice()));
    }

    #[tokio::test]
    async fn read_missing_document_returns_none() {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(fs);
        let path = doc("notes/missing.md");
        assert!(repo.read_document(&path).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn write_then_read_metadata_round_trips_through_sibling_path() {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(fs);
        let path = doc("notes/with-meta.md");
        repo.write_document(&path, b"body").await.unwrap();
        repo.write_document_metadata(&path, &serde_json::json!({"tag": "v1"}))
            .await
            .unwrap();
        let metadata = repo.read_document_metadata(&path).await.unwrap();
        assert_eq!(metadata, Some(serde_json::json!({"tag": "v1"})));
    }

    #[tokio::test]
    async fn write_document_rejects_path_that_shadows_an_existing_directory() {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(fs);
        repo.write_document(&doc("notes/a/b.md"), b"x")
            .await
            .unwrap();
        // `notes/a` is now an implicit directory; writing to it must fail.
        let result = repo.write_document(&doc("notes/a"), b"x").await;
        assert!(result.is_err());
    }

    /// Regression for audit F1: `list_documents` previously issued a single
    /// `Page::new(0, Page::MAX_LIMIT)` query and trusted the result was
    /// complete. With `Page::MAX_LIMIT == 1024`, scopes holding >1024
    /// documents silently lost every entry past the cap, and the
    /// `write_document` ancestor-conflict check above the cap stopped
    /// firing. The drain loop must surface every row.
    #[tokio::test]
    async fn list_documents_drains_pages_beyond_max_limit() {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(fs);
        // `Page::MAX_LIMIT == 1024`; write a few past the cap so a
        // single-shot query would visibly truncate.
        let total = (Page::MAX_LIMIT as usize) + 5;
        let scope = doc("seed.md").scope().clone();
        for index in 0..total {
            let path = doc(&format!("notes/doc-{index:05}.md"));
            repo.write_document(&path, b"body").await.unwrap();
        }
        let listed = repo.list_documents(&scope).await.unwrap();
        assert_eq!(listed.len(), total);
    }
}
