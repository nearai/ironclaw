//! Product-facing memory facade for the #3287 first slice.
//!
//! This module is deliberately a thin adapter boundary. It resolves product
//! targets and relative paths, carries actor/surface/purpose authority into
//! memory service DTOs, and leaves storage/search/provider execution to focused
//! `ironclaw_memory` services.

use std::sync::Arc;

use ironclaw_memory::{
    DocumentMetadata, MemoryAppendDocumentRequest, MemoryBootstrapClearRequest,
    MemoryDocumentEntry, MemoryDocumentPath, MemoryDocumentRecord, MemoryDocumentScope,
    MemoryDocumentService, MemoryLayerService, MemoryLayerWriteMode, MemoryLayerWriteRequest,
    MemoryListDocumentsRequest, MemoryPatchDocumentRequest, MemoryProductSearchHit,
    MemoryProfileService, MemoryProfileSyncRequest, MemoryPromptWriteSafetyDecision,
    MemoryPromptWriteSafetyPolicy, MemoryPromptWriteSafetyRequest, MemoryReadDocumentRequest,
    MemorySearchGroupContext, MemorySearchService, MemorySeedService, MemoryServiceError,
    MemoryStatus, MemoryStatusRequest, MemoryTreeRequest, MemoryVersionListRequest,
    MemoryVersionReadRequest, MemoryVersionRecord, MemoryVersionService, MemoryVersionSummary,
    MemoryWriteActor, MemoryWriteAuthority, MemoryWriteDocumentRequest, MemoryWriteOptions,
    MemoryWritePurpose, MemoryWriteSurface, PromptProtectedPathRegistry, PromptWriteOperation,
};
use thiserror::Error;

const MEMORY_PATH: &str = "MEMORY.md";
const HEARTBEAT_PATH: &str = "HEARTBEAT.md";
const BOOTSTRAP_PATH: &str = "BOOTSTRAP.md";
const PROFILE_PATH: &str = "context/profile.json";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryProductError {
    #[error("invalid memory product request: {reason}")]
    InvalidRequest { reason: String },
    #[error("memory prompt write rejected: {reason}")]
    PromptWriteRejected { reason: String },
    #[error("memory service failed: {source}")]
    Service { source: MemoryServiceError },
}

impl From<MemoryServiceError> for MemoryProductError {
    fn from(source: MemoryServiceError) -> Self {
        Self::Service { source }
    }
}

impl From<ironclaw_host_api::HostApiError> for MemoryProductError {
    fn from(value: ironclaw_host_api::HostApiError) -> Self {
        Self::InvalidRequest {
            reason: value.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryProductWriteTarget {
    Memory,
    DailyLog { local_date: String },
    Heartbeat,
    Bootstrap,
    CustomPath(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryProductWriteMode {
    Append,
    Replace,
    Patch {
        old_string: String,
        new_string: String,
        replace_all: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryProductWriteRequest {
    pub scope: MemoryDocumentScope,
    pub target: MemoryProductWriteTarget,
    pub mode: MemoryProductWriteMode,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub layer_name: Option<String>,
    pub force: bool,
    pub actor: MemoryWriteActor,
    pub surface: MemoryWriteSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductWriteResponse {
    pub relative_path: String,
    pub status: String,
    pub content_length: usize,
    pub actual_layer: Option<String>,
    pub redirected: Option<bool>,
    pub replacements: Option<usize>,
    pub synced_relative_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryProductReadMode {
    Current,
    ListVersions { limit: usize },
    Version { version: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductReadRequest {
    pub scope: MemoryDocumentScope,
    pub relative_path: String,
    pub mode: MemoryProductReadMode,
    pub primary_scope_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemoryProductReadResponse {
    Current(MemoryDocumentRecord),
    Versions(Vec<MemoryVersionSummary>),
    Version(MemoryVersionRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductSearchRequest {
    pub scope: MemoryDocumentScope,
    pub query: String,
    pub limit: usize,
    pub secondary_scopes: Vec<MemoryDocumentScope>,
    pub group_context: Option<MemorySearchGroupContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductListRequest {
    pub scope: MemoryDocumentScope,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductTreeRequest {
    pub scope: MemoryDocumentScope,
    pub root: Option<String>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProductStatusRequest {
    pub scope: MemoryDocumentScope,
}

pub struct MemoryProductServices {
    pub document_service: Arc<dyn MemoryDocumentService>,
    pub search_service: Arc<dyn MemorySearchService>,
    pub layer_service: Arc<dyn MemoryLayerService>,
    pub version_service: Arc<dyn MemoryVersionService>,
    pub seed_service: Arc<dyn MemorySeedService>,
    pub profile_service: Arc<dyn MemoryProfileService>,
    pub prompt_write_policy: Arc<dyn MemoryPromptWriteSafetyPolicy>,
}

pub struct MemoryProductFacade {
    document_service: Arc<dyn MemoryDocumentService>,
    search_service: Arc<dyn MemorySearchService>,
    layer_service: Arc<dyn MemoryLayerService>,
    version_service: Arc<dyn MemoryVersionService>,
    seed_service: Arc<dyn MemorySeedService>,
    profile_service: Arc<dyn MemoryProfileService>,
    prompt_write_policy: Arc<dyn MemoryPromptWriteSafetyPolicy>,
    protected_paths: PromptProtectedPathRegistry,
}

impl MemoryProductFacade {
    pub fn new(services: MemoryProductServices) -> Self {
        Self {
            document_service: services.document_service,
            search_service: services.search_service,
            layer_service: services.layer_service,
            version_service: services.version_service,
            seed_service: services.seed_service,
            profile_service: services.profile_service,
            prompt_write_policy: services.prompt_write_policy,
            protected_paths: PromptProtectedPathRegistry::default(),
        }
    }

    pub async fn write(
        &self,
        request: MemoryProductWriteRequest,
    ) -> Result<MemoryProductWriteResponse, MemoryProductError> {
        let (relative_path, purpose) = resolve_write_target(&request.target)?;
        let purpose = if request.layer_name.is_some() {
            MemoryWritePurpose::LayerWrite
        } else {
            purpose
        };
        validate_write_mode(&request)?;
        let authority = MemoryWriteAuthority::new(request.actor.clone(), request.surface, purpose);
        let path = document_path(&request.scope, &relative_path)?;
        let operation = operation_for_mode(&request.mode);
        let prompt_content = prompt_check_content(&request);

        self.check_prompt_write(path.clone(), operation, prompt_content, authority.clone())
            .await?;

        if matches!(request.target, MemoryProductWriteTarget::Bootstrap) {
            let outcome = self
                .seed_service
                .clear_bootstrap(MemoryBootstrapClearRequest {
                    scope: request.scope,
                    authority,
                })
                .await?;
            return Ok(MemoryProductWriteResponse {
                relative_path: outcome.relative_path,
                status: "cleared".to_string(),
                content_length: 0,
                actual_layer: None,
                redirected: None,
                replacements: None,
                synced_relative_paths: Vec::new(),
            });
        }

        let options = write_options(request.metadata.as_ref(), &authority);
        if let Some(layer_name) = request.layer_name {
            let mode = match request.mode {
                MemoryProductWriteMode::Append => MemoryLayerWriteMode::Append,
                MemoryProductWriteMode::Replace => MemoryLayerWriteMode::Replace,
                MemoryProductWriteMode::Patch { .. } => unreachable!("validated above"),
            };
            let outcome = self
                .layer_service
                .write_layer(MemoryLayerWriteRequest {
                    path,
                    layer_name,
                    content: request.content,
                    mode,
                    force: request.force,
                    options,
                    authority,
                })
                .await?;
            return Ok(MemoryProductWriteResponse {
                relative_path: outcome.relative_path,
                status: "written".to_string(),
                content_length: 0,
                actual_layer: Some(outcome.actual_layer),
                redirected: Some(outcome.redirected),
                replacements: None,
                synced_relative_paths: Vec::new(),
            });
        }

        let mut synced_relative_paths = Vec::new();
        let (status, content_length, replacements) = match request.mode {
            MemoryProductWriteMode::Append => {
                let outcome = self
                    .document_service
                    .append(MemoryAppendDocumentRequest {
                        path: path.clone(),
                        content: request.content,
                        options,
                        authority: authority.clone(),
                    })
                    .await?;
                ("written".to_string(), outcome.content_length, None)
            }
            MemoryProductWriteMode::Replace => {
                let outcome = self
                    .document_service
                    .write(MemoryWriteDocumentRequest {
                        path: path.clone(),
                        content: request.content,
                        options,
                        authority: authority.clone(),
                    })
                    .await?;
                ("written".to_string(), outcome.content_length, None)
            }
            MemoryProductWriteMode::Patch {
                old_string,
                new_string,
                replace_all,
            } => {
                if old_string.is_empty() {
                    return Err(MemoryProductError::InvalidRequest {
                        reason: "old_string cannot be empty".to_string(),
                    });
                }
                let outcome = self
                    .document_service
                    .patch(MemoryPatchDocumentRequest {
                        path: path.clone(),
                        old_string,
                        new_string,
                        replace_all,
                        authority: authority.clone(),
                    })
                    .await?;
                (
                    "patched".to_string(),
                    outcome.content_length,
                    Some(outcome.replacements),
                )
            }
        };

        if path.relative_path() == PROFILE_PATH {
            let outcome = self
                .profile_service
                .sync_profile_documents(MemoryProfileSyncRequest {
                    scope: path.scope().clone(),
                    profile_path: path.clone(),
                    authority,
                })
                .await?;
            synced_relative_paths = outcome.synced_relative_paths;
        }

        Ok(MemoryProductWriteResponse {
            relative_path,
            status,
            content_length,
            actual_layer: None,
            redirected: None,
            replacements,
            synced_relative_paths,
        })
    }

    pub async fn read(
        &self,
        request: MemoryProductReadRequest,
    ) -> Result<MemoryProductReadResponse, MemoryProductError> {
        ensure_product_relative_path(&request.relative_path)?;
        if !request.primary_scope_only
            && self
                .protected_paths
                .classify_relative_path(&request.relative_path)
                .is_some()
        {
            return Err(MemoryProductError::InvalidRequest {
                reason:
                    "identity and prompt-context documents must be read from primary scope only"
                        .to_string(),
            });
        }
        let path = document_path(&request.scope, &request.relative_path)?;
        match request.mode {
            MemoryProductReadMode::Current => {
                let record = self
                    .document_service
                    .read_current(MemoryReadDocumentRequest {
                        path,
                        primary_scope_only: request.primary_scope_only,
                    })
                    .await?;
                Ok(MemoryProductReadResponse::Current(record))
            }
            MemoryProductReadMode::ListVersions { limit } => {
                let versions = self
                    .version_service
                    .list_versions(MemoryVersionListRequest { path, limit })
                    .await?;
                Ok(MemoryProductReadResponse::Versions(versions))
            }
            MemoryProductReadMode::Version { version } => {
                let record = self
                    .version_service
                    .read_version(MemoryVersionReadRequest { path, version })
                    .await?;
                Ok(MemoryProductReadResponse::Version(record))
            }
        }
    }

    pub async fn search(
        &self,
        request: MemoryProductSearchRequest,
    ) -> Result<Vec<MemoryProductSearchHit>, MemoryProductError> {
        let hits = self
            .search_service
            .search(ironclaw_memory::MemoryProductSearchRequest {
                scope: request.scope,
                query: request.query,
                limit: request.limit,
                secondary_scopes: request.secondary_scopes,
                exclude_identity_documents_from_secondary: true,
                group_context: request.group_context,
            })
            .await?;
        Ok(hits)
    }

    pub async fn list(
        &self,
        request: MemoryProductListRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryProductError> {
        validate_optional_parent(request.parent.as_deref())?;
        let entries = self
            .document_service
            .list(MemoryListDocumentsRequest {
                scope: request.scope,
                parent: request.parent,
            })
            .await?;
        Ok(entries)
    }

    pub async fn tree(
        &self,
        request: MemoryProductTreeRequest,
    ) -> Result<Vec<MemoryDocumentEntry>, MemoryProductError> {
        validate_optional_parent(request.root.as_deref())?;
        let entries = self
            .document_service
            .tree(MemoryTreeRequest {
                scope: request.scope,
                root: request.root,
                max_depth: request.max_depth,
            })
            .await?;
        Ok(entries)
    }

    pub async fn status(
        &self,
        request: MemoryProductStatusRequest,
    ) -> Result<MemoryStatus, MemoryProductError> {
        let status = self
            .document_service
            .status(MemoryStatusRequest {
                scope: request.scope,
            })
            .await?;
        Ok(status)
    }

    async fn check_prompt_write(
        &self,
        path: MemoryDocumentPath,
        operation: PromptWriteOperation,
        content: String,
        authority: MemoryWriteAuthority,
    ) -> Result<(), MemoryProductError> {
        let Some(protected_path_class) = self.protected_paths.classify_path(&path) else {
            return Ok(());
        };
        match self
            .prompt_write_policy
            .check_product_write(MemoryPromptWriteSafetyRequest {
                path,
                operation,
                protected_path_class,
                content,
                authority,
            })
            .await?
        {
            MemoryPromptWriteSafetyDecision::Allow => Ok(()),
            MemoryPromptWriteSafetyDecision::Reject { reason } => {
                Err(MemoryProductError::PromptWriteRejected { reason })
            }
        }
    }
}

fn resolve_write_target(
    target: &MemoryProductWriteTarget,
) -> Result<(String, MemoryWritePurpose), MemoryProductError> {
    match target {
        MemoryProductWriteTarget::Memory => {
            Ok((MEMORY_PATH.to_string(), MemoryWritePurpose::Memory))
        }
        MemoryProductWriteTarget::DailyLog { local_date } => {
            validate_daily_log_date(local_date)?;
            Ok((
                format!("daily/{local_date}.md"),
                MemoryWritePurpose::DailyLog,
            ))
        }
        MemoryProductWriteTarget::Heartbeat => {
            Ok((HEARTBEAT_PATH.to_string(), MemoryWritePurpose::Heartbeat))
        }
        MemoryProductWriteTarget::Bootstrap => {
            Ok((BOOTSTRAP_PATH.to_string(), MemoryWritePurpose::Bootstrap))
        }
        MemoryProductWriteTarget::CustomPath(path) => {
            ensure_product_relative_path(path)?;
            Ok((path.clone(), MemoryWritePurpose::CustomPath))
        }
    }
}

fn document_path(
    scope: &MemoryDocumentScope,
    relative_path: &str,
) -> Result<MemoryDocumentPath, MemoryProductError> {
    ensure_product_relative_path(relative_path)?;
    Ok(MemoryDocumentPath::new_with_agent(
        scope.tenant_id(),
        scope.user_id(),
        scope.agent_id(),
        scope.project_id(),
        relative_path,
    )?)
}

fn ensure_product_relative_path(path: &str) -> Result<(), MemoryProductError> {
    if path.trim() != path {
        return Err(MemoryProductError::InvalidRequest {
            reason: "memory product paths must not contain leading or trailing whitespace"
                .to_string(),
        });
    }
    if looks_like_filesystem_path(path) || path.contains("://") {
        return Err(MemoryProductError::InvalidRequest {
            reason: "memory product paths must be legacy-relative workspace paths".to_string(),
        });
    }
    Ok(())
}

fn validate_optional_parent(parent: Option<&str>) -> Result<(), MemoryProductError> {
    if let Some(parent) = parent
        && !parent.is_empty()
    {
        ensure_product_relative_path(parent)?;
    }
    Ok(())
}

fn validate_write_mode(request: &MemoryProductWriteRequest) -> Result<(), MemoryProductError> {
    if request.layer_name.is_some() && matches!(request.mode, MemoryProductWriteMode::Patch { .. })
    {
        return Err(MemoryProductError::InvalidRequest {
            reason: "patch mode cannot be combined with layer writes".to_string(),
        });
    }
    if matches!(request.target, MemoryProductWriteTarget::Bootstrap)
        && matches!(request.mode, MemoryProductWriteMode::Patch { .. })
    {
        return Err(MemoryProductError::InvalidRequest {
            reason: "bootstrap clear cannot be combined with patch mode".to_string(),
        });
    }
    Ok(())
}

fn validate_daily_log_date(local_date: &str) -> Result<(), MemoryProductError> {
    let bytes = local_date.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| matches!(idx, 4 | 7) || byte.is_ascii_digit());
    if !valid {
        return Err(MemoryProductError::InvalidRequest {
            reason: "daily log target requires a YYYY-MM-DD local date".to_string(),
        });
    }
    Ok(())
}

fn looks_like_filesystem_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with("~/") || path.contains('\\') {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn operation_for_mode(mode: &MemoryProductWriteMode) -> PromptWriteOperation {
    match mode {
        MemoryProductWriteMode::Append => PromptWriteOperation::Append,
        MemoryProductWriteMode::Replace => PromptWriteOperation::Write,
        MemoryProductWriteMode::Patch { .. } => PromptWriteOperation::Patch,
    }
}

fn prompt_check_content(request: &MemoryProductWriteRequest) -> String {
    if matches!(request.target, MemoryProductWriteTarget::Bootstrap) {
        return String::new();
    }
    match &request.mode {
        MemoryProductWriteMode::Patch { new_string, .. } => new_string.clone(),
        MemoryProductWriteMode::Append | MemoryProductWriteMode::Replace => request.content.clone(),
    }
}

fn write_options(
    metadata: Option<&serde_json::Value>,
    authority: &MemoryWriteAuthority,
) -> MemoryWriteOptions {
    MemoryWriteOptions {
        metadata: metadata
            .map(DocumentMetadata::from_value)
            .unwrap_or_default(),
        changed_by: Some(actor_label(&authority.actor)),
    }
}

fn actor_label(actor: &MemoryWriteActor) -> String {
    match actor {
        MemoryWriteActor::User { user_id } => format!("user:{user_id}"),
        MemoryWriteActor::Agent { agent_id } => format!("agent:{agent_id}"),
        MemoryWriteActor::Admin { user_id } => format!("admin:{user_id}"),
        MemoryWriteActor::Tool { tool_name } => format!("tool:{tool_name}"),
    }
}
