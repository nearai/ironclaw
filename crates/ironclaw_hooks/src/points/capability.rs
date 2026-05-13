//! Context for the `before_capability` hook point.

use ironclaw_host_api::TenantId;

/// Read-only context handed to a `before_capability` hook.
///
/// Marked `#[non_exhaustive]` so additional fields can be added (capability
/// arguments digest, run id, iteration, surface version, etc.) without
/// breaking existing hook authors when this crate composes with the rest of
/// the Reborn loop wiring.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BeforeCapabilityHookContext {
    pub tenant_id: TenantId,
    pub capability_name: String,
    /// The dispatcher's *opaque* digest of the capability arguments. Hook
    /// authors can compare this digest across calls (e.g., for repetition
    /// detection) but cannot read the underlying args; raw args never reach
    /// hook scope.
    pub arguments_digest: [u8; 32],
}

impl BeforeCapabilityHookContext {
    pub fn new(tenant_id: TenantId, capability_name: String, arguments_digest: [u8; 32]) -> Self {
        Self {
            tenant_id,
            capability_name,
            arguments_digest,
        }
    }
}
