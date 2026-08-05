//! Product-adapter contracts that stay in `ironclaw_host_api`.
//!
//! What is left here after WS1.3/WS1.4/WS1.5 is exactly what neither contracts
//! tier could take: the protocol **auth evidence** family, whose sealed
//! bearer/session constructors §6.1.1 keeps in this crate (WS1.5 moved the
//! verified-inbound half to `ironclaw_extension_contracts::verified_inbound`
//! and deleted the `host-auth-mint` cargo feature, replacing it with the
//! witness-token seal `auth` documents), and the adapter capability flags and
//! the adapter identity newtypes, which §6 assigns to neither tier and which
//! `host_api::user_identity` itself names (`AdapterInstallationId`).
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
// The mint family is deliberately NOT re-exported here. One import path per
// mint function, its owner's (`product_adapter::auth` for bearer/session,
// `ironclaw_extension_contracts::verified_inbound` for channel/webhook) —
// §11.2.4's rule applied to the evidence seam, pinned by
// `reborn_sealed_evidence_mint_ratchet`. A second path is how the family
// reached `ironclaw_extension_host` and `ironclaw_webui` through
// `ironclaw_assistant` before WS1.5.
pub use capabilities::{ProductAdapterCapabilities, ProductCapabilityFlag};
pub use error::{ProductAdapterError, ProductSurfaceRejectionKind};
pub use identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
pub use redaction::{REDACTED_PLACEHOLDER, RedactedDebug, RedactedString};
