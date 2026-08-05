//! External-actor → Reborn user resolution.
//!
//! A channel surface knows only a protocol-shaped actor (`ExternalActorRef`);
//! which Reborn user that actor *is* depends on host-owned identity bindings
//! product does not read. So product asks a resolver wired beside it.
//!
//! The port is declared here and implemented by the extension host (PROPOSAL
//! §6.1.3) — the same shape as [`crate::subject_route`]. It became declarable
//! here once two things stopped blocking it: the error is no longer product's
//! workflow type (see [`crate::error::ProductOperationFailure`], WS2.2), and
//! the binding epoch its response carries is no longer
//! `ironclaw_conversations`' — it moved to
//! `ironclaw_extension_contracts::external`, beside the actor ref whose binding
//! it versions.

use async_trait::async_trait;
use ironclaw_extension_contracts::external::{ExternalActorBindingEpoch, ExternalActorRef};
use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};

use crate::error::ProductOperationFailure;

/// Request passed to host-owned actor-to-user resolvers before the workflow
/// writes a conversation pairing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProductActorUserResolutionRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
}

impl ProductActorUserResolutionRequest {
    pub fn new(
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
    ) -> Self {
        Self {
            adapter_id,
            installation_id,
            external_actor_ref,
        }
    }
}

/// The resolved user, plus the generation of the binding that resolved it.
///
/// The epoch is what makes staleness detectable: a resolver whose binding was
/// re-issued answers with the same `user_id` and a *different* epoch, and the
/// default [`ProductActorUserResolver::resolved_product_actor_user_is_current`]
/// compares the whole value, so a re-pairing invalidates a cached resolution
/// even when the user did not change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProductActorUser {
    pub user_id: UserId,
    pub binding_epoch: Option<ExternalActorBindingEpoch>,
}

impl ResolvedProductActorUser {
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            binding_epoch: None,
        }
    }

    pub fn with_binding_epoch(user_id: UserId, binding_epoch: ExternalActorBindingEpoch) -> Self {
        Self {
            user_id,
            binding_epoch: Some(binding_epoch),
        }
    }
}

/// Resolve the Reborn user an external actor is bound to.
///
/// `Ok(None)` means "this actor is not bound" — a routing decision the caller
/// turns into a pairing prompt, never an error.
#[async_trait]
pub trait ProductActorUserResolver: Send + Sync {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductOperationFailure>;

    /// Whether a previously resolved actor→user binding is still the current
    /// one. Implementations that keep a positive cache MUST bypass it here:
    /// this is the revocation/freshness check, not the hot path.
    async fn resolved_product_actor_user_is_current(
        &self,
        request: &ProductActorUserResolutionRequest,
        expected: &ResolvedProductActorUser,
    ) -> Result<bool, ProductOperationFailure> {
        Ok(self
            .resolve_product_actor_user(request.clone())
            .await?
            .as_ref()
            == Some(expected))
    }
}
