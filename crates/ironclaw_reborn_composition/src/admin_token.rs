//! Admin API token minting port.
//!
//! Kept in its own module so it can appear in the `runtime.product_surface`
//! signature. The trait carries no WebUI/ingress types — just the canonical
//! identifiers and a `SecretString` — so it is dependency-free.

use ironclaw_host_api::{TenantId, UserId};
use secrecy::SecretString;

/// Mints a one-time API bearer for a newly created user. Implemented at the
/// serve layer over the session store (a `SignedTokenSessionStore` is stateless
/// and deterministic from the operator secret, so it can be built independently
/// of the ingress auth surface). Abstracted here so composition needs no
/// dependency on the ingress crate.
#[async_trait::async_trait]
pub trait AdminApiTokenMinter: Send + Sync {
    /// Mint a bearer for `(tenant, user_id)`. On failure returns a short reason
    /// (logged, never surfaced to the client).
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String>;
}
