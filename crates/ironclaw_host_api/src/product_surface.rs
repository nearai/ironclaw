//! Host/kernel-facing product surface contract.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    ActivityId, AdapterInstallationId, AgentId, CapabilityId, ChannelInboundClassification,
    NormalizedInboundMessage, ProductAdapterError, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductSourceChannel, ProjectId, ProtocolAuthEvidence, RedactedString,
    TenantId, UserId,
};

/// One verified, normalized channel message admitted through a product surface.
///
/// The channel ingress router verifies the transport request and runs the
/// adapter's pure normalization first. Product workflow owns conversion into
/// the durable inbound envelope and commit path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInboundSurfaceRequest {
    pub adapter_id: ProductAdapterId,
    pub source_channel: ProductSourceChannel,
    pub installation_id: AdapterInstallationId,
    pub evidence: ProtocolAuthEvidence,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub message: NormalizedInboundMessage,
    pub classification: Option<ChannelInboundClassification>,
}

/// Durable channel admission evidence returned by product workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInboundSurfaceAdmission {
    pub envelope: ProductInboundEnvelope,
    pub ack: ProductInboundAck,
}

/// Admission rejection after product workflow had enough trusted input to build
/// the canonical envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInboundSurfaceRejectedAdmission {
    pub envelope: ProductInboundEnvelope,
    pub error: ProductAdapterError,
}

/// Channel admission outcome returned by the host/product channel ingress door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInboundSurfaceOutcome {
    Admitted(Box<ChannelInboundSurfaceAdmission>),
    Invalid(ProductAdapterError),
    Rejected(Box<ChannelInboundSurfaceRejectedAdmission>),
}

impl ChannelInboundSurfaceOutcome {
    pub fn unavailable() -> Self {
        Self::Invalid(ProductAdapterError::Internal {
            detail: RedactedString::new("channel product surface admission is not available"),
        })
    }
}

/// Typed admission door for extension/channel ingress.
#[async_trait]
pub trait ChannelInboundProductSurface: Send + Sync {
    async fn admit_channel_inbound(
        &self,
        request: ChannelInboundSurfaceRequest,
    ) -> ChannelInboundSurfaceOutcome;
}

/// Authenticated product-surface caller stamped by a trusted terminal boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSurfaceCaller {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub operator_config: bool,
}

impl ProductSurfaceCaller {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
            operator_config: false,
        }
    }

    pub fn with_operator_config(mut self, operator_config: bool) -> Self {
        self.operator_config = operator_config;
        self
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Generic product mutation request. The operation id names a host-stable
/// product capability; authority comes from the trusted caller, not from the
/// payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductSurfaceInvokeRequest {
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

/// Stable product surface exposed by host composition.
#[async_trait]
pub trait ProductSurface: Send + Sync {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError>;

    async fn query(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceQueryRequest,
    ) -> Result<ProductSurfaceQueryPage, ProductSurfaceError>;

    async fn stream_events(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceStreamRequest,
    ) -> Result<ProductSurfaceStreamResponse, ProductSurfaceError>;
}

/// Product surface bound to one authenticated caller at a trusted edge.
///
/// Route/channel consumers pass this handle inward so operation request DTOs do
/// not carry authority-bearing caller data.
#[derive(Clone)]
pub struct BoundProductSurface {
    surface: Arc<dyn ProductSurface>,
    caller: ProductSurfaceCaller,
}

impl BoundProductSurface {
    pub fn new(surface: Arc<dyn ProductSurface>, caller: ProductSurfaceCaller) -> Self {
        Self { surface, caller }
    }

    pub fn caller(&self) -> &ProductSurfaceCaller {
        &self.caller
    }

    pub async fn invoke(
        &self,
        request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError> {
        self.surface.invoke(self.caller.clone(), request).await
    }

    pub async fn query(
        &self,
        request: ProductSurfaceQueryRequest,
    ) -> Result<ProductSurfaceQueryPage, ProductSurfaceError> {
        self.surface.query(self.caller.clone(), request).await
    }

    pub async fn stream_events(
        &self,
        request: ProductSurfaceStreamRequest,
    ) -> Result<ProductSurfaceStreamResponse, ProductSurfaceError> {
        self.surface
            .stream_events(self.caller.clone(), request)
            .await
    }
}
