//! ProductAdapter trait.
//!
//! Concrete adapter implementations (Telegram v2, Web, CLI, Slack v2, ...)
//! implement this trait against the contract defined in this crate. Adapters
//! parse external protocol payloads into a [`crate::ProductInboundEnvelope`],
//! and protocol-translate [`crate::ProductOutboundEnvelope`] back into the
//! external surface using the constrained egress capability.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::ProtocolAuthEvidence;
use crate::capabilities::ProductAdapterCapabilities;
use crate::egress::ProtocolHttpEgress;
use crate::error::ProductAdapterError;
use crate::identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
use crate::inbound::ProductInboundEnvelope;
use crate::outbound::ProductOutboundEnvelope;

/// Health snapshot for ops/observability surfaces.
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

    /// Parse a verified protocol payload into a structured inbound envelope.
    ///
    /// `auth_evidence` is constructed by the host before this call. The
    /// adapter MUST refuse to construct an envelope when the evidence is not
    /// `Verified`.
    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: ProtocolAuthEvidence,
    ) -> Result<Option<ProductInboundEnvelope>, ProductAdapterError>;

    /// Render a projection-derived outbound envelope into the external
    /// surface. Adapters use the supplied [`ProtocolHttpEgress`] for any
    /// network I/O.
    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
    ) -> Result<(), ProductAdapterError>;

    fn health(&self) -> ProductAdapterHealth {
        ProductAdapterHealth::Healthy
    }
}
