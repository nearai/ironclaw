//! The capability-policy **engine**: the durable delta store, the milestone
//! [`PolicyResolver`], and the host-owned adapters that feed the configuration
//! and approval dimensions of dispatch (#5261 D3 / D5 / D6).
//!
//! This module is deliberately #4544-independent: it depends only on the
//! [`ironclaw_capability_policy`] crate and the durable
//! [`CapabilityPolicyDeltaStore`], NOT on the scoped-lifecycle installation
//! store that backs the per-`(tenant, user)` capability **availability**
//! resolver (which lives in [`crate::capability_surface_policy`]). Keeping the
//! engine and the availability resolver on opposite sides of a file boundary
//! lets the two ship as separate PRs.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_host_api::VirtualPath;
use ironclaw_turns::run_profile::LoopRunContext;

use ironclaw_capability_policy::{
    CapabilityPolicyDeltaStore, PolicyResolver, PolicySubject, StaticCapabilityDefaultPolicySource,
    StoreBackedPolicyResolver,
};
use ironclaw_product_workflow_storage::FilesystemCapabilityPolicyDeltaStore;

/// Durable virtual root for the local-dev capability-policy **delta** store
/// (#5273). Sits under the SAME mounted prefix as the installation store
/// (`/tenants/capability_policy`, the durable libSQL mount) but in a sibling
/// subtree (`/policy_deltas`) so delta leaves never collide with the lifecycle
/// store's `/installations` / `/installation_ids` leaves.
pub(crate) const LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT: &str =
    "/tenants/capability_policy/policy_deltas";

/// Construct the local-dev capability-policy delta store over the durable
/// `/tenants` mount. This is the SINGLE delta-store the runtime builds — the
/// dispatch `PolicyResolver` reads it and the admin REST write surface (#5268 /
/// #5273) writes it. Both share this backing so writes are visible to reads.
pub(crate) fn local_dev_capability_policy_delta_store(
    filesystem: Arc<dyn ironclaw_filesystem::RootFilesystem>,
) -> Arc<dyn CapabilityPolicyDeltaStore> {
    let root = VirtualPath::new(LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT)
        .expect("LOCAL_DEV_CAPABILITY_POLICY_DELTA_ROOT is a valid virtual path");
    Arc::new(FilesystemCapabilityPolicyDeltaStore::with_root(
        filesystem, root,
    ))
}

/// Build the milestone `PolicyResolver` over the shared delta store (#5261 D3).
///
/// The default source uses the milestone default
/// ([`CapabilityDefaultPolicy::available_default`](ironclaw_capability_policy::CapabilityDefaultPolicy::available_default)):
/// installed == available unless an admin delta hides it, with no admin opinion
/// on identity/approval/config. The resolver is the read path for every
/// dimension; it shares the SAME delta-store `Arc` the admin write surface
/// holds — never construct a second.
pub(crate) fn build_capability_policy_resolver(
    delta_store: Arc<dyn CapabilityPolicyDeltaStore>,
) -> Arc<dyn PolicyResolver> {
    let defaults = StaticCapabilityDefaultPolicySource::new(
        ironclaw_capability_policy::CapabilityDefaultPolicy::available_default(),
    );
    Arc::new(StoreBackedPolicyResolver::new(defaults, delta_store))
}

/// Adapts the shared [`PolicyResolver`] into the loop's host-owned
/// [`LoopCapabilityConfigSource`] (#5261 configuration dimension). Supplies the
/// admin policy config (`EffectivePolicy.config`) to deep-merge into a
/// capability's resolved input before dispatch.
///
/// Holds the SAME resolver `Arc` the availability seam reads, so config and
/// availability stay consistent for a turn.
pub(crate) struct PolicyResolverConfigSource {
    policy: Arc<dyn PolicyResolver>,
}

impl PolicyResolverConfigSource {
    pub(crate) fn new(policy: Arc<dyn PolicyResolver>) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl ironclaw_loop_support::LoopCapabilityConfigSource for PolicyResolverConfigSource {
    async fn config_for(
        &self,
        run_context: &LoopRunContext,
        capability_id: &CapabilityId,
    ) -> Result<Option<serde_json::Value>, ironclaw_turns::run_profile::AgentLoopHostError> {
        // Use the SAME acting-principal derivation the availability seam uses.
        let Some(user_id) = crate::capability_surface_policy::principal_user_id(
            &run_context.scope,
            run_context.actor.as_ref(),
        ) else {
            // Ownerless / actor-fallback turn: no subject, so no admin config to
            // overlay. Fail-OPEN — the model input dispatches un-merged.
            return Ok(None);
        };
        let subject = PolicySubject {
            tenant_id: run_context.scope.tenant_id.clone(),
            user_id: user_id.clone(),
        };
        match self.policy.resolve(&subject, capability_id).await {
            Ok(effective) => {
                // `available_default()` config is `Null`; a delta with no
                // `config_patch` leaves it `Null`. Treat `Null` as "no admin
                // opinion" so the merge is skipped entirely.
                if effective.config.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(effective.config))
                }
            }
            Err(error) => {
                // Fail-OPEN (#5261 D5 configuration): a resolver fault or an
                // unavailable backend must NOT end the turn. Drop the admin
                // config and dispatch the un-merged model input. Logged at debug
                // so it never corrupts the REPL/TUI surface (see CLAUDE.md).
                tracing::debug!(
                    %error,
                    capability = %capability_id.as_str(),
                    "capability policy config resolution failed; dispatching un-merged input"
                );
                Ok(None)
            }
        }
    }
}

/// Adapts the shared [`PolicyResolver`] into the dep-light approval module's
/// host-owned [`AdminApprovalSource`] (#5261 D6 approval dimension). Supplies
/// the admin (org-wide) approval opinion (`EffectivePolicy.approval`) so the
/// dispatch approval chain can apply admin Deny/Allow precedence.
///
/// Holds the SAME resolver `Arc` the availability and config seams read, so all
/// three dimensions stay consistent for a turn.
pub(crate) struct PolicyResolverAdminApprovalSource {
    policy: Arc<dyn PolicyResolver>,
}

impl PolicyResolverAdminApprovalSource {
    pub(crate) fn new(policy: Arc<dyn PolicyResolver>) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl crate::profile_approval_authorization::AdminApprovalSource
    for PolicyResolverAdminApprovalSource
{
    async fn admin_approval(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<ironclaw_host_api::PermissionMode> {
        let subject = PolicySubject {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
        };
        match self.policy.resolve(&subject, capability_id).await {
            // Only a definite admin opinion (Allow/Deny) is surfaced; `Ask`
            // carries no org-wide decision, so it maps to `None` ("no admin
            // opinion") exactly like a missing row or a fault. This makes the
            // trait doc literally true and lets the dispatch chain fall through
            // to the existing user/profile steps (require_approval_for_profile_policy
            // matches only `Some(Allow)` / `Some(Deny)`, so `None` for `Ask`
            // preserves the fall-through).
            Ok(effective) => match effective.approval {
                ironclaw_host_api::PermissionMode::Allow
                | ironclaw_host_api::PermissionMode::Deny => Some(effective.approval),
                ironclaw_host_api::PermissionMode::Ask => None,
            },
            Err(error) => {
                // Fail-SAFE (#5261 D5 approval): a resolver fault must NOT
                // auto-approve (privilege escalation). Returning `None` makes
                // the dispatch chain treat this as "no admin opinion" and fall
                // through to the existing user/profile steps. Logged at debug so
                // it never corrupts the REPL/TUI surface (see CLAUDE.md).
                tracing::debug!(
                    %error,
                    capability = %capability_id.as_str(),
                    "capability policy approval resolution failed; deferring to user/profile chain"
                );
                None
            }
        }
    }
}

/// Whether the per-`(tenant, user)` capability policy resolver (#5267 / #5261)
/// is active for this runtime. Compiled in by the `capability-policy` feature,
/// but OFF unless `IRONCLAW_REBORN_CAPABILITY_POLICY` is set truthy
/// (`1` / `true` / `yes` / `on`). Enabling the feature alone therefore never
/// changes local-dev behaviour — the operator opts in. Mirrors the
/// `HooksActivationConfig` master-flag-default-off shape with an env toggle.
///
/// Lives here (shared `pub(crate)`) so both `runtime.rs` (availability seam
/// construction) and `factory.rs` (shared delta-store / resolver handle
/// construction) gate on the same switch.
pub(crate) fn capability_policy_activated() -> bool {
    std::env::var("IRONCLAW_REBORN_CAPABILITY_POLICY")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_capability_policy::{CapabilityPolicyDelta, InMemoryCapabilityPolicyDeltaStore};
    use ironclaw_host_api::TenantId;

    /// `build_capability_policy_resolver` over an empty delta store resolves to
    /// the milestone default (`available_default()`): every capability is
    /// available with no admin opinion on config/approval.
    #[tokio::test]
    async fn resolver_over_empty_store_yields_available_default() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        let resolver = build_capability_policy_resolver(Arc::new(store));

        let subject = PolicySubject {
            tenant_id: TenantId::from_trusted("tenant:acme".to_string()),
            user_id: ironclaw_host_api::UserId::from_trusted("user:bob".to_string()),
        };
        let capability = CapabilityId::new("nearai.web_search").expect("valid capability id");
        let effective = resolver
            .resolve(&subject, &capability)
            .await
            .expect("resolve");

        assert!(
            effective.available,
            "available_default() makes every capability available"
        );
        assert!(
            effective.config.is_null(),
            "available_default() carries no admin config opinion"
        );
    }

    /// A `Hidden` admin delta flips `available` to `false` through the
    /// store-backed resolver the engine builds.
    #[tokio::test]
    async fn resolver_applies_hidden_delta() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        let tenant = TenantId::from_trusted("tenant:acme".to_string());
        let user = ironclaw_host_api::UserId::from_trusted("user:bob".to_string());
        let capability = CapabilityId::new("nearai.web_search").expect("valid capability id");

        store
            .upsert_delta(
                &tenant,
                CapabilityPolicyDelta {
                    scope: ironclaw_capability_policy::PolicyScope::User {
                        user_id: user.clone(),
                    },
                    capability: capability.clone(),
                    availability: Some(ironclaw_capability_policy::Availability::Hidden),
                    identity: None,
                    approval: None,
                    config_patch: None,
                },
            )
            .await
            .expect("seed policy delta");

        let resolver = build_capability_policy_resolver(Arc::new(store));
        let subject = PolicySubject {
            tenant_id: tenant,
            user_id: user,
        };
        let effective = resolver
            .resolve(&subject, &capability)
            .await
            .expect("resolve");

        assert!(
            !effective.available,
            "a User-scope Hidden delta makes the capability unavailable"
        );
    }
}
