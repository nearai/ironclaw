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

    fn health(&self) -> ProductAdapterHealth {
        ProductAdapterHealth::Healthy
    }
}
