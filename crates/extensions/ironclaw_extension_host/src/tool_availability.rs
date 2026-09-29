//! Tool availability from extension credentials (upstream #7836's rule).
//!
//! A tool is usable when it is registered, its extension is installed and
//! activated, it is authorized for the run's scope, and every credential it
//! marks mandatory is ready. The first three are already true of any tool the
//! loop host asks about: the host runtime's visible surface lists only
//! authorized capabilities from the active registry. This predicate adds the
//! last one: each required product-auth credential must resolve to a
//! configured account for the acting user, through
//! [`missing_runtime_credential_auth_requirements`], the same lookup the
//! activation gate uses.
//!
//! The predicate only decides what is *advertised*. A tool it rules out stays
//! callable, and calling it is what raises the auth gate whose setup prompt
//! routes by the requirement's `requester_extension` (the extension name).
//!
//! Credentials sourced from a raw secret handle (rather than a product-auth
//! account) are not checked here; the dispatch-time credential pre-flight
//! still covers them.

use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::future::join_all;
use ironclaw_auth::product_auth::credentials::runtime_credentials::{
    RuntimeCredentialAccountSelectionService, missing_runtime_credential_auth_requirements,
};
use ironclaw_extension_registry::SharedExtensionRegistry;
use ironclaw_host_api::{
    capability::CapabilityDescriptor,
    decision::RuntimeCredentialAuthRequirement,
    dispatch::CredentialStageError,
    ids::{CapabilityId, UserId},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::{
    LoopRunContext, TOOL_AVAILABILITY_LOOKUP_TIMEOUT, ToolAvailability, ToolAvailabilityPredicate,
    ToolUnavailableReason,
};
use tracing::debug;

const TOOL_AVAILABILITY_LOG_TARGET: &str = "ironclaw::reborn::tool_availability";

/// The composition-supplied [`ToolAvailabilityPredicate`]: capabilities from
/// the active extension registry, credential readiness from product auth.
pub fn extension_tool_availability(
    active_registry: Arc<SharedExtensionRegistry>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    fallback_user_id: UserId,
) -> Arc<dyn ToolAvailabilityPredicate> {
    Arc::new(ExtensionToolAvailability {
        active_registry,
        credential_accounts,
        fallback_user_id,
        lookup_timeout: TOOL_AVAILABILITY_LOOKUP_TIMEOUT,
    })
}

struct ExtensionToolAvailability {
    active_registry: Arc<SharedExtensionRegistry>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    /// The user a run without an actor acts as; the same fallback the
    /// capability host uses, so both resolve the same accounts.
    fallback_user_id: UserId,
    lookup_timeout: Duration,
}

impl fmt::Debug for ExtensionToolAvailability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionToolAvailability")
            .field("lookup_timeout", &self.lookup_timeout)
            .finish_non_exhaustive()
    }
}

/// One credential requirement's readiness for the acting user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Ready,
    Missing,
    /// The lookup failed or timed out.
    Unknown,
}

/// The mandatory product-auth requirements of one capability, attributed to
/// the extension that provides it.
fn mandatory_requirements(
    descriptor: &CapabilityDescriptor,
) -> Vec<RuntimeCredentialAuthRequirement> {
    descriptor
        .runtime_credentials
        .iter()
        .filter(|credential| credential.required)
        .filter_map(|credential| {
            credential.product_auth_requirement_for(descriptor.provider.clone())
        })
        .collect()
}

impl ExtensionToolAvailability {
    async fn readiness(
        &self,
        scope: &ResourceScope,
        requirement: &RuntimeCredentialAuthRequirement,
    ) -> Readiness {
        let lookup = missing_runtime_credential_auth_requirements(
            self.credential_accounts.as_ref(),
            scope,
            vec![requirement.clone()],
        );
        match tokio::time::timeout(self.lookup_timeout, lookup).await {
            Ok(Ok(missing)) if missing.is_empty() => Readiness::Ready,
            Ok(Ok(_)) | Ok(Err(CredentialStageError::AuthRequired)) => Readiness::Missing,
            Ok(Err(CredentialStageError::Backend)) => Readiness::Unknown,
            Err(_) => Readiness::Unknown,
        }
    }
}

#[async_trait]
impl ToolAvailabilityPredicate for ExtensionToolAvailability {
    async fn availability(
        &self,
        run_context: &LoopRunContext,
        capability_ids: &[CapabilityId],
    ) -> BTreeMap<CapabilityId, ToolAvailability> {
        let scope = run_context.acting_resource_scope(&self.fallback_user_id);
        let registry = self.active_registry.snapshot();
        let mut answers = BTreeMap::new();
        let mut pending: Vec<(CapabilityId, Vec<RuntimeCredentialAuthRequirement>)> = Vec::new();
        let mut distinct: Vec<RuntimeCredentialAuthRequirement> = Vec::new();
        for capability_id in capability_ids {
            // A capability outside the extension registry is host-provided:
            // the authorized surface already vouches for it and it declares
            // no account to connect.
            let requirements = registry
                .get_capability(capability_id)
                .map(mandatory_requirements)
                .unwrap_or_default();
            if requirements.is_empty() {
                answers.insert(capability_id.clone(), ToolAvailability::Available);
                continue;
            }
            for requirement in &requirements {
                if !distinct.contains(requirement) {
                    distinct.push(requirement.clone());
                }
            }
            pending.push((capability_id.clone(), requirements));
        }

        // Many tools of one extension share a requirement: look each up once,
        // concurrently, each under its own deadline.
        let readiness: Vec<Readiness> = join_all(
            distinct
                .iter()
                .map(|requirement| self.readiness(&scope, requirement)),
        )
        .await;
        let readiness_of = |requirement: &RuntimeCredentialAuthRequirement| {
            distinct
                .iter()
                .position(|candidate| candidate == requirement)
                .and_then(|index| readiness.get(index).copied())
                .unwrap_or(Readiness::Unknown)
        };
        let mut missing = 0_usize;
        let mut unknown = 0_usize;
        for (capability_id, requirements) in pending {
            let states: Vec<Readiness> = requirements.iter().map(readiness_of).collect();
            let answer = if states.contains(&Readiness::Missing) {
                missing += 1;
                ToolAvailability::Unavailable(ToolUnavailableReason::CredentialMissing)
            } else if states.contains(&Readiness::Unknown) {
                unknown += 1;
                ToolAvailability::Unknown
            } else {
                ToolAvailability::Available
            };
            answers.insert(capability_id, answer);
        }
        debug!(
            target: TOOL_AVAILABILITY_LOG_TARGET,
            tools = capability_ids.len(),
            credential_lookups = distinct.len(),
            credential_missing = missing,
            unknown,
            "checked tool availability for the acting user"
        );
        answers
    }
}

#[cfg(test)]
mod tests;
