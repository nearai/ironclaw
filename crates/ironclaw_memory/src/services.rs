//! Focused Reborn memory service contracts.
//!
//! These traits are the product-facing service vocabulary above repository
//! backends. They intentionally carry resolved memory scope plus legacy-relative
//! paths; product callers must not construct scoped `/memory/tenants/...` paths.

use async_trait::async_trait;
use ironclaw_host_api::HostApiError;

use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope};
use crate::safety::{PromptProtectedPathClass, PromptWriteOperation};

/// Stable service error code returned by Reborn memory service facades.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryServiceErrorCode {
    InvalidRequest,
    NotFound,
    AccessDenied,
    Conflict,
    Unavailable,
    PromptWriteRejected,
    Internal,
}

impl MemoryServiceErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::AccessDenied => "access_denied",
            Self::Conflict => "conflict",
            Self::Unavailable => "unavailable",
            Self::PromptWriteRejected => "prompt_write_rejected",
            Self::Internal => "internal",
        }
    }
}

/// Sanitized error returned by focused memory services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceError {
    code: MemoryServiceErrorCode,
    message: String,
}

impl MemoryServiceError {
    pub fn new(code: MemoryServiceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: sanitize_service_message(message.into()),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(MemoryServiceErrorCode::InvalidRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(MemoryServiceErrorCode::NotFound, message)
    }

    pub fn prompt_write_rejected(message: impl Into<String>) -> Self {
        Self::new(MemoryServiceErrorCode::PromptWriteRejected, message)
    }

    pub fn code(&self) -> MemoryServiceErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for MemoryServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for MemoryServiceError {}

impl From<HostApiError> for MemoryServiceError {
    fn from(value: HostApiError) -> Self {
        Self::invalid_request(value.to_string())
    }
}

const MEMORY_SERVICE_DETAIL_MARKERS: &[&str] = &[
    "no such table",
    "drop table",
    "sql",
    "sqlite",
    "libsql",
    "postgres",
    "provider",
    "api key",
    "secret",
    "token",
    "/tmp/",
    "/private/",
    "/var/folders/",
    "\\appdata\\",
];

fn sanitize_service_message(message: String) -> String {
    let lower = message.to_ascii_lowercase();
    if MEMORY_SERVICE_DETAIL_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        "memory service operation failed".to_string()
    } else {
        message
    }
}

/// Product actor that caused a memory write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryWriteActor {
    User { user_id: String },
    Agent { agent_id: String },
    Admin { user_id: String },
    Tool { tool_name: String },
}

/// Product surface that caused a memory write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWriteSurface {
    Cli,
    Web,
    LlmTool,
    SetupAdmin,
    PromptContext,
}

/// Product intent for a memory write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWritePurpose {
    Memory,
    DailyLog,
    Heartbeat,
    Bootstrap,
    CustomPath,
    Metadata,
    LayerWrite,
    ProfileSync,
    Seed,
}

/// Actor/surface/purpose authority that must flow into prompt-write policy and audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWriteAuthority {
    pub actor: MemoryWriteActor,
    pub surface: MemoryWriteSurface,
    pub purpose: MemoryWritePurpose,
}

impl MemoryWriteAuthority {
    pub fn new(
        actor: MemoryWriteActor,
        surface: MemoryWriteSurface,
        purpose: MemoryWritePurpose,
    ) -> Self {
        Self {
            actor,
            surface,
            purpose,
        }
    }
}

/// Current memory document content returned by document reads.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryDocumentRecord {
    pub path: MemoryDocumentPath,
    pub content: String,
    pub metadata: serde_json::Value,
}

/// Summary entry returned by list/tree operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDocumentEntry {
    pub relative_path: String,
    pub is_directory: bool,
}

/// Memory workspace status DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStatus {
    pub document_count: usize,
    pub indexed_document_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReadDocumentRequest {
    pub path: MemoryDocumentPath,
    pub primary_scope_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryWriteDocumentRequest {
    pub path: MemoryDocumentPath,
    pub content: String,
    pub options: MemoryWriteOptions,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryAppendDocumentRequest {
    pub path: MemoryDocumentPath,
    pub content: String,
    pub options: MemoryWriteOptions,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPatchDocumentRequest {
    pub path: MemoryDocumentPath,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPatchDocumentOutcome {
    pub replacements: usize,
    pub content_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWriteDocumentOutcome {
    pub relative_path: String,
    pub content_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryListDocumentsRequest {
    pub scope: MemoryDocumentScope,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTreeRequest {
    pub scope: MemoryDocumentScope,
    pub root: Option<String>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStatusRequest {
    pub scope: MemoryDocumentScope,
}

#[async_trait]
pub trait MemoryDocumentService: Send + Sync {
    async fn read_current(
        &self,
        request: MemoryReadDocumentRequest,
    ) -> Result<MemoryDocumentRecord, MemoryServiceError>;

    async fn write(
        &self,
        request: MemoryWriteDocumentRequest,
    ) -> Result<MemoryWriteDocumentOutcome, MemoryServiceError>;

    async fn append(
        &self,
        request: MemoryAppendDocumentRequest,
    ) -> Result<MemoryWriteDocumentOutcome, MemoryServiceError>;

    async fn patch(
        &self,
        request: MemoryPatchDocumentRequest,
    ) -> Result<MemoryPatchDocumentOutcome, MemoryServiceError>;

    async fn list(
        &self,
        request: MemoryListDocumentsRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryServiceError>;

    async fn tree(
        &self,
        request: MemoryTreeRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryServiceError>;

    async fn status(
        &self,
        request: MemoryStatusRequest,
    ) -> Result<MemoryStatus, MemoryServiceError>;
}

/// Product search request. Query embedding generation belongs inside the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductSearchRequest {
    pub scope: MemoryDocumentScope,
    pub query: String,
    pub limit: usize,
    pub secondary_scopes: Vec<MemoryDocumentScope>,
    pub exclude_identity_documents_from_secondary: bool,
    pub group_context: Option<MemorySearchGroupContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySearchGroupContext {
    pub conversation_id: String,
    pub personal_memory_allowed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryProductSearchHit {
    pub relative_path: String,
    pub content: String,
    pub score: f32,
}

#[async_trait]
pub trait MemorySearchService: Send + Sync {
    async fn search(
        &self,
        request: MemoryProductSearchRequest,
    ) -> Result<Vec<MemoryProductSearchHit>, MemoryServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryLayerWriteMode {
    Append,
    Replace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryLayerWriteRequest {
    pub path: MemoryDocumentPath,
    pub layer_name: String,
    pub content: String,
    pub mode: MemoryLayerWriteMode,
    pub force: bool,
    pub options: MemoryWriteOptions,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLayerWriteOutcome {
    pub relative_path: String,
    pub actual_layer: String,
    pub redirected: bool,
}

#[async_trait]
pub trait MemoryLayerService: Send + Sync {
    async fn write_layer(
        &self,
        request: MemoryLayerWriteRequest,
    ) -> Result<MemoryLayerWriteOutcome, MemoryServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryVersionListRequest {
    pub path: MemoryDocumentPath,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryVersionReadRequest {
    pub path: MemoryDocumentPath,
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryVersionSummary {
    pub version: i32,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryVersionRecord {
    pub path: MemoryDocumentPath,
    pub version: i32,
    pub content: String,
    pub content_hash: String,
}

#[async_trait]
pub trait MemoryVersionService: Send + Sync {
    async fn list_versions(
        &self,
        request: MemoryVersionListRequest,
    ) -> Result<Vec<MemoryVersionSummary>, MemoryServiceError>;

    async fn read_version(
        &self,
        request: MemoryVersionReadRequest,
    ) -> Result<MemoryVersionRecord, MemoryServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBootstrapClearRequest {
    pub scope: MemoryDocumentScope,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBootstrapClearOutcome {
    pub relative_path: String,
}

#[async_trait]
pub trait MemorySeedService: Send + Sync {
    async fn clear_bootstrap(
        &self,
        request: MemoryBootstrapClearRequest,
    ) -> Result<MemoryBootstrapClearOutcome, MemoryServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProfileSyncRequest {
    pub scope: MemoryDocumentScope,
    pub profile_path: MemoryDocumentPath,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProfileSyncOutcome {
    pub synced_relative_paths: Vec<String>,
}

#[async_trait]
pub trait MemoryProfileService: Send + Sync {
    async fn sync_profile_documents(
        &self,
        request: MemoryProfileSyncRequest,
    ) -> Result<MemoryProfileSyncOutcome, MemoryServiceError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryPromptWriteSafetyRequest {
    pub path: MemoryDocumentPath,
    pub operation: PromptWriteOperation,
    pub protected_path_class: PromptProtectedPathClass,
    pub content: String,
    pub authority: MemoryWriteAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPromptWriteSafetyDecision {
    Allow,
    Reject { reason: String },
}

#[async_trait]
pub trait MemoryPromptWriteSafetyPolicy: Send + Sync {
    async fn check_product_write(
        &self,
        request: MemoryPromptWriteSafetyRequest,
    ) -> Result<MemoryPromptWriteSafetyDecision, MemoryServiceError>;
}
