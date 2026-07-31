//! Product-adapter contracts that stay in `ironclaw_host_api`.
//!
//! What is left here after WS1.3/WS1.4 is exactly what neither contracts tier
//! could take: the protocol **auth evidence** family, whose sealed
//! bearer/session constructors §6.1.1 keeps in this crate and whose
//! verified-inbound constructors WS1's "consolidate sealed evidence minting"
//! row moves to `ironclaw_extension_contracts` (that row owns the
//! `host-auth-mint` feature deletion too), and the adapter capability flags
//! and the adapter identity newtypes, which §6 assigns to neither tier and
//! which `host_api::user_identity` itself names (`AdapterInstallationId`).
//!
//! Everything else moved and is reached at its new home, never through here:
//! the `ChannelAdapter`/`ToolAdapter` family, channel egress, and the external
//! vendor refs are `ironclaw_extension_contracts`; the inbound/outbound/
//! projection product DTOs and the `ProductSurface` membrane are
//! `ironclaw_product_contracts`. There is deliberately no re-export bridge —
//! a second import path is the defect the §11.2.4 scans exist to prevent.

pub mod auth;
pub mod capabilities;
mod error;
pub mod identity;
pub mod redaction;

pub use crate::product_adapter_error::ProtocolAuthFailure;
pub use crate::product_adapter_error::ProtocolHttpEgressError;
pub use auth::{AuthRequirement, ProtocolAuthEvidence, VerifiedAuthClaim};
#[cfg(feature = "host-auth-mint")]
pub use auth::{
    mark_bearer_token_verified, mark_bearer_token_verified_for_tenant,
    mark_request_signature_verified, mark_request_signature_verified_for_tenant,
    mark_session_verified, mark_session_verified_for_tenant, mark_shared_secret_header_verified,
    mark_shared_secret_header_verified_for_tenant,
};
pub use capabilities::{ProductAdapterCapabilities, ProductCapabilityFlag};
pub use error::{ProductAdapterError, ProductSurfaceRejectionKind};
pub use identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
pub use redaction::{REDACTED_PLACEHOLDER, RedactedDebug, RedactedString};
