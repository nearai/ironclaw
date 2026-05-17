//! Filesystem-backed memory document repository.
//!
//! Routes every memory document operation through the unified
//! [`RootFilesystem`] trait from `ironclaw_filesystem`. This is the
//! single Reborn-native memory repository — the per-backend
//! `Reborn{LibSql,Postgres}MemoryDocumentRepository` SQL repos were
//! collapsed onto this implementation once `ironclaw_filesystem` grew
//! native FTS, vector, and CAS support.
//!
//! ## Storage layout
//!
//! For a document with scope `S` and relative path `R`, the on-disk
//! filesystem-virtual paths are:
//!
//! | Purpose         | Virtual path                              | Record kind                   |
//! |-----------------|-------------------------------------------|-------------------------------|
//! | Document body   | `<S>/<R>`                                 | `memory_document`             |
//! | Metadata sidecar| `<S>/<R>.meta`                            | (opaque file)                 |
//! | Chunk projection| `<S>/<R>.chunks/<n>`                      | `memory_chunk`                |
//! | Version history | `<S>/<R>.versions/<n>`                    | `memory_document_version`     |
//!
//! All sidecar subtrees are filtered defensively by record `kind` so a
//! user document that happens to share a sibling name (e.g.
//! `foo.chunks/0` as a literal user path) is never deleted or treated
//! as an internal projection.
//!
//! ## Concurrency
//!
//! Writes use CAS (`CasExpectation::Version`) where the contract
//! requires conflict detection (append, chunk replace). The
//! `RootFilesystem::begin` multi-key transaction op is intentionally
//! not relied upon — every consumer must work against backends that
//! expose only CAS (see `ironclaw_filesystem/CLAUDE.md` invariant 2).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind,
    IndexName, IndexSpec, IndexValue, Page, RecordKind, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;

use crate::chunking::{MemoryChunkWrite, content_bytes_sha256, content_sha256};
use crate::indexer::{MemoryChunkReplaceOutcome, MemoryDocumentIndexRepository};
use crate::metadata::{DocumentMetadata, MemoryWriteOptions, find_nearest_config, is_config_path};
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, fuse_memory_search_results,
};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, ensure_document_path_does_not_conflict,
    scoped_memory_changed_by_key,
};

const DOCUMENT_KIND: &str = "memory_document";
const CHUNK_KIND: &str = "memory_chunk";
const VERSION_KIND: &str = "memory_document_version";

const META_SUFFIX: &str = ".meta";
const CHUNKS_SUFFIX: &str = ".chunks";
const VERSIONS_SUFFIX: &str = ".versions";

/// Stable indexed-projection keys carried on memory records.
pub(crate) mod fs_keys {
    pub const TENANT: &str = "tenant_id";
    pub const USER: &str = "user_id";
    pub const AGENT: &str = "agent_id";
    pub const PROJECT: &str = "project_id";
    pub const CONTENT: &str = "content";
    pub const EMBEDDING: &str = "embedding";
    pub const CHUNK_INDEX: &str = "chunk_index";
    pub const DOC_PATH: &str = "doc_relative_path";
    pub const VERSION: &str = "version";
}

/// Filesystem-backed memory document repository.
///
/// Wraps a shared [`RootFilesystem`] handle and routes every memory
/// operation through the unified `put` / `get` / `query` ops.
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
}

impl<F> FilesystemMemoryDocumentRepository<F>
where
    F: RootFilesystem + 'static,
{
    fn document_kind() -> RecordKind {
        RecordKind::new(DOCUMENT_KIND)
            .unwrap_or_else(|_| unreachable!("`memory_document` is a valid record kind"))
    }

    fn chunk_kind() -> RecordKind {
        RecordKind::new(CHUNK_KIND)
            .unwrap_or_else(|_| unreachable!("`memory_chunk` is a valid record kind"))
    }

    fn version_kind() -> RecordKind {
        RecordKind::new(VERSION_KIND)
            .unwrap_or_else(|_| unreachable!("`memory_document_version` is a valid record kind"))
    }

    fn index_key(name: &'static str) -> IndexKey {
        IndexKey::new(name).unwrap_or_else(|_| unreachable!("{name} is a valid index key"))
    }

    /// Declare the FTS and Vector indexes the search path needs. libSQL
    /// and Postgres only translate `Filter::Fts` / `Filter::VectorNearest`
    /// once a matching index has been registered; without this the search
    /// silently degrades to `Unsupported`. Tolerates `Unsupported` for
    /// backends that don't materialize indexes (the in-memory backend
    /// still serves both filter kinds via in-memory scan).
    async fn ensure_search_indexes(
        &self,
        prefix: &VirtualPath,
        embedding_dim: Option<u32>,
    ) -> Result<(), FilesystemError> {
        let fts = IndexSpec::new(
            IndexName::new("memory_chunks_content_fts")
                .unwrap_or_else(|_| unreachable!("valid index name")),
            vec![Self::index_key(fs_keys::CONTENT)],
            IndexKind::Fts,
        );
        match self.filesystem.ensure_index(prefix, &fts).await {
            Ok(()) | Err(FilesystemError::Unsupported { .. }) => {}
            Err(error) => return Err(error),
        }
        if let Some(dim) = embedding_dim {
            let vector = IndexSpec::new(
                IndexName::new("memory_chunks_embedding_vector")
                    .unwrap_or_else(|_| unreachable!("valid index name")),
                vec![Self::index_key(fs_keys::EMBEDDING)],
                IndexKind::Vector { dim },
            );
            match self.filesystem.ensure_index(prefix, &vector).await {
                Ok(()) | Err(FilesystemError::Unsupported { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn document_virtual_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        path.virtual_path()
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn sibling_path(
        path: &MemoryDocumentPath,
        suffix: &str,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let body = Self::document_virtual_path(path, operation)?;
        VirtualPath::new(format!("{}{suffix}", body.as_str()))
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn metadata_virtual_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        Self::sibling_path(path, META_SUFFIX, operation)
    }

    fn chunks_prefix_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        Self::sibling_path(path, CHUNKS_SUFFIX, operation)
    }

    fn versions_prefix_path(
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        Self::sibling_path(path, VERSIONS_SUFFIX, operation)
    }

    fn chunk_child_path(
        path: &MemoryDocumentPath,
        index: usize,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let prefix = Self::chunks_prefix_path(path, operation)?;
        VirtualPath::new(format!("{}/{index}", prefix.as_str()))
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn version_child_path(
        path: &MemoryDocumentPath,
        version: u64,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let prefix = Self::versions_prefix_path(path, operation)?;
        VirtualPath::new(format!("{}/{version}", prefix.as_str()))
            .map_err(|error| memory_error(valid_memory_path(), operation, error.to_string()))
    }

    fn apply_scope_projection(mut entry: Entry, scope: &MemoryDocumentScope) -> Entry {
        entry = entry
            .with_indexed(
                Self::index_key(fs_keys::TENANT),
                IndexValue::Text(scope.tenant_id().to_string()),
            )
            .with_indexed(
                Self::index_key(fs_keys::USER),
                IndexValue::Text(scope.user_id().to_string()),
            );
        if let Some(agent_id) = scope.agent_id() {
            entry = entry.with_indexed(
                Self::index_key(fs_keys::AGENT),
                IndexValue::Text(agent_id.to_string()),
            );
        }
        if let Some(project_id) = scope.project_id() {
            entry = entry.with_indexed(
                Self::index_key(fs_keys::PROJECT),
                IndexValue::Text(project_id.to_string()),
            );
        }
        entry
    }

    fn build_document_entry(scope: &MemoryDocumentScope, bytes: &[u8]) -> Entry {
        let mut entry = Entry::record(Self::document_kind(), &serde_json::Value::Null)
            .unwrap_or_else(|_| Entry::bytes(Vec::new()));
        entry.body = bytes.to_vec();
        Self::apply_scope_projection(entry, scope)
    }

    fn build_chunk_entry(
        path: &MemoryDocumentPath,
        index: usize,
        chunk: &MemoryChunkWrite,
    ) -> Entry {
        let mut entry = Entry::record(Self::chunk_kind(), &serde_json::Value::Null)
            .unwrap_or_else(|_| Entry::bytes(Vec::new()));
        entry.body = chunk.content.as_bytes().to_vec();
        entry = Self::apply_scope_projection(entry, path.scope())
            .with_indexed(
                Self::index_key(fs_keys::CONTENT),
                IndexValue::Text(chunk.content.clone()),
            )
            .with_indexed(
                Self::index_key(fs_keys::CHUNK_INDEX),
                IndexValue::I64(index as i64),
            )
            .with_indexed(
                Self::index_key(fs_keys::DOC_PATH),
                IndexValue::Text(path.relative_path().to_string()),
            );
        if let Some(embedding) = &chunk.embedding {
            entry = entry.with_indexed(
                Self::index_key(fs_keys::EMBEDDING),
                IndexValue::Bytes(encode_embedding_blob(embedding)),
            );
        }
        entry
    }

    fn build_version_entry(
        path: &MemoryDocumentPath,
        version: u64,
        previous_content: &str,
        changed_by: Option<&str>,
    ) -> Entry {
        let mut payload = serde_json::Map::new();
        payload.insert(
            "content".to_string(),
            serde_json::Value::String(previous_content.to_string()),
        );
        payload.insert(
            "content_hash".to_string(),
            serde_json::Value::String(content_sha256(previous_content)),
        );
        if let Some(changed_by) = changed_by {
            payload.insert(
                "changed_by".to_string(),
                serde_json::Value::String(changed_by.to_string()),
            );
        }
        let mut entry = Entry::record(Self::version_kind(), &serde_json::Value::Object(payload))
            .unwrap_or_else(|_| Entry::bytes(Vec::new()));
        entry = Self::apply_scope_projection(entry, path.scope())
            .with_indexed(
                Self::index_key(fs_keys::DOC_PATH),
                IndexValue::Text(path.relative_path().to_string()),
            )
            .with_indexed(
                Self::index_key(fs_keys::VERSION),
                IndexValue::I64(version as i64),
            );
        entry
    }

    /// Iterate every record under `scope` in pages of `Page::MAX_LIMIT`,
    /// regardless of how many records the backend stores. Mirrors the
    /// helper that the scaffold's prior `list_documents` used (PR #3679
    /// audit F1 — single-shot queries silently truncate at MAX_LIMIT).
    async fn query_all(
        &self,
        prefix: &VirtualPath,
        filter: &Filter,
    ) -> Result<Vec<ironclaw_filesystem::VersionedEntry>, FilesystemError> {
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = match self.filesystem.query(prefix, filter, page).await {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => break,
                Err(error) => return Err(error),
            };
            let received = entries.len() as u64;
            out.extend(entries);
            if received < Page::MAX_LIMIT as u64 {
                break;
            }
            offset = offset.saturating_add(received);
        }
        Ok(out)
    }

    /// Read the document body bytes plus its current backend version
    /// at the moment of the read. The version travels with the bytes
    /// so the caller can drive `CasExpectation::Version` against the
    /// same observation.
    async fn read_document_with_version(
        &self,
        path: &MemoryDocumentPath,
        operation: FilesystemOperation,
    ) -> Result<Option<(Vec<u8>, ironclaw_filesystem::RecordVersion)>, FilesystemError> {
        let virtual_path = Self::document_virtual_path(path, operation)?;
        let entry = self.filesystem.get(&virtual_path).await?;
        Ok(entry.map(|versioned| (versioned.entry.body, versioned.version)))
    }

    async fn save_document_version(
        &self,
        path: &MemoryDocumentPath,
        previous_content: &str,
        changed_by: Option<&str>,
    ) -> Result<u64, FilesystemError> {
        let prefix = Self::versions_prefix_path(path, FilesystemOperation::WriteFile)?;
        let entries = self.query_all(&prefix, &Filter::All).await?;
        let next_version = entries
            .iter()
            .filter(|versioned| {
                versioned
                    .entry
                    .kind
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == VERSION_KIND)
            })
            .filter_map(|versioned| {
                match versioned
                    .entry
                    .indexed
                    .get(&Self::index_key(fs_keys::VERSION))
                {
                    Some(IndexValue::I64(version)) => u64::try_from(*version).ok(),
                    _ => None,
                }
            })
            .max()
            .unwrap_or(0)
            + 1;
        let entry = Self::build_version_entry(path, next_version, previous_content, changed_by);
        let virtual_path =
            Self::version_child_path(path, next_version, FilesystemOperation::WriteFile)?;
        self.filesystem
            .put(&virtual_path, entry, CasExpectation::Absent)
            .await?;
        Ok(next_version)
    }

    /// Delete every chunk record under a document's `.chunks/` sibling
    /// subtree, leaving any non-chunk record (e.g. a literal user
    /// document at a colliding path) untouched.
    async fn delete_chunks_kind_aware(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError> {
        let prefix = Self::chunks_prefix_path(path, FilesystemOperation::WriteFile)?;
        let entries = self.query_all(&prefix, &Filter::All).await?;
        for versioned in entries {
            let is_chunk = versioned
                .entry
                .kind
                .as_ref()
                .is_some_and(|kind| kind.as_str() == CHUNK_KIND);
            if !is_chunk {
                continue;
            }
            match self.filesystem.delete(&versioned.path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Resolve `(doc_path, doc_metadata)` for every document in `scope`,
    /// then look up effective metadata via `.config` inheritance. Used
    /// by `write_document_metadata` to decide which docs should have
    /// their chunks cleared after a metadata write that turns
    /// `skip_indexing` on.
    async fn collect_scope_resolved_metadata(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<(MemoryDocumentPath, DocumentMetadata)>, FilesystemError> {
        let documents = self.list_documents(scope).await?;
        let mut config_metadata = HashMap::<String, serde_json::Value>::new();
        let mut doc_metadata = HashMap::<String, serde_json::Value>::new();
        for document in &documents {
            let metadata = self
                .read_document_metadata(document)
                .await?
                .unwrap_or_else(|| serde_json::json!({}));
            if is_config_path(document.relative_path()) {
                config_metadata.insert(document.relative_path().to_string(), metadata.clone());
            }
            doc_metadata.insert(document.relative_path().to_string(), metadata);
        }
        Ok(documents
            .into_iter()
            .map(|document| {
                let raw = doc_metadata
                    .get(document.relative_path())
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let base = find_nearest_config(document.relative_path(), &config_metadata)
                    .unwrap_or_else(|| serde_json::json!({}));
                let resolved = DocumentMetadata::from_value(&DocumentMetadata::merge(&base, &raw));
                (document, resolved)
            })
            .collect())
    }

    fn document_relative_path_from_chunk_entry(
        entry: &ironclaw_filesystem::VersionedEntry,
    ) -> Option<String> {
        match entry
            .entry
            .indexed
            .get(&IndexKey::new(fs_keys::DOC_PATH).ok()?)
        {
            Some(IndexValue::Text(value)) => Some(value.clone()),
            _ => None,
        }
    }

    fn map_chunk_hits_to_ranked_results(
        scope: &MemoryDocumentScope,
        entries: Vec<ironclaw_filesystem::VersionedEntry>,
    ) -> Vec<RankedMemorySearchResult> {
        let mut results = Vec::with_capacity(entries.len());
        for (index, versioned) in entries.into_iter().enumerate() {
            if versioned
                .entry
                .kind
                .as_ref()
                .is_none_or(|kind| kind.as_str() != CHUNK_KIND)
            {
                continue;
            }
            let Some(doc_relative) = Self::document_relative_path_from_chunk_entry(&versioned)
            else {
                continue;
            };
            let Ok(doc_path) = MemoryDocumentPath::new_with_agent(
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id(),
                scope.project_id(),
                doc_relative,
            ) else {
                continue;
            };
            let snippet = String::from_utf8_lossy(&versioned.entry.body).into_owned();
            results.push(RankedMemorySearchResult {
                path: doc_path,
                snippet,
                rank: (index as u32).saturating_add(1),
            });
        }
        results
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
        Ok(self
            .read_document_with_version(path, FilesystemOperation::ReadFile)
            .await?
            .map(|(bytes, _)| bytes))
    }

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        // Direct repository writes pick up the same default
        // `changed_by` the native repos used so version-history
        // attribution survives bypass of the higher backend seam.
        let options = MemoryWriteOptions {
            changed_by: Some(scoped_memory_changed_by_key(path.scope())),
            ..MemoryWriteOptions::default()
        };
        self.write_document_with_options(path, bytes, &options)
            .await
    }

    async fn write_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<(), FilesystemError> {
        // Path-conflict check against existing documents in the scope:
        // a new path must not shadow or be shadowed by another doc.
        let existing_documents = self.list_documents(path.scope()).await?;
        ensure_document_path_does_not_conflict(
            path,
            &existing_documents,
            FilesystemOperation::WriteFile,
        )?;

        let previous = self
            .read_document_with_version(path, FilesystemOperation::WriteFile)
            .await?;

        // PR #3679 review fix (finding #2): the document CAS write must
        // happen BEFORE the version archive, not after. The previous
        // order archived the prior content first, then attempted the
        // CAS put — if a concurrent writer won the CAS race the loser
        // had already inserted a `.versions/<n>` record for an
        // overwrite/append that never happened, corrupting the version
        // history (and potentially blocking the real writer's later
        // archive, since `save_document_version` uses
        // `CasExpectation::Absent` for the computed next version).
        // Doing the put first means a CAS loss returns
        // `VersionMismatch` before any sidecar is touched.
        let virtual_path = Self::document_virtual_path(path, FilesystemOperation::WriteFile)?;
        let entry = Self::build_document_entry(path.scope(), bytes);
        let cas = match previous.as_ref() {
            Some((_, version)) => CasExpectation::Version(*version),
            None => CasExpectation::Absent,
        };
        self.filesystem.put(&virtual_path, entry, cas).await?;

        // Document write succeeded — now archive the prior content. A
        // failure here is best-effort: the new body is durable, the
        // version trail just gains a gap. `save_document_version`
        // already tolerates concurrent writers via `Absent` CAS.
        if let Some((previous_bytes, _previous_version)) = previous.as_ref() {
            let previous_content = std::str::from_utf8(previous_bytes).map_err(|_| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    "memory document content must be UTF-8",
                )
            })?;
            let new_content = std::str::from_utf8(bytes).map_err(|_| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    "memory document content must be UTF-8",
                )
            })?;
            let should_version = options.metadata.skip_versioning != Some(true)
                && previous_content != new_content
                && !previous_content.is_empty();
            if should_version {
                self.save_document_version(path, previous_content, options.changed_by.as_deref())
                    .await?;
            }
        }
        Ok(())
    }

    async fn compare_and_append_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let append_content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                Self::document_virtual_path(path, FilesystemOperation::AppendFile)
                    .unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::AppendFile,
                "memory document content must be UTF-8",
            )
        })?;

        let previous = self
            .read_document_with_version(path, FilesystemOperation::AppendFile)
            .await?;

        // Hash-compare against the read body. We recompute the hash on
        // every call (rather than trust a stored hash projection) so
        // the contract stays bit-identical to the native repos that
        // also recomputed.
        let current_hash = previous
            .as_ref()
            .map(|(bytes, _)| content_bytes_sha256(bytes));
        if current_hash.as_deref() != expected_previous_hash {
            return Ok(MemoryAppendOutcome::Conflict);
        }

        let virtual_path = Self::document_virtual_path(path, FilesystemOperation::AppendFile)?;
        match previous {
            Some((previous_bytes, previous_version)) => {
                let previous_content = std::str::from_utf8(&previous_bytes).map_err(|_| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::AppendFile,
                        "memory document content must be UTF-8",
                    )
                })?;
                let combined = format!("{previous_content}{append_content}");
                let should_version = options.metadata.skip_versioning != Some(true)
                    && previous_content != combined
                    && !previous_content.is_empty();
                // PR #3679 review fix (finding #2): document CAS first,
                // version archive after success. See the matching comment
                // in `write_document_with_options`.
                let entry = Self::build_document_entry(path.scope(), combined.as_bytes());
                let put_result = self
                    .filesystem
                    .put(
                        &virtual_path,
                        entry,
                        CasExpectation::Version(previous_version),
                    )
                    .await;
                match put_result {
                    Ok(_) => {
                        if should_version {
                            self.save_document_version(
                                path,
                                previous_content,
                                options.changed_by.as_deref(),
                            )
                            .await?;
                        }
                        Ok(MemoryAppendOutcome::Appended)
                    }
                    Err(FilesystemError::VersionMismatch { .. }) => {
                        Ok(MemoryAppendOutcome::Conflict)
                    }
                    Err(error) => Err(error),
                }
            }
            None => {
                let existing_documents = self.list_documents(path.scope()).await?;
                ensure_document_path_does_not_conflict(
                    path,
                    &existing_documents,
                    FilesystemOperation::AppendFile,
                )?;
                let entry = Self::build_document_entry(path.scope(), bytes);
                match self
                    .filesystem
                    .put(&virtual_path, entry, CasExpectation::Absent)
                    .await
                {
                    Ok(_) => Ok(MemoryAppendOutcome::Appended),
                    Err(FilesystemError::VersionMismatch { .. }) => {
                        Ok(MemoryAppendOutcome::Conflict)
                    }
                    Err(error) => Err(error),
                }
            }
        }
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
        // Gate the chunk-clear cascade on the targeted document
        // actually existing: the native repos required `rows_affected
        // > 0` from the UPDATE before scanning the scope to clear
        // descendant chunks (zmanian #3180 MED). For a metadata-only
        // path that targets a `.config` we treat "config exists OR
        // some descendant doc exists" as the gate, mirroring the SQL
        // contract: writing a fresh root `.config` doesn't wipe
        // descendants until the config row itself exists.
        let document_exists = self.read_document(path).await?.is_some();

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

        let parsed_metadata = DocumentMetadata::from_value(metadata);
        if !document_exists || parsed_metadata.skip_indexing != Some(true) {
            return Ok(());
        }

        let resolved = self.collect_scope_resolved_metadata(path.scope()).await?;
        for (doc_path, resolved_metadata) in resolved {
            if !metadata_clear_applies_to(path.relative_path(), doc_path.relative_path()) {
                continue;
            }
            if resolved_metadata.skip_indexing != Some(true) {
                continue;
            }
            self.delete_chunks_kind_aware(&doc_path).await?;
        }
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
        let prefix_str = format!("{}/", prefix.as_str().trim_end_matches('/'));
        let entries = self.query_all(&prefix, &Filter::All).await?;
        let mut documents = Vec::new();
        let mut seen = HashSet::<String>::new();
        for versioned in entries {
            // Only true document records contribute to the listing.
            if versioned
                .entry
                .kind
                .as_ref()
                .is_none_or(|kind| kind.as_str() != DOCUMENT_KIND)
            {
                continue;
            }
            let Some(relative) = versioned.path.as_str().strip_prefix(&prefix_str) else {
                continue;
            };
            // Defensive sibling-suffix filter: even though kind alone
            // should exclude these, the convention is small enough
            // that being explicit guards against any future
            // kind-less write landing on a sibling subtree.
            if relative.ends_with(META_SUFFIX) || relative_is_sidecar(relative) {
                continue;
            }
            if !seen.insert(relative.to_string()) {
                continue;
            }
            if let Ok(document) = MemoryDocumentPath::new_with_agent(
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id(),
                scope.project_id(),
                relative,
            ) {
                documents.push(document);
            }
        }
        documents.sort();
        Ok(documents)
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        let prefix = scope.virtual_prefix().map_err(|error| {
            memory_error(
                valid_memory_path(),
                FilesystemOperation::Query,
                error.to_string(),
            )
        })?;

        // PR #3679 review fix (finding #4): declare the FTS / Vector
        // indexes the query path requires. libSQL / Postgres backends
        // need the index registered before they translate `Filter::Fts`
        // / `Filter::VectorNearest` to a native query — without it they
        // return `Unsupported` and the search silently degrades. Tolerate
        // `Unsupported` for backends that don't support these index
        // kinds (the in-memory backend still serves FTS over a scan).
        self.ensure_search_indexes(&prefix, request.query_embedding_dim())
            .await?;

        let full_text_results = if request.full_text() {
            let filter = Filter::Fts {
                key: Self::index_key(fs_keys::CONTENT),
                query: request.query().to_string(),
            };
            let page = Page::new(
                0,
                request.pre_fusion_limit().min(Page::MAX_LIMIT as usize) as u32,
            );
            let entries = self.filesystem.query(&prefix, &filter, page).await?;
            Self::map_chunk_hits_to_ranked_results(scope, entries)
        } else {
            Vec::new()
        };

        let vector_results = if request.vector() {
            if let Some(embedding) = request.query_embedding() {
                let filter = Filter::VectorNearest {
                    key: Self::index_key(fs_keys::EMBEDDING),
                    embedding: embedding.to_vec(),
                    limit: request.pre_fusion_limit() as u32,
                };
                // `VectorNearest` overrides `Page::limit`; the limit on
                // the filter is what drives top-k. We still pass a
                // bounded page to satisfy the API.
                let page = Page::new(
                    0,
                    request.pre_fusion_limit().min(Page::MAX_LIMIT as usize) as u32,
                );
                let entries = self.filesystem.query(&prefix, &filter, page).await?;
                Self::map_chunk_hits_to_ranked_results(scope, entries)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Ok(fuse_memory_search_results(
            full_text_results,
            vector_results,
            request,
        ))
    }
}

#[async_trait]
impl<F> MemoryDocumentIndexRepository for FilesystemMemoryDocumentRepository<F>
where
    F: RootFilesystem + 'static,
{
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<MemoryChunkReplaceOutcome, FilesystemError> {
        let Some((current_bytes, _)) = self
            .read_document_with_version(path, FilesystemOperation::WriteFile)
            .await?
        else {
            return Ok(MemoryChunkReplaceOutcome::SkippedMissingDocument);
        };
        if content_bytes_sha256(&current_bytes) != expected_content_hash {
            return Ok(MemoryChunkReplaceOutcome::SkippedStaleContentHash);
        }
        // PR #3679 review fix (finding #3): re-check the document hash
        // BEFORE the sweep, not after. The original ordering (sweep, then
        // re-check) would delete a winning reindexer's fresh chunks if
        // they landed between our initial read and the sweep — the
        // post-sweep hash check prevented writing stale chunks on top,
        // but the deletion itself was data loss. Doing the second read
        // first means a stale worker observes the document moved on and
        // returns `SkippedStaleContentHash` without touching the chunk
        // subtree. There is still a narrow window between the second
        // hash check and the sweep, but that window cannot lose data
        // because any concurrent writer's chunks are guarded by the
        // per-chunk `CasExpectation::Absent` check at write time below.
        let Some((bytes_pre_sweep, _)) = self
            .read_document_with_version(path, FilesystemOperation::WriteFile)
            .await?
        else {
            return Ok(MemoryChunkReplaceOutcome::SkippedMissingDocument);
        };
        if content_bytes_sha256(&bytes_pre_sweep) != expected_content_hash {
            return Ok(MemoryChunkReplaceOutcome::SkippedStaleContentHash);
        }
        // Sweep the chunk subtree so the new chunk numbering starts
        // cleanly from zero.
        self.delete_chunks_kind_aware(path).await?;
        for (index, chunk) in chunks.iter().enumerate() {
            let entry = Self::build_chunk_entry(path, index, chunk);
            let chunk_path = Self::chunk_child_path(path, index, FilesystemOperation::WriteFile)?;
            // CAS::Absent: the post-sweep state should have no chunk
            // at this index. If a racing reindex slipped in between,
            // surface as Stale so the caller's hash-guard contract
            // decides what to do next (the next reindex on the
            // winner's content_hash will re-fill).
            match self
                .filesystem
                .put(&chunk_path, entry, CasExpectation::Absent)
                .await
            {
                Ok(_) => {}
                Err(FilesystemError::VersionMismatch { .. }) => {
                    return Ok(MemoryChunkReplaceOutcome::SkippedStaleContentHash);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(MemoryChunkReplaceOutcome::Replaced)
    }
}

fn relative_is_sidecar(relative: &str) -> bool {
    relative
        .split('/')
        .any(|segment| segment.ends_with(CHUNKS_SUFFIX) || segment.ends_with(VERSIONS_SUFFIX))
}

fn metadata_clear_applies_to(metadata_target_path: &str, candidate_path: &str) -> bool {
    if !is_config_path(metadata_target_path) {
        return candidate_path == metadata_target_path;
    }
    match metadata_target_path.rsplit_once('/') {
        Some((parent, _)) => candidate_path.starts_with(&format!("{parent}/")),
        None => true,
    }
}

fn encode_embedding_blob(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use std::sync::Arc;

    fn doc(relative: &str) -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant-a", "alice", Some("proj-1"), relative)
            .unwrap_or_else(|_| unreachable!("valid memory document path"))
    }

    fn fresh_repo() -> (
        Arc<InMemoryBackend>,
        FilesystemMemoryDocumentRepository<InMemoryBackend>,
    ) {
        let fs = Arc::new(InMemoryBackend::new());
        let repo = FilesystemMemoryDocumentRepository::new(Arc::clone(&fs));
        (fs, repo)
    }

    fn search_request(query: &str) -> MemorySearchRequest {
        MemorySearchRequest::new(query).expect("non-empty query")
    }

    #[tokio::test]
    async fn write_and_read_round_trip_a_document_through_unified_put_get() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/welcome.md");
        repo.write_document(&path, b"hello").await.unwrap();
        let read = repo.read_document(&path).await.unwrap();
        assert_eq!(read.as_deref(), Some(b"hello".as_slice()));
    }

    #[tokio::test]
    async fn read_missing_document_returns_none() {
        let (_, repo) = fresh_repo();
        assert!(
            repo.read_document(&doc("notes/missing.md"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn write_then_read_metadata_round_trips_through_sibling_path() {
        let (_, repo) = fresh_repo();
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
        let (_, repo) = fresh_repo();
        repo.write_document(&doc("notes/a/b.md"), b"x")
            .await
            .unwrap();
        let result = repo.write_document(&doc("notes/a"), b"x").await;
        assert!(result.is_err());
    }

    /// Regression for audit F1: `list_documents` previously issued a single
    /// `Page::new(0, Page::MAX_LIMIT)` query and trusted the result was
    /// complete. The drain loop must surface every row past the cap.
    #[tokio::test]
    async fn list_documents_drains_pages_beyond_max_limit() {
        let (_, repo) = fresh_repo();
        let total = (Page::MAX_LIMIT as usize) + 5;
        let scope = doc("seed.md").scope().clone();
        for index in 0..total {
            let path = doc(&format!("notes/doc-{index:05}.md"));
            repo.write_document(&path, b"body").await.unwrap();
        }
        let listed = repo.list_documents(&scope).await.unwrap();
        assert_eq!(listed.len(), total);
    }

    #[tokio::test]
    async fn overwrite_archives_previous_content_to_a_version_sibling() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/versioned.md");
        repo.write_document(&path, b"first").await.unwrap();
        repo.write_document(&path, b"second").await.unwrap();
        // Newest body served from the document path.
        assert_eq!(
            repo.read_document(&path).await.unwrap().as_deref(),
            Some(b"second".as_slice())
        );
        // Version 1 lands at the `.versions/1` sibling with the previous body.
        let version_path =
            FilesystemMemoryDocumentRepository::<InMemoryBackend>::version_child_path(
                &path,
                1,
                FilesystemOperation::ReadFile,
            )
            .unwrap();
        let entry = repo.filesystem.get(&version_path).await.unwrap().unwrap();
        assert_eq!(entry.entry.kind.as_ref().unwrap().as_str(), VERSION_KIND);
        let payload: serde_json::Value = serde_json::from_slice(&entry.entry.body).unwrap();
        assert_eq!(payload["content"], serde_json::json!("first"));
    }

    #[tokio::test]
    async fn overwrite_with_skip_versioning_does_not_write_a_version_row() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/skip-version.md");
        repo.write_document(&path, b"alpha").await.unwrap();
        let mut options = MemoryWriteOptions {
            changed_by: Some("test".to_string()),
            ..Default::default()
        };
        options.metadata.skip_versioning = Some(true);
        repo.write_document_with_options(&path, b"beta", &options)
            .await
            .unwrap();
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::versions_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let versions: Vec<_> = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == VERSION_KIND)
            })
            .collect();
        assert!(
            versions.is_empty(),
            "skip_versioning must suppress version sibling writes"
        );
    }

    #[tokio::test]
    async fn overwrite_with_identical_content_does_not_write_a_version_row() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/idempotent.md");
        repo.write_document(&path, b"same").await.unwrap();
        repo.write_document(&path, b"same").await.unwrap();
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::versions_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let versions = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == VERSION_KIND)
            })
            .count();
        assert_eq!(
            versions, 0,
            "identical content overwrite must not archive a version"
        );
    }

    #[tokio::test]
    async fn compare_and_append_with_matching_hash_appends_and_archives() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/append.md");
        repo.write_document(&path, b"hello").await.unwrap();
        let expected = content_sha256("hello");
        let options = MemoryWriteOptions {
            changed_by: Some("test".to_string()),
            ..Default::default()
        };
        let outcome = repo
            .compare_and_append_document_with_options(&path, Some(&expected), b" world", &options)
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Appended);
        assert_eq!(
            repo.read_document(&path).await.unwrap().as_deref(),
            Some(b"hello world".as_slice())
        );
    }

    #[tokio::test]
    async fn compare_and_append_with_mismatched_hash_returns_conflict() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/append-conflict.md");
        repo.write_document(&path, b"hello").await.unwrap();
        let outcome = repo
            .compare_and_append_document_with_options(
                &path,
                Some("sha256:not-the-actual-hash"),
                b"!",
                &MemoryWriteOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Conflict);
        // Body must remain untouched on Conflict.
        assert_eq!(
            repo.read_document(&path).await.unwrap().as_deref(),
            Some(b"hello".as_slice())
        );
    }

    #[tokio::test]
    async fn compare_and_append_to_missing_document_creates_it_when_hash_is_none() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/append-fresh.md");
        let outcome = repo
            .compare_and_append_document_with_options(
                &path,
                None,
                b"fresh",
                &MemoryWriteOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Appended);
        assert_eq!(
            repo.read_document(&path).await.unwrap().as_deref(),
            Some(b"fresh".as_slice())
        );
    }

    #[tokio::test]
    async fn replace_chunks_inserts_records_under_chunks_subtree() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/chunked.md");
        repo.write_document(&path, b"alpha beta gamma")
            .await
            .unwrap();
        let hash = content_sha256("alpha beta gamma");
        let chunks = vec![
            MemoryChunkWrite {
                content: "alpha beta".to_string(),
                embedding: Some(vec![0.1, 0.2, 0.3]),
            },
            MemoryChunkWrite {
                content: "gamma".to_string(),
                embedding: None,
            },
        ];
        let outcome = repo
            .replace_document_chunks_if_current(&path, &hash, &chunks)
            .await
            .unwrap();
        assert_eq!(outcome, MemoryChunkReplaceOutcome::Replaced);

        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let chunk_entries: Vec<_> = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .collect();
        assert_eq!(chunk_entries.len(), 2);
    }

    #[tokio::test]
    async fn replace_chunks_with_stale_hash_returns_skipped_and_does_not_mutate() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/stale-chunks.md");
        repo.write_document(&path, b"orig").await.unwrap();
        let chunks = vec![MemoryChunkWrite {
            content: "orig".to_string(),
            embedding: None,
        }];
        let outcome = repo
            .replace_document_chunks_if_current(&path, "sha256:nope", &chunks)
            .await
            .unwrap();
        assert_eq!(outcome, MemoryChunkReplaceOutcome::SkippedStaleContentHash);
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        assert!(
            entries.iter().all(|v| v
                .entry
                .kind
                .as_ref()
                .is_none_or(|k| k.as_str() != CHUNK_KIND)),
            "stale hash must not insert chunks"
        );
    }

    #[tokio::test]
    async fn replace_chunks_for_missing_document_returns_skipped_missing() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/no-such-doc.md");
        let outcome = repo
            .replace_document_chunks_if_current(&path, "sha256:irrelevant", &[])
            .await
            .unwrap();
        assert_eq!(outcome, MemoryChunkReplaceOutcome::SkippedMissingDocument);
    }

    #[tokio::test]
    async fn replace_chunks_with_empty_chunk_list_clears_existing_chunks() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/clearable.md");
        repo.write_document(&path, b"body").await.unwrap();
        let hash = content_sha256("body");
        let chunks = vec![MemoryChunkWrite {
            content: "body".to_string(),
            embedding: None,
        }];
        repo.replace_document_chunks_if_current(&path, &hash, &chunks)
            .await
            .unwrap();
        repo.replace_document_chunks_if_current(&path, &hash, &[])
            .await
            .unwrap();
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let chunk_entries = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .count();
        assert_eq!(chunk_entries, 0);
    }

    #[tokio::test]
    async fn metadata_skip_indexing_true_clears_chunks_for_the_targeted_doc() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/will-skip.md");
        repo.write_document(&path, b"body").await.unwrap();
        let hash = content_sha256("body");
        repo.replace_document_chunks_if_current(
            &path,
            &hash,
            &[MemoryChunkWrite {
                content: "body".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();

        repo.write_document_metadata(&path, &serde_json::json!({"skip_indexing": true}))
            .await
            .unwrap();

        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let chunk_entries = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .count();
        assert_eq!(chunk_entries, 0, "skip_indexing must clear existing chunks");
    }

    #[tokio::test]
    async fn config_skip_indexing_clears_descendant_chunks_but_not_unrelated_paths() {
        let (_, repo) = fresh_repo();
        let folder_doc = doc("folder/inside.md");
        let other_doc = doc("other/outside.md");
        repo.write_document(&folder_doc, b"inside").await.unwrap();
        repo.write_document(&other_doc, b"outside").await.unwrap();
        repo.replace_document_chunks_if_current(
            &folder_doc,
            &content_sha256("inside"),
            &[MemoryChunkWrite {
                content: "inside".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();
        repo.replace_document_chunks_if_current(
            &other_doc,
            &content_sha256("outside"),
            &[MemoryChunkWrite {
                content: "outside".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();

        let config_path = doc("folder/.config");
        repo.write_document(&config_path, b"{}").await.unwrap();
        repo.write_document_metadata(&config_path, &serde_json::json!({"skip_indexing": true}))
            .await
            .unwrap();

        // Folder doc's chunks gone; sibling-folder doc unaffected.
        let folder_prefix =
            FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
                &folder_doc,
                FilesystemOperation::ReadFile,
            )
            .unwrap();
        let folder_entries = repo.query_all(&folder_prefix, &Filter::All).await.unwrap();
        assert!(
            folder_entries.iter().all(|v| v
                .entry
                .kind
                .as_ref()
                .is_none_or(|k| k.as_str() != CHUNK_KIND)),
            "folder/.config skip_indexing must clear descendant chunks"
        );

        let other_prefix =
            FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
                &other_doc,
                FilesystemOperation::ReadFile,
            )
            .unwrap();
        let other_entries = repo.query_all(&other_prefix, &Filter::All).await.unwrap();
        let other_chunks = other_entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .count();
        assert_eq!(
            other_chunks, 1,
            "unrelated path's chunks must survive the .config cascade"
        );
    }

    #[tokio::test]
    async fn config_skip_indexing_respects_descendant_override() {
        let (_, repo) = fresh_repo();
        let descendant = doc("folder/note.md");
        repo.write_document(&descendant, b"body").await.unwrap();
        let hash = content_sha256("body");
        repo.replace_document_chunks_if_current(
            &descendant,
            &hash,
            &[MemoryChunkWrite {
                content: "body".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();

        // Descendant explicitly overrides skip_indexing=false.
        repo.write_document_metadata(&descendant, &serde_json::json!({"skip_indexing": false}))
            .await
            .unwrap();

        let config_path = doc("folder/.config");
        repo.write_document(&config_path, b"{}").await.unwrap();
        repo.write_document_metadata(&config_path, &serde_json::json!({"skip_indexing": true}))
            .await
            .unwrap();

        // Resolved metadata for the descendant: explicit `false` wins
        // over inherited `true`, so chunks must survive.
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &descendant,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let chunk_entries = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .count();
        assert_eq!(
            chunk_entries, 1,
            "explicit descendant skip_indexing=false must beat parent .config skip_indexing=true"
        );
    }

    #[tokio::test]
    async fn metadata_on_missing_root_config_does_not_clear_unrelated_chunks() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/keepers.md");
        repo.write_document(&path, b"body").await.unwrap();
        repo.replace_document_chunks_if_current(
            &path,
            &content_sha256("body"),
            &[MemoryChunkWrite {
                content: "body".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();
        // Write metadata at a root-level `.config` that has no document
        // backing — `rows_affected`-equivalent gate must keep the
        // descendant chunks intact (zmanian #3180 MED gate).
        let root_config = doc(".config");
        repo.write_document_metadata(&root_config, &serde_json::json!({"skip_indexing": true}))
            .await
            .unwrap();
        let prefix = FilesystemMemoryDocumentRepository::<InMemoryBackend>::chunks_prefix_path(
            &path,
            FilesystemOperation::ReadFile,
        )
        .unwrap();
        let entries = repo.query_all(&prefix, &Filter::All).await.unwrap();
        let chunk_entries = entries
            .iter()
            .filter(|v| {
                v.entry
                    .kind
                    .as_ref()
                    .is_some_and(|k| k.as_str() == CHUNK_KIND)
            })
            .count();
        assert_eq!(
            chunk_entries, 1,
            "metadata write to a non-existent .config must not wipe descendants"
        );
    }

    #[tokio::test]
    async fn search_returns_chunk_hits_mapped_back_to_document_paths() {
        let (_, repo) = fresh_repo();
        let alpha = doc("notes/alpha.md");
        let beta = doc("notes/beta.md");
        repo.write_document(&alpha, b"alpha document text")
            .await
            .unwrap();
        repo.write_document(&beta, b"beta document text")
            .await
            .unwrap();
        repo.replace_document_chunks_if_current(
            &alpha,
            &content_sha256("alpha document text"),
            &[MemoryChunkWrite {
                content: "alpha document text".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();
        repo.replace_document_chunks_if_current(
            &beta,
            &content_sha256("beta document text"),
            &[MemoryChunkWrite {
                content: "beta document text".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();

        let request = search_request("alpha").with_vector(false);
        let results = repo
            .search_documents(alpha.scope(), &request)
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "FTS must filter out non-matching chunks");
        assert_eq!(results[0].path.relative_path(), "notes/alpha.md");
        assert!(results[0].full_text_rank.is_some());
        assert!(results[0].vector_rank.is_none());
    }

    #[tokio::test]
    async fn search_fuses_full_text_and_vector_branches_when_both_match() {
        let (_, repo) = fresh_repo();
        let alpha = doc("notes/alpha.md");
        let beta = doc("notes/beta.md");
        repo.write_document(&alpha, b"alpha").await.unwrap();
        repo.write_document(&beta, b"beta").await.unwrap();
        // Chunks with explicit embeddings so vector ranking is
        // deterministic. Query embedding is closest to alpha's vector.
        repo.replace_document_chunks_if_current(
            &alpha,
            &content_sha256("alpha"),
            &[MemoryChunkWrite {
                content: "alpha".to_string(),
                embedding: Some(vec![1.0, 0.0, 0.0]),
            }],
        )
        .await
        .unwrap();
        repo.replace_document_chunks_if_current(
            &beta,
            &content_sha256("beta"),
            &[MemoryChunkWrite {
                content: "beta".to_string(),
                embedding: Some(vec![0.0, 1.0, 0.0]),
            }],
        )
        .await
        .unwrap();

        let request = search_request("alpha").with_query_embedding(vec![0.99, 0.01, 0.0]);
        let results = repo
            .search_documents(alpha.scope(), &request)
            .await
            .unwrap();
        let alpha_result = results
            .iter()
            .find(|r| r.path.relative_path() == "notes/alpha.md")
            .expect("alpha must rank");
        assert!(alpha_result.is_hybrid(), "alpha should fuse FTS + vector");
    }

    #[tokio::test]
    async fn chunks_under_doc_subtree_do_not_block_document_writes() {
        // Defensive check: even though kind filtering keeps user docs
        // and chunks logically separated, the underlying backend's
        // directory-overlap check could otherwise reject a write that
        // shares a path prefix with a sidecar. Our `.chunks` /
        // `.versions` suffix ensures the prefix-check doesn't fire.
        let (_, repo) = fresh_repo();
        let path = doc("notes/coexist.md");
        repo.write_document(&path, b"v1").await.unwrap();
        repo.replace_document_chunks_if_current(
            &path,
            &content_sha256("v1"),
            &[MemoryChunkWrite {
                content: "v1".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();
        // A second write must still succeed despite an existing
        // chunks sibling at `<doc>.chunks/0`.
        repo.write_document(&path, b"v2").await.unwrap();
        assert_eq!(
            repo.read_document(&path).await.unwrap().as_deref(),
            Some(b"v2".as_slice())
        );
    }

    #[tokio::test]
    async fn list_documents_filters_out_chunk_and_version_siblings() {
        let (_, repo) = fresh_repo();
        let path = doc("notes/just-one.md");
        repo.write_document(&path, b"a").await.unwrap();
        repo.write_document(&path, b"b").await.unwrap(); // produces .versions/1
        repo.replace_document_chunks_if_current(
            &path,
            &content_sha256("b"),
            &[MemoryChunkWrite {
                content: "b".to_string(),
                embedding: None,
            }],
        )
        .await
        .unwrap();
        let listed = repo.list_documents(path.scope()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].relative_path(), "notes/just-one.md");
    }
}
