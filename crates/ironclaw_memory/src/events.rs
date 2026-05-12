//! Metadata-only memory significant-event seam.
//!
//! Producer services emit these facts after memory side effects complete. The
//! payload deliberately excludes raw document bytes, raw search queries, raw
//! host paths, and layer/path names; downstream adapters can project stable
//! metadata through durable audit/event logs without becoming memory backends.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CorrelationId, ResourceScope};

use crate::chunking::content_sha256;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope};
use crate::search::MemorySearchRequest;

/// Redacted caller/audit context attached to memory events when the caller has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAuditContext {
    pub resource_scope: ResourceScope,
    pub correlation_id: CorrelationId,
}

impl MemoryAuditContext {
    pub fn new(resource_scope: ResourceScope, correlation_id: CorrelationId) -> Self {
        Self {
            resource_scope,
            correlation_id,
        }
    }
}

/// Error returned by host-composed memory event sinks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEventSinkError {
    reason: String,
}

impl MemoryEventSinkError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl std::fmt::Display for MemoryEventSinkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for MemoryEventSinkError {}

/// Significant memory fact class emitted by memory services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySignificantEventKind {
    DocumentWritten,
    DocumentDeleted,
    DocumentIndexed,
    SearchPerformed,
    LayerRedirected,
}

/// Public caller surface that produced the memory fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySignificantEventSource {
    RepositoryMemoryBackend,
    MemoryDocumentFilesystem,
    ChunkingMemoryDocumentIndexer,
}

/// Stable metadata-only status for a memory fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySignificantEventStatus {
    Written,
    Deleted,
    Indexed,
    Performed,
    Redirected,
}

impl MemorySignificantEventStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::Deleted => "deleted",
            Self::Indexed => "indexed",
            Self::Performed => "performed",
            Self::Redirected => "redirected",
        }
    }
}

/// Redacted metadata-only memory event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySignificantEvent {
    pub kind: MemorySignificantEventKind,
    pub source: MemorySignificantEventSource,
    pub status: MemorySignificantEventStatus,
    pub scope: MemoryDocumentScope,
    /// SHA-256 of the memory-relative path when the event is document-scoped.
    /// This gives correlatable metadata without exposing path text.
    pub relative_path_hash: Option<String>,
    /// Number of bytes accepted by the memory write operation.
    pub byte_count: Option<u64>,
    /// Number of chunks written by indexing, when known.
    pub chunk_count: Option<u64>,
    /// Number of search results returned, when known.
    pub result_count: Option<u64>,
    pub full_text: Option<bool>,
    pub vector: Option<bool>,
    pub audit_context: Option<MemoryAuditContext>,
}

impl MemorySignificantEvent {
    pub fn document_written(
        path: &MemoryDocumentPath,
        source: MemorySignificantEventSource,
        byte_count: u64,
    ) -> Self {
        Self {
            kind: MemorySignificantEventKind::DocumentWritten,
            source,
            status: MemorySignificantEventStatus::Written,
            scope: path.scope().clone(),
            relative_path_hash: Some(content_sha256(path.relative_path())),
            byte_count: Some(byte_count),
            chunk_count: None,
            result_count: None,
            full_text: None,
            vector: None,
            audit_context: None,
        }
    }

    pub fn document_indexed(
        path: &MemoryDocumentPath,
        source: MemorySignificantEventSource,
        chunk_count: u64,
    ) -> Self {
        Self {
            kind: MemorySignificantEventKind::DocumentIndexed,
            source,
            status: MemorySignificantEventStatus::Indexed,
            scope: path.scope().clone(),
            relative_path_hash: Some(content_sha256(path.relative_path())),
            byte_count: None,
            chunk_count: Some(chunk_count),
            result_count: None,
            full_text: None,
            vector: None,
            audit_context: None,
        }
    }

    pub fn search_performed(
        scope: &MemoryDocumentScope,
        source: MemorySignificantEventSource,
        request: &MemorySearchRequest,
        result_count: u64,
    ) -> Self {
        Self {
            kind: MemorySignificantEventKind::SearchPerformed,
            source,
            status: MemorySignificantEventStatus::Performed,
            scope: scope.clone(),
            relative_path_hash: None,
            byte_count: None,
            chunk_count: None,
            result_count: Some(result_count),
            full_text: Some(request.full_text()),
            vector: Some(request.vector()),
            audit_context: None,
        }
    }

    pub fn with_audit_context(mut self, audit_context: Option<&MemoryAuditContext>) -> Self {
        self.audit_context = audit_context.cloned();
        self
    }
}

/// Host-composed sink for durable redacted memory significant events.
#[async_trait]
pub trait MemorySignificantEventSink: Send + Sync {
    async fn record_memory_significant_event(
        &self,
        event: MemorySignificantEvent,
    ) -> Result<(), MemoryEventSinkError>;
}

pub(crate) async fn record_memory_significant_event(
    sink: Option<&Arc<dyn MemorySignificantEventSink>>,
    event: MemorySignificantEvent,
) {
    let Some(sink) = sink else {
        return;
    };
    if let Err(error) = sink.record_memory_significant_event(event).await {
        tracing::debug!(error = %error, "memory significant-event sink failed");
    }
}
