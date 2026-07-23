//! Host/kernel-facing product surface contract.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{ActivityId, AgentId, CapabilityId, ProjectId, TenantId, UserId};

/// Host-stable product operation identifiers used with
/// [`ProductSurfaceInvokeRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurfaceOperation {
    ChannelInboundAdmit,
    CreateThread,
    SubmitTurn,
    CancelRun,
    ResolveGate,
    RetryRun,
    ProjectCreate,
    ProjectFsRead,
    FsRead,
    AttachmentRead,
    TraceAccountLoginLink,
    TraceHoldAuthorize,
    OperatorConfigSetKey,
    OperatorServiceLifecycle,
    LlmTestConnection,
    LlmListModels,
    LlmNearAiLogin,
    LlmNearAiWalletLogin,
    LlmCodexLogin,
    AdminUserCreate,
    AdminUserDeleteSecret,
    AutomationPause,
    AutomationResume,
    AutomationRename,
    AutomationDelete,
}

impl ProductSurfaceOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChannelInboundAdmit => "channel.admit_inbound",
            Self::CreateThread => "webui.create_thread",
            Self::SubmitTurn => "webui.submit_turn",
            Self::CancelRun => "webui.cancel_run",
            Self::ResolveGate => "webui.resolve_gate",
            Self::RetryRun => "webui.retry_run",
            Self::ProjectCreate => "project.create",
            Self::ProjectFsRead => "project.fs.read",
            Self::FsRead => "fs.read",
            Self::AttachmentRead => "attachment.read",
            Self::TraceAccountLoginLink => "trace.account_login_link",
            Self::TraceHoldAuthorize => "trace.hold_authorize",
            Self::OperatorConfigSetKey => "operator.config.set_key",
            Self::OperatorServiceLifecycle => "operator.service.lifecycle",
            Self::LlmTestConnection => "llm.test_connection",
            Self::LlmListModels => "llm.list_models",
            Self::LlmNearAiLogin => "llm.nearai.login",
            Self::LlmNearAiWalletLogin => "llm.nearai.wallet_login",
            Self::LlmCodexLogin => "llm.codex.login",
            Self::AdminUserCreate => "admin.user.create",
            Self::AdminUserDeleteSecret => "admin.user.delete_secret",
            Self::AutomationPause => "automation.pause",
            Self::AutomationResume => "automation.resume",
            Self::AutomationRename => "automation.rename",
            Self::AutomationDelete => "automation.delete",
        }
    }

    pub fn capability_id(self) -> Result<CapabilityId, crate::HostApiError> {
        CapabilityId::new(self.as_str()).map_err(|_| crate::HostApiError::InvalidId {
            kind: "product_surface_operation",
            value: self.as_str().to_string(),
            reason: "built-in product surface operation id is invalid".to_string(),
        })
    }
}

/// Authenticated product-surface caller stamped by a trusted terminal boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSurfaceCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

impl ProductSurfaceCaller {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }
}

/// Generic product mutation request. The operation id names a host-stable
/// product capability; authority comes from the trusted caller, not from the
/// payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceInvokeRequest {
    pub caller: ProductSurfaceCaller,
    pub operation_id: CapabilityId,
    pub input: serde_json::Value,
    pub activity_id: ActivityId,
}

/// Generic product mutation response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceInvokeResponse {
    pub output: serde_json::Value,
}

/// Generic read-only product view request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceQueryRequest {
    pub caller: ProductSurfaceCaller,
    pub view_id: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

/// Generic read-only product view page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceQueryPage {
    pub items: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Generic product event stream request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceStreamRequest {
    pub caller: ProductSurfaceCaller,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_cursor: Option<String>,
}

/// Generic product event stream response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceStreamResponse {
    pub events: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurfaceErrorCode {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    Unavailable,
    Internal,
}

/// Stable product-surface error family for terminal rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurfaceErrorKind {
    Validation,
    Duplicate,
    Busy,
    ParticipantDenied,
    BlockedApproval,
    BlockedAuthentication,
    BlockedResource,
    ReplayUnavailable,
    TimelineUnavailable,
    ServiceUnavailable,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurfaceValidationCode {
    MissingField,
    Blank,
    TooLong,
    InvalidId,
    InvalidControlCharacter,
    InvalidValue,
    UnknownKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("product surface error: {code:?}")]
pub struct ProductSurfaceError {
    pub code: ProductSurfaceErrorCode,
    pub kind: ProductSurfaceErrorKind,
    pub status_code: u16,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_code: Option<ProductSurfaceValidationCode>,
}

impl ProductSurfaceError {
    pub fn from_status(code: ProductSurfaceErrorCode, status_code: u16, retryable: bool) -> Self {
        Self::from_status_kind(code, default_kind_for_code(code), status_code, retryable)
    }

    pub fn from_status_kind(
        code: ProductSurfaceErrorCode,
        kind: ProductSurfaceErrorKind,
        status_code: u16,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            kind,
            status_code,
            retryable,
            field: None,
            validation_code: None,
        }
    }

    pub fn validation(
        field: impl Into<String>,
        validation_code: ProductSurfaceValidationCode,
    ) -> Self {
        Self {
            code: ProductSurfaceErrorCode::InvalidRequest,
            kind: ProductSurfaceErrorKind::Validation,
            status_code: 400,
            retryable: false,
            field: Some(field.into()),
            validation_code: Some(validation_code),
        }
    }

    pub fn unavailable(retryable: bool) -> Self {
        Self::from_status_kind(
            ProductSurfaceErrorCode::Unavailable,
            ProductSurfaceErrorKind::ServiceUnavailable,
            503,
            retryable,
        )
    }

    pub fn service_unavailable(retryable: bool) -> Self {
        Self::unavailable(retryable)
    }

    pub fn not_found() -> Self {
        Self::from_status(ProductSurfaceErrorCode::NotFound, 404, false)
    }

    pub fn internal() -> Self {
        Self::from_status(ProductSurfaceErrorCode::Internal, 500, false)
    }

    pub fn internal_invariant() -> Self {
        Self::internal()
    }

    pub fn internal_from(source: impl std::fmt::Display) -> Self {
        tracing::error!(error = %source, "internal product surface error");
        Self::internal()
    }
}

fn default_kind_for_code(code: ProductSurfaceErrorCode) -> ProductSurfaceErrorKind {
    match code {
        ProductSurfaceErrorCode::InvalidRequest => ProductSurfaceErrorKind::Validation,
        ProductSurfaceErrorCode::Unauthenticated | ProductSurfaceErrorCode::Forbidden => {
            ProductSurfaceErrorKind::ParticipantDenied
        }
        ProductSurfaceErrorCode::NotFound => ProductSurfaceErrorKind::NotFound,
        ProductSurfaceErrorCode::Conflict => ProductSurfaceErrorKind::Conflict,
        ProductSurfaceErrorCode::RateLimited => ProductSurfaceErrorKind::Busy,
        ProductSurfaceErrorCode::Unavailable => ProductSurfaceErrorKind::ServiceUnavailable,
        ProductSurfaceErrorCode::Internal => ProductSurfaceErrorKind::Internal,
    }
}

/// Stable product surface exposed by host kernels to product terminals.
#[async_trait]
pub trait ProductSurface: Send + Sync {
    async fn invoke(
        &self,
        request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError>;

    async fn query(
        &self,
        request: ProductSurfaceQueryRequest,
    ) -> Result<ProductSurfaceQueryPage, ProductSurfaceError>;

    async fn stream_events(
        &self,
        request: ProductSurfaceStreamRequest,
    ) -> Result<ProductSurfaceStreamResponse, ProductSurfaceError>;
}
