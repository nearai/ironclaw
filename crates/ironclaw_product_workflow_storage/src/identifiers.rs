//! Shared identifier-derivation helpers used by both backends.
//!
//! These are security-relevant: the derived `UserId` is what scopes every
//! downstream Reborn operation (turn submission, secrets visibility, thread
//! ownership). Both the libSQL and Postgres binding services MUST use the
//! same derivation so a switch of DB backend never silently re-maps
//! external actors to different canonical users.

use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{ProductWorkflowError, ResolveBindingRequest};

/// Derive a canonical Reborn `UserId` from external adapter refs.
///
/// Format (stable across restarts and DB backends):
///   `{adapter_id}_{installation_id}_{external_actor_kind}_{external_actor_id}`
///
/// The four input components are all validated upstream by their newtype
/// constructors (`ProductAdapterId`, `AdapterInstallationId`,
/// `ExternalActorRef`), so we know they are non-empty, control-character
/// free, and bounded in length. The concatenation is then validated by
/// `UserId::new` which applies the host-api scope-id rules.
pub(crate) fn derive_user_id(
    request: &ResolveBindingRequest,
) -> Result<UserId, ProductWorkflowError> {
    let raw = format!(
        "{}_{}_{}_{}",
        request.adapter_id.as_str(),
        request.installation_id.as_str(),
        request.external_actor_ref.kind(),
        request.external_actor_ref.id(),
    );
    UserId::new(raw).map_err(|e| ProductWorkflowError::BindingResolutionFailed {
        reason: e.to_string(),
    })
}
