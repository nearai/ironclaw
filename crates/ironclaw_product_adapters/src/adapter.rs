//! ProductAdapter trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::{AuthRequirement, ProtocolAuthEvidence};
use crate::capabilities::ProductAdapterCapabilities;
use crate::egress::{DeclaredEgressTarget, OutboundDeliverySink, ProtocolHttpEgress};
use crate::error::ProductAdapterError;
use crate::identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
use crate::inbound::ParsedProductInbound;
use crate::outbound::{ProductOutboundEnvelope, ProductRenderOutcome};

/// Host-materialized workspace file for one native channel delivery.
///
/// This value is deliberately not serializable: bytes stay in the trusted
/// host-to-adapter call and never enter durable product envelopes.
#[derive(Clone, PartialEq, Eq)]
pub struct ProductOutboundAttachment {
    workspace_path: String,
    filename: String,
    mime_type: String,
    bytes: Vec<u8>,
}

impl ProductOutboundAttachment {
    pub fn new(
        workspace_path: impl Into<String>,
        filename: impl Into<String>,
        mime_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Self, ProductAdapterError> {
        let workspace_path = workspace_path.into();
        let filename = filename.into();
        let mime_type = mime_type.into();
        if !workspace_path.starts_with("/workspace/")
            || workspace_path.split('/').any(|segment| segment == "..")
        {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "workspace_attachment_path",
                reason: "path must be confined below /workspace".into(),
            });
        }
        if filename.is_empty()
            || filename.contains(['/', '\\'])
            || filename.chars().any(char::is_control)
        {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "workspace_attachment_filename",
                reason: "filename must be a single non-empty path component".into(),
            });
        }
        if mime_type.is_empty() {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "workspace_attachment_mime_type",
                reason: "MIME type must not be empty".into(),
            });
        }
        Ok(Self {
            workspace_path,
            filename,
            mime_type,
            bytes,
        })
    }

    pub fn workspace_path(&self) -> &str {
        &self.workspace_path
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductAdapterHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

#[async_trait]
pub trait ProductAdapter: Send + Sync {
    fn adapter_id(&self) -> &ProductAdapterId;

    fn installation_id(&self) -> &AdapterInstallationId;

    fn surface_kind(&self) -> ProductSurfaceKind;

    fn capabilities(&self) -> &ProductAdapterCapabilities;

    /// Host-visible protocol-auth policy. The host must enforce this before it
    /// constructs verified auth evidence and calls [`Self::parse_inbound`].
    fn auth_requirement(&self) -> &AuthRequirement;

    /// Host-visible egress allowlist and credential handles declared by this
    /// adapter installation.
    fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &[]
    }

    /// Parse an authenticated protocol payload into adapter-controlled fields.
    /// Trusted fields (adapter id, installation id, auth claim, received_at)
    /// are stamped by the host via [`crate::TrustedInboundContext`]. Ignored
    /// authenticated events must return an explicit `ProductInboundPayload::NoOp`.
    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedProductInbound, ProductAdapterError>;

    /// Render a projection-derived outbound envelope into the external surface.
    /// Implementations use `delivery_sink` to report the exact attempt outcome.
    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError>;

    /// Render with transient host-materialized workspace files. Text-only
    /// behavior remains backward compatible; adapters that do not implement
    /// files fail closed instead of silently dropping them.
    async fn render_outbound_with_attachments(
        &self,
        envelope: ProductOutboundEnvelope,
        attachments: Vec<ProductOutboundAttachment>,
        egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        if attachments.is_empty() {
            return self.render_outbound(envelope, egress, delivery_sink).await;
        }
        Err(ProductAdapterError::Internal {
            detail: crate::redaction::RedactedString::new(
                "product adapter does not support outbound attachments",
            ),
        })
    }

    fn health(&self) -> ProductAdapterHealth {
        ProductAdapterHealth::Healthy
    }
}
